#[path = "../src/files.rs"]
mod files;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/sessions.rs"]
mod sessions;
#[path = "../src/store.rs"]
mod store;
use model::CreateSession;
use std::process::Command;

#[test]
#[ignore = "disposable live tmux server and user service"]
fn persistent_idempotent_hostile_directory() {
    let protected = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .unwrap()
        .stdout;
    let fixture = tempfile::tempdir().unwrap();
    let dir = fixture
        .path()
        .join("-- quotes ' spaces 日本語 $(touch NEVER)");
    std::fs::create_dir(&dir).unwrap();
    let old_home = std::env::var_os("HOME");
    let old_shell = std::env::var_os("SHELL");
    std::env::set_var("HOME", fixture.path());
    std::env::set_var("SHELL", "/bin/sh");
    let request = CreateSession {
        key: "fixture-one".into(),
        directory: dir.to_string_lossy().into_owned(),
        provider: "shell".into(),
        name: "disposable fixture".into(),
    };
    let result = std::panic::catch_unwind(|| {
        let first = sessions::create(&request).unwrap();
        let retry = sessions::create(&request).unwrap();
        assert_eq!(first.id, retry.id);
        assert_eq!(first.pid, retry.pid);
        assert_eq!(first.started, retry.started);
        assert_eq!(first.boot_id, retry.boot_id);
        let cgroup = std::fs::read_to_string(format!("/proc/{}/cgroup", first.pid)).unwrap();
        if std::path::Path::new(&format!("/run/user/{}/bus", unsafe { libc::geteuid() })).exists() {
            assert!(
                cgroup.contains("cx-tmux-") || cgroup.contains("tmux-spawn-"),
                "managed tmux must have independent service cgroup: {cgroup}"
            );
        }
        assert_eq!(first.directory, request.directory);
        assert_eq!(retry.name, request.name);
        assert!(sessions::list().unwrap().iter().any(|s| s.id == first.id));
        assert!(!dir.join("NEVER").exists());
        let socket = first.socket.unwrap();
        let prefix = Command::new("tmux")
            .args(["-S", &socket, "show-options", "-g", "prefix"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&prefix.stdout).trim(), "prefix None");
        Command::new("tmux")
            .args(["-S", &socket, "kill-session", "-t", &first.id])
            .status()
            .unwrap();
        assert!(sessions::create(&request)
            .unwrap_err()
            .to_string()
            .contains("ended"));
        println!(
            "PASS id={} pid={} start={} boot={} cwd={:?}",
            first.id, first.pid, first.started, first.boot_id, first.directory
        );
    });
    let sock = fixture.path().join(".local/state/cx/managed.sock");
    let _ = Command::new("tmux")
        .arg("-S")
        .arg(sock)
        .arg("kill-server")
        .status();
    if let Some(v) = old_home {
        std::env::set_var("HOME", v);
    }
    if let Some(v) = old_shell {
        std::env::set_var("SHELL", v);
    }
    let after = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .unwrap()
        .stdout;
    assert_eq!(protected, after, "external tmux sessions must be unchanged");
    if let Err(e) = result {
        std::panic::resume_unwind(e)
    }
}
