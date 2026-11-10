//! Pair codes for the ssh-overlay backend (docs/mesh.md, "ssh-overlay: cx mesh pair").
//!
//! A code is 11 Crockford base32 symbols (0-9 A-Z without I L O U), shown as `XXXX-XXXX-XXX`: 10 random
//! symbols (50 bits) and one check symbol. Input ignores case, dashes and spaces; `I`/`L` read as 1 and `O` as 0.
//! The check makes the weighted sum of all 11 symbols zero in GF(32), so every single mistyped symbol and every
//! swap of two neighbours is caught offline. The code carries no address: the joiner names the viewer's host.
//!
//! The code is never stored. The viewer keeps only [`key`], a SHA-256 of it, which keys the HMAC-SHA256
//! confirmations of the pairing ([`mac`], [`verify`]; RustCrypto `hmac`, constant-time comparison).
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Random symbols in a code (5 bits each).
pub const TOKEN_LEN: usize = 10;
/// Symbols in a code, the check symbol included.
pub const LEN: usize = TOKEN_LEN + 1;

pub fn random<const N: usize>() -> Result<[u8; N]> {
    let mut out = [0u8; N];
    let mut filled = 0;
    while filled < N {
        let n = unsafe { libc::getrandom(out[filled..].as_mut_ptr().cast(), N - filled, 0) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e).context("no system randomness");
        }
        filled += n as usize;
    }
    Ok(out)
}

/// Multiply by x in GF(32) = GF(2)[x] / (x^5 + x^2 + 1), where x is primitive.
fn times_x(r: u8) -> u8 {
    let r = r << 1;
    if r & 32 != 0 {
        r ^ 0b100101
    } else {
        r
    }
}

/// Σ s_i · x^(n-1-i) in GF(32), by Horner's rule. A valid code sums to zero.
fn syndrome(symbols: &[u8]) -> u8 {
    symbols.iter().fold(0, |r, &s| times_x(r) ^ s)
}

fn with_check(token: &[u8]) -> String {
    let check = times_x(syndrome(token));
    token
        .iter()
        .chain([&check])
        .map(|&s| ALPHABET[s as usize] as char)
        .collect()
}

/// A fresh code, normalized (no dashes).
pub fn new_code() -> Result<String> {
    // 256 is a multiple of 32, so the low 5 bits of a random byte are uniform.
    let token = random::<TOKEN_LEN>()?.map(|b| b & 31);
    Ok(with_check(&token))
}

/// `XXXX-XXXX-XXX`: how a code is shown.
pub fn display(code: &str) -> String {
    code.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

/// The normalized code from what was typed, its check verified.
pub fn parse(text: &str) -> Result<String> {
    let mut symbols = Vec::new();
    for c in text.chars() {
        let v = match c.to_ascii_uppercase() {
            '-' | ' ' | '\t' => continue,
            'O' => 0,
            'I' | 'L' => 1,
            c => match ALPHABET.iter().position(|&a| a as char == c) {
                Some(i) => i as u8,
                None => bail!("a pair code has only letters and digits (not {c:?})"),
            },
        };
        symbols.push(v);
        anyhow::ensure!(symbols.len() <= LEN, "a pair code has {LEN} characters");
    }
    anyhow::ensure!(symbols.len() == LEN, "a pair code has {LEN} characters");
    anyhow::ensure!(
        syndrome(&symbols) == 0,
        "pair code check failed; check for a mistyped character"
    );
    Ok(symbols.iter().map(|&v| ALPHABET[v as usize] as char).collect())
}

/// What the viewer stores of a code, and the HMAC key of its pairing: SHA-256("cx-pair-v1|key|" || code).
pub fn key(code: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"cx-pair-v1|key|");
    h.update(code.as_bytes());
    h.finalize().into()
}

fn hmac(key: &[u8; 32], label: &str, fields: &[&[u8]]) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(b"cx-pair-v1|");
    m.update(label.as_bytes());
    m.update(b"|");
    // Length-prefixed, so no two field lists give the same input.
    for f in fields {
        m.update(&(f.len() as u32).to_be_bytes());
        m.update(f);
    }
    m
}

