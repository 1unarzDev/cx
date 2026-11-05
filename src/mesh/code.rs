//! Join codes for the ssh-overlay backend (docs/mesh.md, "ssh-overlay: cx join").
//!
//! Two forms share Crockford base32 (0-9 A-Z without I L O U; input is case-insensitive, `I`/`L`
//! read as 1 and `O` as 0, dashes and spaces are ignored):
//! - A **direct code** carries everything the joiner needs: the viewer's SSH account and reachable
//!   addresses, a one-time token, a truncated SHA-256 of the viewer's SSH host key, and the expiry,
//!   followed by a CRC-16 that catches every single mistyped symbol. It is about 60 symbols, meant to be pasted.
//! - A **short code** is 12 symbols (50 random bits and a CRC-10), redeemable only through
//!   a mesh Worker the owner runs: the Worker holds the direct code sealed under a key derived from
//!   the short code, identified by a separate hash of it, and hands it out once.
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256, Sha512};
use std::net::{IpAddr, SocketAddr};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
pub const VERSION: u8 = 1;
pub const TOKEN_LEN: usize = 8;
pub const FINGERPRINT_LEN: usize = 10;
pub const SHORT_LEN: usize = 12;
const MAX_ADDRESSES: usize = 4;

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

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &b in bytes {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Symbols (0..32) of a typed code, normalized; rejects anything outside the alphabet.
fn symbols(text: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for c in text.chars() {
        let c = c.to_ascii_uppercase();
        let v = match c {
            '-' | ' ' | '\t' => continue,
            'O' => 0,
            'I' | 'L' => 1,
            _ => match ALPHABET.iter().position(|&a| a as char == c) {
                Some(i) => i as u8,
                None => bail!("a join code has only letters and digits (not {c:?})"),
            },
        };
        out.push(v);
    }
    anyhow::ensure!(out.len() <= 256, "join code too long");
    Ok(out)
}

fn decode_bytes(sym: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &s in sym {
        acc = (acc << 5) | s as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

pub fn group(code: &str, size: usize) -> String {
    code.as_bytes()
        .chunks(size)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

/// A CRC over a bit stream (MSB first). Appended after the data in the same bit order, a
/// CRC of width w detects every error burst of at most w bits, so any single mistyped symbol
/// (5 bits) is always caught: CRC-16/CCITT for direct codes, CRC-10/ATM for short codes.
fn crc(bits: impl Iterator<Item = bool>, poly: u16, width: u32, init: u16) -> u16 {
    let top = 1u16 << (width - 1);
    let mask = if width == 16 {
        u16::MAX
    } else {
        (1 << width) - 1
    };
    let mut r = init;
    for bit in bits {
        let feedback = ((r & top) != 0) ^ bit;
        r = (r << 1) & mask;
        if feedback {
            r ^= poly;
        }
    }
    r & mask
}

fn crc16(bytes: &[u8]) -> u16 {
    crc(
        bytes
            .iter()
            .flat_map(|b| (0..8).rev().map(move |i| (b >> i) & 1 == 1)),
        0x1021,
        16,
        0xffff,
    )
}

fn sha(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

fn hmac(key: &[u8; 32], parts: &[&[u8]]) -> [u8; 32] {
    let mut k = [0u8; 64];
    k[..32].copy_from_slice(key);
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    for p in parts {
        inner.update(p);
    }
    let inner: [u8; 32] = inner.finalize().into();
    sha(&[&opad, &inner])
}

fn eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// SHA-256 of an `ssh-ed25519 AAAA...` key's blob: what `ssh-keygen -l -E sha256` prints.
pub fn host_key_digest(key: &str) -> Result<[u8; 32]> {
    let blob = key
        .split_whitespace()
        .nth(1)
        .context("invalid SSH public key")?;
    let blob = STANDARD.decode(blob).context("invalid SSH public key")?;
    Ok(sha(&[&blob]))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Direct {
    pub token: [u8; TOKEN_LEN],
    pub fingerprint: [u8; FINGERPRINT_LEN],
    /// Unix seconds, minute precision.
    pub expires: u64,
    pub account: String,
    pub addresses: Vec<SocketAddr>,
}

fn valid_account(a: &str) -> bool {
    !a.is_empty()
        && a.len() <= 32
        && !a.starts_with('-')
        && a.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
}

impl Direct {
    pub fn new(
        account: &str,
        addresses: Vec<SocketAddr>,
        host_key: &str,
        ttl_secs: u64,
        now: u64,
    ) -> Result<Self> {
        anyhow::ensure!(
            valid_account(account),
            "invalid SSH account for a join code"
        );
        anyhow::ensure!(
            !addresses.is_empty() && addresses.len() <= MAX_ADDRESSES,
            "a join code carries 1 to {MAX_ADDRESSES} addresses"
        );
        let mut fingerprint = [0u8; FINGERPRINT_LEN];
        fingerprint.copy_from_slice(&host_key_digest(host_key)?[..FINGERPRINT_LEN]);
        Ok(Self {
            token: random()?,
            fingerprint,
            expires: (now + ttl_secs).div_ceil(60) * 60,
            account: account.into(),
            addresses,
        })
    }

    fn payload(&self) -> Vec<u8> {
        let mut p = vec![VERSION];
        p.extend_from_slice(&self.token);
        p.extend_from_slice(&self.fingerprint);
        p.extend_from_slice(&((self.expires / 60) as u32).to_be_bytes());
        p.push(self.account.len() as u8);
        p.extend_from_slice(self.account.as_bytes());
        p.push(self.addresses.len() as u8);
        for a in &self.addresses {
            let port = a.port() != 22;
            match a.ip() {
                IpAddr::V4(ip) => {
                    p.push(4 | if port { 0x80 } else { 0 });
                    p.extend_from_slice(&ip.octets());
                }
                IpAddr::V6(ip) => {
                    p.push(6 | if port { 0x80 } else { 0 });
                    p.extend_from_slice(&ip.octets());
                }
            }
            if port {
                p.extend_from_slice(&a.port().to_be_bytes());
            }
        }
        let check = crc16(&p);
        p.extend_from_slice(&check.to_be_bytes());
        p
    }

    /// The code as typed or pasted: grouped by 5.
    pub fn encode(&self) -> String {
        group(&encode(&self.payload()), 5)
    }

    pub fn decode(text: &str) -> Result<Self> {
        let sym = symbols(text)?;
        anyhow::ensure!(
            sym.len() > SHORT_LEN,
            "this is a short code; redeem it through its mesh URL"
        );
        let bytes = decode_bytes(&sym);
        anyhow::ensure!(
            bytes.len() >= 1 + TOKEN_LEN + FINGERPRINT_LEN + 4 + 2 + 2,
            "join code too short"
        );
        let (body, check) = bytes.split_at(bytes.len() - 2);
        // Re-encoding must give the same symbols: this rejects a missing or extra symbol and any
        // non-zero pad bits in the last one (which would otherwise slip past the checksum).
        let canonical: Vec<u8> = encode(&bytes)
            .bytes()
            .map(|c| ALPHABET.iter().position(|&a| a == c).unwrap_or(0) as u8)
            .collect();
        anyhow::ensure!(
            canonical == sym,
            "join code has the wrong length; check for a missing, extra or mistyped character"
        );
        anyhow::ensure!(
            eq(check, &crc16(body).to_be_bytes()),
            "join code checksum does not match; check for a mistyped character"
        );
        anyhow::ensure!(
            body[0] == VERSION,
            "join code version {} is not supported; update cx",
            body[0]
        );
        let mut r = Reader { b: body, i: 1 };
        let mut token = [0u8; TOKEN_LEN];
        token.copy_from_slice(r.take(TOKEN_LEN)?);
        let mut fingerprint = [0u8; FINGERPRINT_LEN];
        fingerprint.copy_from_slice(r.take(FINGERPRINT_LEN)?);
        let minutes = u32::from_be_bytes(r.take(4)?.try_into()?);
        let len = r.take(1)?[0] as usize;
        let account = std::str::from_utf8(r.take(len)?)?.to_owned();
        anyhow::ensure!(
            valid_account(&account),
            "join code carries an invalid account"
        );
        let count = r.take(1)?[0] as usize;
        anyhow::ensure!(
            (1..=MAX_ADDRESSES).contains(&count),
            "join code carries no usable address"
        );
        let mut addresses = Vec::new();
        for _ in 0..count {
            let tag = r.take(1)?[0];
            let ip: IpAddr = match tag & 0x7f {
                4 => <[u8; 4]>::try_from(r.take(4)?)?.into(),
                6 => <[u8; 16]>::try_from(r.take(16)?)?.into(),
                _ => bail!("join code carries an unknown address kind"),
            };
            let port = if tag & 0x80 != 0 {
                u16::from_be_bytes(r.take(2)?.try_into()?)
            } else {
                22
            };
            anyhow::ensure!(
                port != 0 && !ip.is_unspecified() && !ip.is_multicast(),
                "join code carries an invalid address"
            );
            addresses.push(SocketAddr::new(ip, port));
        }
        anyhow::ensure!(r.i == body.len(), "join code has trailing data");
        Ok(Self {
            token,
            fingerprint,
            expires: minutes as u64 * 60,
            account,
            addresses,
        })
    }

    pub fn expired(&self, now: u64) -> bool {
        now >= self.expires
    }

    pub fn matches_host_key(&self, key: &str) -> bool {
        host_key_digest(key).is_ok_and(|d| eq(&d[..FINGERPRINT_LEN], &self.fingerprint))
    }

    /// The invite's Ed25519 seed; the viewer authorizes its public key, the joiner signs with it.
    pub fn invite_seed(&self) -> [u8; 32] {
        let mut h = Sha512::new();
        h.update(b"cx-join-v1 invite key");
        h.update(self.token);
        let d = h.finalize();
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&d[..32]);
        seed
    }

    /// A stable id of the invite that never reveals the token.
    pub fn invite_id(&self) -> String {
        sha(&[b"cx-join-id-v1", &self.token])[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        anyhow::ensure!(self.i + n <= self.b.len(), "join code is truncated");
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }
}

/// A 12-symbol short code: 10 random symbols and their CRC-10.
pub fn short_code() -> Result<String> {
    let r: [u8; 10] = random()?;
    let body: String = r
        .iter()
        .map(|b| ALPHABET[(b & 31) as usize] as char)
        .collect();
    Ok(with_short_check(&body))
}

fn with_short_check(body: &str) -> String {
    let bits = body.bytes().flat_map(|c| {
        let v = ALPHABET.iter().position(|&a| a == c).unwrap_or(0) as u16;
        (0..5).rev().map(move |i| (v >> i) & 1 == 1)
    });
    let v = crc(bits, 0x233, 10, 0);
    format!(
        "{body}{}{}",
        ALPHABET[(v >> 5) as usize] as char,
        ALPHABET[(v & 31) as usize] as char
    )
}

/// The normalized short code (12 symbols, checksum verified).
pub fn parse_short(text: &str) -> Result<String> {
    let sym = symbols(text)?;
    anyhow::ensure!(
        sym.len() == SHORT_LEN,
        "a short join code has {SHORT_LEN} characters"
    );
    let s: String = sym.iter().map(|&v| ALPHABET[v as usize] as char).collect();
    anyhow::ensure!(
        with_short_check(&s[..SHORT_LEN - 2]) == s,
        "join code checksum does not match; check for a mistyped character"
    );
    Ok(s)
}

/// What the Worker indexes a short code by (it never sees the code itself).
pub fn short_lookup(short: &str) -> String {
    sha(&[b"cx-pair-lookup-v1", short.as_bytes()])
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn seal_keys(short: &str) -> ([u8; 32], [u8; 32]) {
    (
        sha(&[b"cx-pair-seal-v1", short.as_bytes()]),
        sha(&[b"cx-pair-mac-v1", short.as_bytes()]),
    )
}

fn keystream(key: &[u8; 32], nonce: &[u8], data: &mut [u8]) {
    for (i, chunk) in data.chunks_mut(32).enumerate() {
        let block = sha(&[key, nonce, &(i as u32).to_be_bytes()]);
        for (d, k) in chunk.iter_mut().zip(block) {
            *d ^= k;
        }
    }
}

/// Encrypt-then-MAC (SHA-256 counter keystream, HMAC-SHA256 tag) under keys from the short code.
pub fn seal(short: &str, plain: &[u8]) -> Result<String> {
    let (enc, mac) = seal_keys(short);
    let nonce: [u8; 16] = random()?;
    let mut ct = plain.to_vec();
    keystream(&enc, &nonce, &mut ct);
    let tag = hmac(&mac, &[&nonce, &ct]);
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    out.extend_from_slice(&tag[..16]);
    Ok(STANDARD.encode(out))
}

pub fn unseal(short: &str, sealed: &str) -> Result<Vec<u8>> {
    let raw = STANDARD
        .decode(sealed.trim())
        .context("sealed invite is not base64")?;
    anyhow::ensure!(
        raw.len() >= 32 && raw.len() <= 4096,
        "sealed invite has the wrong size"
    );
    let (enc, mac) = seal_keys(short);
    let (nonce, rest) = raw.split_at(16);
    let (ct, tag) = rest.split_at(rest.len() - 16);
    anyhow::ensure!(
        eq(tag, &hmac(&mac, &[nonce, ct])[..16]),
        "sealed invite failed authentication (wrong code, or altered in transit)"
    );
    let mut plain = ct.to_vec();
    keystream(&enc, nonce, &mut plain);
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

    fn sample() -> Direct {
        Direct::new(
            "lunar_z",
            vec![
                "192.168.1.20:22".parse().unwrap(),
                "[fd7a:115c:a1e0::5]:2222".parse().unwrap(),
            ],
            KEY,
            3600,
            1_800_000_000,
        )
        .unwrap()
    }

    #[test]
    fn base32_round_trip_and_alphabet() {
        for len in 0..40 {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let text = encode(&bytes);
            assert!(text.bytes().all(|b| ALPHABET.contains(&b)));
            assert!(!text.contains(['I', 'L', 'O', 'U']));
            assert_eq!(decode_bytes(&symbols(&text).unwrap()), bytes);
        }
        assert_eq!(symbols("o-Il").unwrap(), vec![0, 1, 1]);
        assert!(symbols("U").is_err());
    }

    #[test]
    fn direct_code_round_trips_with_normalization() {
        let d = sample();
        let code = d.encode();
        assert!(code.replace('-', "").len() < 100, "{code} is pasteable");
        let one = Direct::new(
            "lunarz",
            vec!["192.168.1.20:22".parse().unwrap()],
            KEY,
            3600,
            0,
        )
        .unwrap();
        assert!(
            one.encode().replace('-', "").len() <= 64,
            "one IPv4 address: {}",
            one.encode()
        );
        assert_eq!(Direct::decode(&code).unwrap(), d);
        let typed = code
            .to_lowercase()
            .replace('-', " ")
            .replace('0', "o")
            .replace('1', "l");
        assert_eq!(Direct::decode(&typed).unwrap(), d);
        assert_eq!(d.expires % 60, 0);
        assert!(d.expires >= 1_800_003_600);
        assert!(d.matches_host_key(KEY));
        assert!(!d.matches_host_key(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"
        ));
        assert_eq!(d.invite_id().len(), 16);
        assert_ne!(d.invite_seed(), sample().invite_seed(), "tokens are random");
    }

    #[test]
    fn checksum_catches_every_single_symbol_typo() {
        for _ in 0..10 {
            single_typos_fail(&sample().encode().replace('-', ""));
        }
    }

    fn single_typos_fail(code: &str) {
        let code = code.to_owned();
        let mut caught = 0;
        let mut total = 0;
        for i in 0..code.len() {
            for &c in ALPHABET {
                let mut b = code.clone().into_bytes();
                if b[i] == c {
                    continue;
                }
                b[i] = c;
                total += 1;
                if Direct::decode(std::str::from_utf8(&b).unwrap()).is_err() {
                    caught += 1;
                }
            }
        }
        // CRC-16 detects every burst of up to 16 bits; a single symbol is a 5-bit burst.
        assert_eq!(caught, total);
        let mut short = code.clone();
        short.pop();
        assert!(Direct::decode(&short).is_err());
        assert!(Direct::decode(&format!("{code}0")).is_err());
    }

    #[test]
    fn crc16_ccitt_false_vector() {
        assert_eq!(crc16(b"123456789"), 0x29b1);
    }

    #[test]
    fn short_code_check_catches_every_single_symbol_typo() {
        for _ in 0..20 {
            let s = short_code().unwrap();
            for i in 0..SHORT_LEN {
                for &c in ALPHABET {
                    let mut b = s.clone().into_bytes();
                    if b[i] == c {
                        continue;
                    }
                    b[i] = c;
                    assert!(parse_short(std::str::from_utf8(&b).unwrap()).is_err());
                }
            }
        }
    }

    #[test]
    fn expiry_is_checked() {
        let d = sample();
        assert!(!d.expired(1_800_000_000));
        assert!(!d.expired(d.expires - 1));
        assert!(d.expired(d.expires));
    }

    #[test]
    fn rejects_bad_fields() {
        assert!(Direct::new("-x", vec!["1.2.3.4:22".parse().unwrap()], KEY, 60, 0).is_err());
        assert!(Direct::new("a", vec![], KEY, 60, 0).is_err());
        assert!(Direct::new("a", vec!["1.2.3.4:22".parse().unwrap(); 5], KEY, 60, 0).is_err());
        assert!(Direct::decode("ABCD").is_err());
    }

    #[test]
    fn short_codes_check_and_seal() {
        let s = short_code().unwrap();
        assert_eq!(s.len(), SHORT_LEN);
        assert_eq!(parse_short(&group(&s, 4).to_lowercase()).unwrap(), s);
        let mut typo = s.clone().into_bytes();
        typo[3] = if typo[3] == b'A' { b'B' } else { b'A' };
        assert!(parse_short(std::str::from_utf8(&typo).unwrap()).is_err());
        assert!(Direct::decode(&s)
            .unwrap_err()
            .to_string()
            .contains("short code"));
        let lookup = short_lookup(&s);
        assert_eq!(lookup.len(), 64);
        assert!(!lookup.contains(&s.to_lowercase()));
        let code = sample().encode();
        let sealed = seal(&s, code.as_bytes()).unwrap();
        assert!(!sealed.contains(&code));
        assert_eq!(unseal(&s, &sealed).unwrap(), code.as_bytes());
        let other = short_code().unwrap();
        assert!(unseal(&other, &sealed).is_err());
        let mut raw = STANDARD.decode(&sealed).unwrap();
        raw[20] ^= 1;
        assert!(
            unseal(&s, &STANDARD.encode(raw)).is_err(),
            "tampering fails the MAC"
        );
    }

    #[test]
    fn hmac_matches_python_hmac() {
        // python3 -c 'import hmac;print(hmac.new(b"\x0b"*32,b"Hi There","sha256").hexdigest())'
        let tag = hmac(&[0x0b; 32], &[b"Hi There"]);
        let hex: String = tag.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, HMAC_0B32_HI_THERE);
    }
    const HMAC_0B32_HI_THERE: &str =
        "198a607eb44bfbc69903a0f1cf2bbdc5ba0aa3f3d9ae3c1c7a3b1696a0b68cf7";
}
