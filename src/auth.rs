//! A private, short-lived OpenSSH askpass bridge. Secrets never enter argv or env.
use anyhow::{anyhow, bail, Context, Result};
use std::{
    cell::RefCell,
    fs::{self, DirBuilder, File},
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        io::AsRawFd,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    process::Command,
    sync::mpsc,
    thread,
    time::Duration,
};

const MAX_FRAME: usize = 8192;
const SOCKET_ENV: &str = "CX_ASKPASS_SOCKET";
const NONCE_ENV: &str = "CX_ASKPASS_NONCE";
const CLIENT_ENV: &str = "CX_SSH_ASKPASS_CLIENT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    HostKey,
    Password,
    KeyPassphrase,
    Verification,
}
pub struct Prompt {
    pub kind: PromptKind,
    pub text: String,
}
pub enum Answer {
    Submit(String),
    Cancel,
}

#[derive(Clone)]
struct Locator {
    socket: PathBuf,
    nonce: String,
}
thread_local! { static BROKER: RefCell<Option<Locator>> = const { RefCell::new(None) }; }

struct PrivateDir(PathBuf);
impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0.join("askpass"));
        let _ = fs::remove_dir(&self.0);
    }
}

// Volatile clearing is best effort; the UI and OpenSSH also hold transient copies.
fn clear(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
}

fn nonce() -> Result<String> {
    let mut bytes = [0u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Apply only to interactive SSH commands constructed inside `with_broker`'s worker.
/// Returns false outside enrollment, preserving existing ordinary SSH behavior.
pub fn configure(command: &mut Command) -> Result<bool> {
    BROKER.with(|slot| {
        let borrowed = slot.borrow();
        let Some(locator) = borrowed.as_ref() else {
            return Ok(false);
        };
        command
            .env("SSH_ASKPASS", std::env::current_exe()?)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", "cx-askpass")
            .env(CLIENT_ENV, "1")
            .env(SOCKET_ENV, &locator.socket)
            .env(NONCE_ENV, &locator.nonce)
            .args(["-o", "ForwardAgent=no", "-o", "StrictHostKeyChecking=ask"]);
        Ok(true)
    })
}

pub fn is_askpass_client() -> bool {
    std::env::var_os(CLIENT_ENV).as_deref() == Some(std::ffi::OsStr::new("1"))
}

fn same_user(stream: &UnixStream) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let status = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut size,
            )
        };
        if status != 0 || cred.uid != unsafe { libc::geteuid() } {
            bail!("askpass peer rejected");
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = stream;
        bail!("private askpass is supported only on Linux");
    }
    Ok(())
}