/// HMAC-SHA256(key, "cx-pair-v1|<label>|" || each field with a 4-byte length prefix), as hex.
pub fn mac(key: &[u8; 32], label: &str, fields: &[&[u8]]) -> String {
    hmac(key, label, fields)
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether `tag` (hex) is [`mac`] of these fields; compared in constant time by the `hmac` crate.
pub fn verify(key: &[u8; 32], label: &str, fields: &[&[u8]], tag: &str) -> bool {
    let Some(raw) = hex(tag) else { return false };
    hmac(key, label, fields).verify_slice(&raw).is_ok()
}

fn hex(s: &str) -> Option<Vec<u8>> {
    if s.len() != 64 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// `SHA256:...` of an `ssh-ed25519 AAAA...` key: what `ssh-keygen -l` prints.
pub fn fingerprint(key: &str) -> Result<String> {
    let blob = key
        .split_whitespace()
        .nth(1)
        .context("invalid SSH public key")?;
    let blob = STANDARD.decode(blob).context("invalid SSH public key")?;
    let digest: [u8; 32] = Sha256::digest(&blob).into();
    Ok(format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_short_and_normalize() {
        let c = new_code().unwrap();
        assert_eq!(c.len(), LEN);
        assert!(c.bytes().all(|b| ALPHABET.contains(&b)));
        assert_eq!(parse(&c).unwrap(), c);
        let shown = display(&c);
        assert_eq!(shown.len(), 13, "{shown}");
        assert_eq!(parse(&shown).unwrap(), c);
        let typed = shown
            .to_lowercase()
            .replace('-', " ")
            .replace('0', "o")
            .replace('1', "l");
        assert_eq!(parse(&typed).unwrap(), c);
        assert_ne!(new_code().unwrap(), c, "fresh randomness each time");
        for bad in ["", "ABCD", &format!("{c}0"), &c[..LEN - 1], "U000000000O"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(parse(&format!("{}U", &c[..LEN - 1])).is_err());
    }

    /// Every single substitution and every swap of neighbours (the check symbol included) fails the check.
    #[test]
    fn check_catches_every_typo_and_neighbour_swap() {
        for _ in 0..50 {
            let c = new_code().unwrap().into_bytes();
            for i in 0..LEN {
                for &a in ALPHABET {
                    if a != c[i] {
                        let mut t = c.clone();
                        t[i] = a;
                        assert!(parse(std::str::from_utf8(&t).unwrap()).is_err());
                    }
                }
                if i + 1 < LEN && c[i] != c[i + 1] {
                    let mut t = c.clone();
                    t.swap(i, i + 1);
                    assert!(parse(std::str::from_utf8(&t).unwrap()).is_err());
                }
            }
        }
    }

    #[test]
    fn x_is_primitive_so_weights_never_repeat() {
        let mut r = 1u8;
        for n in 1..=31 {
            r = times_x(r);
            assert_eq!(r == 1, n == 31, "x^{n}");
        }
    }

    #[test]
    fn macs_bind_key_label_and_every_field() {
        let k = key("ABCDEFGHJK0");
        assert_ne!(k, key("ABCDEFGHJK1"));
        let t = mac(&k, "join", &[b"ssh-ed25519 AAAA", b"fp"]);
        assert_eq!(t.len(), 64);
        assert!(verify(&k, "join", &[b"ssh-ed25519 AAAA", b"fp"], &t));
        assert!(!verify(&k, "accept", &[b"ssh-ed25519 AAAA", b"fp"], &t));
        assert!(!verify(&k, "join", &[b"ssh-ed25519 AAAB", b"fp"], &t));
        assert!(!verify(&k, "join", &[b"ssh-ed25519 AAAAf", b"p"], &t));
        assert!(!verify(&key("X"), "join", &[b"ssh-ed25519 AAAA", b"fp"], &t));
        assert!(!verify(&k, "join", &[b"ssh-ed25519 AAAA", b"fp"], &t[..62]));
        assert!(!verify(&k, "join", &[b"ssh-ed25519 AAAA", b"fp"], "zz"));
        // The HMAC-SHA256 the hmac crate computes is the standard one (RFC 4231 test case 2).
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(b"Jefe").unwrap();
        m.update(b"what do ya want for nothing?");
        let hex: String = m
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn fingerprints_match_ssh_keygen() {
        let key =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("k.pub"), format!("{key}\n")).unwrap();
        let out = std::process::Command::new("ssh-keygen")
            .args(["-l", "-E", "sha256", "-f"])
            .arg(dir.path().join("k.pub"))
            .output()
            .unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        assert_eq!(
            text.split_whitespace().nth(1),
            Some(fingerprint(key).unwrap().as_str())
        );
    }
}