fn frame_write(stream: &mut UnixStream, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_FRAME {
        bail!("askpass message too large");
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(bytes)?;
    Ok(())
}
fn frame_read(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_FRAME {
        bail!("askpass message too large");
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn limits(stream: &UnixStream, seconds: u64) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(seconds)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(())
}

fn classify(text: String) -> Prompt {
    let lower = text.to_ascii_lowercase();
    let kind = if lower.contains("authenticity of host") || lower.contains("continue connecting") {
        PromptKind::HostKey
    } else if lower.contains("passphrase") {
        PromptKind::KeyPassphrase
    } else if lower.contains("password") {
        PromptKind::Password
    } else {
        PromptKind::Verification
    };
    // Reject terminal controls; retain line breaks needed for host fingerprints.
    let text = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    Prompt { kind, text }
}

/// Called before normal CLI parsing. Do not print errors or diagnostics on stdout.
pub fn askpass_client() -> Result<()> {
    let socket = std::env::var_os(SOCKET_ENV).context("missing askpass locator")?;
    let nonce = std::env::var(NONCE_ENV).context("missing askpass token")?;
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let mut stream = UnixStream::connect(socket).context("askpass connection failed")?;
    same_user(&stream)?;
    limits(&stream, 300)?;
    frame_write(&mut stream, nonce.as_bytes())?;
    frame_write(&mut stream, prompt.as_bytes())?;
    let mut response = frame_read(&mut stream)?;
    let result = if response.first() == Some(&1) {
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(&response[1..])
            .and_then(|_| stdout.write_all(b"\n"))
            .map_err(Into::into)
    } else {
        Err(anyhow!("authentication cancelled"))
    };
    clear(&mut response);
    result
}

pub fn with_broker(
    work: impl FnOnce() -> Result<()> + Send + 'static,
    mut prompt: impl FnMut(Prompt) -> Result<Answer>,
) -> Result<()> {
    let token = nonce()?;
    let directory = std::env::temp_dir().join(format!(
        "cx-askpass-{}-{}",
        std::process::id(),
        &token[..16]
    ));
    DirBuilder::new().mode(0o700).create(&directory)?;
    let directory = PrivateDir(directory);
    let socket = directory.0.join("askpass");
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let locator = Locator {
        socket,
        nonce: token.clone(),
    };
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        BROKER.with(|slot| *slot.borrow_mut() = Some(locator));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            .unwrap_or_else(|_| Err(anyhow!("authentication worker failed")));
        BROKER.with(|slot| *slot.borrow_mut() = None);
        let _ = sender.send(result);
    });
    let mut cancelled = false;
    let mut callback_error = None;
    let result = loop {
        match receiver.try_recv() {
            Ok(result) => break result,
            Err(mpsc::TryRecvError::Disconnected) => {
                break Err(anyhow!("authentication worker disconnected"))
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                // An unauthenticated local client cannot hold the enrollment loop indefinitely.
                let request = (|| -> Result<Prompt> {
                    same_user(&stream)?;
                    limits(&stream, 2)?;
                    if frame_read(&mut stream)? != token.as_bytes() {
                        bail!("askpass token rejected");
                    }
                    Ok(classify(String::from_utf8(frame_read(&mut stream)?)?))
                })();
                if let Ok(request) = request {
                    let host_key = request.kind == PromptKind::HostKey;
                    let answer = if cancelled {
                        Answer::Cancel
                    } else {
                        match prompt(request) {
                            Ok(answer) => answer,
                            Err(error) => {
                                callback_error = Some(error);
                                Answer::Cancel
                            }
                        }
                    };
                    match answer {
                        Answer::Cancel => {
                            cancelled = true;
                            let _ = frame_write(&mut stream, &[0]);
                        }
                        Answer::Submit(mut value) => {
                            let valid = value.len() < MAX_FRAME
                                && !value.contains(['\n', '\r', '\0'])
                                && (!host_key || matches!(value.as_str(), "yes" | "no"));
                            if valid {
                                let mut bytes = Vec::with_capacity(value.len() + 1);
                                bytes.push(1);
                                bytes.extend_from_slice(value.as_bytes());
                                let _ = frame_write(&mut stream, &bytes);
                                clear(&mut bytes);
                            } else {
                                cancelled = true;
                                let _ = frame_write(&mut stream, &[0]);
                            }
                            unsafe {
                                clear(value.as_bytes_mut());
                            }
                        }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20))
            }
            Err(error) => {
                cancelled = true;
                callback_error = Some(error.into());
                thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let _ = worker.join();
    if let Some(error) = callback_error {
        return Err(error);
    }
    if cancelled {
        bail!("authentication cancelled");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn request(text: &str, token_override: Option<&str>) -> Result<Vec<u8>> {
        let locator = BROKER.with(|slot| slot.borrow().clone()).unwrap();
        let metadata = fs::metadata(locator.socket.parent().unwrap())?;
        assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
        assert_eq!(
            fs::metadata(&locator.socket)?.permissions().mode() & 0o777,
            0o600
        );
        let mut stream = UnixStream::connect(locator.socket)?;
        limits(&stream, 5)?;
        frame_write(
            &mut stream,
            token_override.unwrap_or(&locator.nonce).as_bytes(),
        )?;
        frame_write(&mut stream, text.as_bytes())?;
        frame_read(&mut stream)
    }

    #[test]
    fn private_broker_delivers_secret_and_cleans_up() -> Result<()> {
        let path = Arc::new(std::sync::Mutex::new(None));
        let worker_path = path.clone();
        with_broker(
            move || {
                let mut command = Command::new("ssh");
                assert!(configure(&mut command)?);
                let env: Vec<_> = command.get_envs().collect();
                assert!(!env
                    .iter()
                    .any(|(_, value)| value.is_some_and(|v| v == "synthetic-secret")));
                *worker_path.lock().unwrap() =
                    BROKER.with(|slot| slot.borrow().as_ref().map(|l| l.socket.clone()));
                assert_eq!(request("fixture password:", None)?, b"\x01synthetic-secret");
                Ok(())
            },
            |prompt| {
                assert_eq!(prompt.kind, PromptKind::Password);
                Ok(Answer::Submit("synthetic-secret".into()))
            },
        )?;
        assert!(!path.lock().unwrap().as_ref().unwrap().exists());
        assert!(!configure(&mut Command::new("ssh"))?);
        Ok(())
    }

    #[test]
    fn cancellation_denies_later_requests() {
        let count = Arc::new(AtomicUsize::new(0));
        let callbacks = count.clone();
        let result = with_broker(
            || {
                assert_eq!(request("password:", None)?, b"\x00");
                assert_eq!(request("password:", None)?, b"\x00");
                Ok(())
            },
            move |_| {
                callbacks.fetch_add(1, Ordering::SeqCst);
                Ok(Answer::Cancel)
            },
        );
        assert!(result.is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unauthorized_client_never_prompts() -> Result<()> {
        with_broker(
            || {
                assert!(request("password:", Some("wrong-token")).is_err());
                Ok(())
            },
            |_| panic!("unauthorized prompt"),
        )
    }

    #[test]
    fn oversized_frame_is_rejected_before_allocation_or_prompt() -> Result<()> {
        with_broker(
            || {
                let locator = BROKER.with(|slot| slot.borrow().clone()).unwrap();
                let mut stream = UnixStream::connect(locator.socket)?;
                limits(&stream, 5)?;
                frame_write(&mut stream, locator.nonce.as_bytes())?;
                stream.write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())?;
                assert!(frame_read(&mut stream).is_err());
                Ok(())
            },
            |_| panic!("oversized prompt reached UI"),
        )
    }

    #[test]
    fn callback_failure_denies_request_and_returns_error() {
        let result = with_broker(
            || {
                assert_eq!(request("password:", None)?, b"\x00");
                Ok(())
            },
            |_| bail!("fixture UI failure"),
        );
        assert_eq!(result.unwrap_err().to_string(), "fixture UI failure");
    }

    #[test]
    fn rejects_multiline_answer_and_implicit_host_approval() {
        for answer in ["yes\n", "synthetic-secret"] {
            assert!(with_broker(
                || {
                    assert_eq!(
                        request(
                            "Are you sure you want to continue connecting (yes/no)?",
                            None
                        )?,
                        b"\x00"
                    );
                    Ok(())
                },
                |_| Ok(Answer::Submit(answer.into()))
            )
            .is_err());
        }
    }

    #[test]
    fn prompt_types_and_terminal_controls() {
        assert_eq!(
            classify("Enter passphrase for key:\u{1b}[2J".into()).kind,
            PromptKind::KeyPassphrase
        );
        assert_eq!(
            classify("Verification code:".into()).kind,
            PromptKind::Verification
        );
        assert!(!classify("x\u{1b}[2J".into()).text.contains('\u{1b}'));
    }
}
