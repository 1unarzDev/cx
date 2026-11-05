//! Just enough Ed25519 to turn a 32-byte seed into its public key (RFC 8032 5.1.5), and to write
//! that pair as an unencrypted OpenSSH private key file. The join code's one-time token derives the
//! seed, so both sides know the invite key without sending it: the viewer authorizes the public key
//! (restricted, forced command, expiring) and the joiner signs with the seed. Signing itself is
//! OpenSSH's; nothing here signs or verifies. Arithmetic is constant-time in the scalar (select by
//! mask, fixed iteration count). Tested against the RFC 8032 vectors and `ssh-keygen -Y`.
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha512};

type Fe = [u64; 5];
const MASK: u64 = (1 << 51) - 1;

fn fe_from_bytes(b: &[u8; 32]) -> Fe {
    let load = |i: usize| -> u64 {
        let mut v = [0u8; 8];
        let end = (i + 8).min(32);
        v[..end - i].copy_from_slice(&b[i..end]);
        u64::from_le_bytes(v)
    };
    [
        load(0) & MASK,
        (load(6) >> 3) & MASK,
        (load(12) >> 6) & MASK,
        (load(19) >> 1) & MASK,
        (load(24) >> 12) & MASK,
    ]
}

fn carry(mut h: Fe) -> Fe {
    for _ in 0..2 {
        let c0 = h[0] >> 51;
        h[0] &= MASK;
        h[1] += c0;
        let c1 = h[1] >> 51;
        h[1] &= MASK;
        h[2] += c1;
        let c2 = h[2] >> 51;
        h[2] &= MASK;
        h[3] += c2;
        let c3 = h[3] >> 51;
        h[3] &= MASK;
        h[4] += c3;
        let c4 = h[4] >> 51;
        h[4] &= MASK;
        h[0] += c4 * 19;
    }
    h
}

fn fe_to_bytes(h: &Fe) -> [u8; 32] {
    let mut h = carry(*h);
    // h < 2p here; q = 1 exactly when h >= p. Add 19q and drop bit 255 (constant time).
    let mut q = (h[0] + 19) >> 51;
    for limb in &h[1..] {
        q = (limb + q) >> 51;
    }
    h[0] += 19 * q;
    for i in 0..4 {
        h[i + 1] += h[i] >> 51;
        h[i] &= MASK;
    }
    h[4] &= MASK;
    let mut out = [0u8; 32];
    let mut acc: u128 = 0;
    let mut bits = 0;
    let mut k = 0;
    for limb in h {
        acc |= (limb as u128) << bits;
        bits += 51;
        while bits >= 8 && k < 32 {
            out[k] = acc as u8;
            acc >>= 8;
            bits -= 8;
            k += 1;
        }
    }
    if k < 32 {
        out[k] = acc as u8;
    }
    out
}

fn add(a: &Fe, b: &Fe) -> Fe {
    carry([
        a[0] + b[0],
        a[1] + b[1],
        a[2] + b[2],
        a[3] + b[3],
        a[4] + b[4],
    ])
}

fn sub(a: &Fe, b: &Fe) -> Fe {
    // a + 4p - b keeps every limb positive for carried inputs (limbs < 2^52).
    const P4: Fe = [
        0x1F_FFFF_FFFF_FFB4,
        0x1F_FFFF_FFFF_FFFC,
        0x1F_FFFF_FFFF_FFFC,
        0x1F_FFFF_FFFF_FFFC,
        0x1F_FFFF_FFFF_FFFC,
    ];
    carry([
        a[0] + P4[0] - b[0],
        a[1] + P4[1] - b[1],
        a[2] + P4[2] - b[2],
        a[3] + P4[3] - b[3],
        a[4] + P4[4] - b[4],
    ])
}

fn mul(a: &Fe, b: &Fe) -> Fe {
    let m = |x: u64, y: u64| (x as u128) * (y as u128);
    let b19 = [b[1] * 19, b[2] * 19, b[3] * 19, b[4] * 19];
    let r0 = m(a[0], b[0]) + m(a[1], b19[3]) + m(a[2], b19[2]) + m(a[3], b19[1]) + m(a[4], b19[0]);
    let r1 = m(a[0], b[1]) + m(a[1], b[0]) + m(a[2], b19[3]) + m(a[3], b19[2]) + m(a[4], b19[1]);
    let r2 = m(a[0], b[2]) + m(a[1], b[1]) + m(a[2], b[0]) + m(a[3], b19[3]) + m(a[4], b19[2]);
    let r3 = m(a[0], b[3]) + m(a[1], b[2]) + m(a[2], b[1]) + m(a[3], b[0]) + m(a[4], b19[3]);
    let r4 = m(a[0], b[4]) + m(a[1], b[3]) + m(a[2], b[2]) + m(a[3], b[1]) + m(a[4], b[0]);
    let mask = MASK as u128;
    let c = r0 >> 51;
    let h0 = (r0 & mask) as u64;
    let r1 = r1 + c;
    let c = r1 >> 51;
    let h1 = (r1 & mask) as u64;
    let r2 = r2 + c;
    let c = r2 >> 51;
    let h2 = (r2 & mask) as u64;
    let r3 = r3 + c;
    let c = r3 >> 51;
    let h3 = (r3 & mask) as u64;
    let r4 = r4 + c;
    let c = r4 >> 51;
    let h4 = (r4 & mask) as u64;
    let h0 = h0 as u128 + c * 19;
    carry([(h0 & mask) as u64, h1 + (h0 >> 51) as u64, h2, h3, h4])
}

fn invert(z: &Fe) -> Fe {
    // z^(p-2), p-2 = 2^255 - 21: square-and-multiply over the fixed exponent bits.
    let mut result: Fe = [1, 0, 0, 0, 0];
    let mut exp = [0xffu8; 32];
    exp[0] = 0xeb; // 2^255 - 21 little-endian: eb ff .. ff 7f
    exp[31] = 0x7f;
    for i in (0..255).rev() {
        result = mul(&result, &result);
        if (exp[i / 8] >> (i % 8)) & 1 == 1 {
            result = mul(&result, z);
        }
    }
    result
}

#[derive(Clone, Copy)]
struct Point {
    x: Fe,
    y: Fe,
    z: Fe,
    t: Fe,
}

fn d2() -> Fe {
    // d = -121665/121666 mod p
    const D: [u8; 32] = [
        0xa3, 0x78, 0x59, 0x13, 0xca, 0x4d, 0xeb, 0x75, 0xab, 0xd8, 0x41, 0x41, 0x4d, 0x0a, 0x70,
        0x00, 0x98, 0xe8, 0x79, 0x77, 0x79, 0x40, 0xc7, 0x8c, 0x73, 0xfe, 0x6f, 0x2b, 0xee, 0x6c,
        0x03, 0x52,
    ];
    let d = fe_from_bytes(&D);
    add(&d, &d)
}

fn point_add(p: &Point, q: &Point, d2: &Fe) -> Point {
    // RFC 8032 5.1.4, a = -1, complete (also doubles).
    let a = mul(&sub(&p.y, &p.x), &sub(&q.y, &q.x));
    let b = mul(&add(&p.y, &p.x), &add(&q.y, &q.x));
    let c = mul(&mul(&p.t, d2), &q.t);
    let zz = mul(&p.z, &q.z);
    let d = add(&zz, &zz);
    let e = sub(&b, &a);
    let f = sub(&d, &c);
    let g = add(&d, &c);
    let h = add(&b, &a);
    Point {
        x: mul(&e, &f),
        y: mul(&g, &h),
        t: mul(&e, &h),
        z: mul(&f, &g),
    }
}

fn select(a: &Point, b: &Point, bit: u64) -> Point {
    let m = 0u64.wrapping_sub(bit & 1);
    let s = |x: &Fe, y: &Fe| -> Fe {
        let mut r = [0; 5];
        for i in 0..5 {
            r[i] = (x[i] & !m) | (y[i] & m);
        }
        r
    };
    Point {
        x: s(&a.x, &b.x),
        y: s(&a.y, &b.y),
        z: s(&a.z, &b.z),
        t: s(&a.t, &b.t),
    }
}

fn base() -> Point {
    const BX: [u8; 32] = [
        0x1a, 0xd5, 0x25, 0x8f, 0x60, 0x2d, 0x56, 0xc9, 0xb2, 0xa7, 0x25, 0x95, 0x60, 0xc7, 0x2c,
        0x69, 0x5c, 0xdc, 0xd6, 0xfd, 0x31, 0xe2, 0xa4, 0xc0, 0xfe, 0x53, 0x6e, 0xcd, 0xd3, 0x36,
        0x69, 0x21,
    ];
    let mut by = [0x66u8; 32];
    by[0] = 0x58;
    let x = fe_from_bytes(&BX);
    let y = fe_from_bytes(&by);
    Point {
        x,
        y,
        z: [1, 0, 0, 0, 0],
        t: mul(&x, &y),
    }
}

/// The Ed25519 public key of a 32-byte seed.
pub fn public_key(seed: &[u8; 32]) -> [u8; 32] {
    let h = Sha512::digest(seed);
    let mut s = [0u8; 32];
    s.copy_from_slice(&h[..32]);
    s[0] &= 248;
    s[31] &= 127;
    s[31] |= 64;
    let d2 = d2();
    let b = base();
    let mut q = Point {
        x: [0; 5],
        y: [1, 0, 0, 0, 0],
        z: [1, 0, 0, 0, 0],
        t: [0; 5],
    };
    for i in (0..256).rev() {
        q = point_add(&q, &q, &d2);
        let r = point_add(&q, &b, &d2);
        q = select(&q, &r, ((s[i / 8] >> (i % 8)) & 1) as u64);
    }
    let zi = invert(&q.z);
    let x = fe_to_bytes(&mul(&q.x, &zi));
    let mut y = fe_to_bytes(&mul(&q.y, &zi));
    y[31] |= (x[0] & 1) << 7;
    s.fill(0);
    y
}

fn string(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

/// `ssh-ed25519 AAAA...` for a public key.
pub fn openssh_public(public: &[u8; 32]) -> String {
    let mut blob = Vec::new();
    string(&mut blob, b"ssh-ed25519");
    string(&mut blob, public);
    format!("ssh-ed25519 {}", STANDARD.encode(blob))
}

/// An unencrypted `openssh-key-v1` private key file for (seed, public). `check` is the random
/// check word the format repeats twice (any value works; callers pass randomness).
pub fn openssh_private(seed: &[u8; 32], public: &[u8; 32], comment: &str, check: u32) -> String {
    let mut pub_blob = Vec::new();
    string(&mut pub_blob, b"ssh-ed25519");
    string(&mut pub_blob, public);
    let mut private = Vec::new();
    private.extend_from_slice(&check.to_be_bytes());
    private.extend_from_slice(&check.to_be_bytes());
    string(&mut private, b"ssh-ed25519");
    string(&mut private, public);
    let mut sk = seed.to_vec();
    sk.extend_from_slice(public);
    string(&mut private, &sk);
    sk.fill(0);
    string(&mut private, comment.as_bytes());
    let mut pad = 1u8;
    while private.len() % 8 != 0 {
        private.push(pad);
        pad += 1;
    }
    let mut file = b"openssh-key-v1\0".to_vec();
    string(&mut file, b"none");
    string(&mut file, b"none");
    string(&mut file, b"");
    file.extend_from_slice(&1u32.to_be_bytes());
    string(&mut file, &pub_blob);
    string(&mut file, &private);
    private.fill(0);
    let encoded = STANDARD.encode(&file);
    file.fill(0);
    let mut out = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
    for chunk in encoded.as_bytes().chunks(70) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push('\n');
    }
    out.push_str("-----END OPENSSH PRIVATE KEY-----\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn rfc8032_vectors() {
        for (seed, public) in [
            (
                "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
                "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            ),
            (
                "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
                "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
            ),
            (
                "c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7",
                "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025",
            ),
            (
                "f5e5767cf153319517630f226876b86c8160cc583bc013744c6bf255f5cc0ee5",
                "278117fc144c72340f67d0f2316e8386ceffbf2b2428c9c51fef7c597f1d426e",
            ),
        ] {
            assert_eq!(public_key(&hex(seed)), hex(public), "seed {seed}");
        }
    }

    #[test]
    fn field_round_trip_reduces() {
        let mut p = [0xffu8; 32];
        p[0] = 0xed;
        p[31] = 0x7f; // p itself encodes as 0
        assert_eq!(fe_to_bytes(&fe_from_bytes(&p)), [0u8; 32]);
        let one = {
            let mut b = [0u8; 32];
            b[0] = 1;
            b
        };
        let x = fe_from_bytes(&hex(
            "1ad5258f602d56c9b2a7259560c72c695cdcd6fd31e2a4c0fe536ecdd3366921",
        ));
        assert_eq!(fe_to_bytes(&mul(&x, &invert(&x))), one);
    }

    /// OpenSSH itself signs with the file and verifies against the derived public key.
    #[test]
    fn openssh_signs_with_the_written_key() {
        if std::process::Command::new("ssh-keygen")
            .arg("-?")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let seed = [7u8; 32];
        let public = public_key(&seed);
        let key = dir.path().join("k");
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&key)
            .unwrap();
        use std::io::Write;
        f.write_all(openssh_private(&seed, &public, "t", 0x1234_5678).as_bytes())
            .unwrap();
        drop(f);
        let y = std::process::Command::new("ssh-keygen")
            .args(["-y", "-f"])
            .arg(&key)
            .output()
            .unwrap();
        assert!(y.status.success());
        let derived = String::from_utf8(y.stdout).unwrap();
        assert!(derived.starts_with(&openssh_public(&public)));
        let msg = dir.path().join("m");
        std::fs::write(&msg, b"cx join").unwrap();
        let sign = std::process::Command::new("ssh-keygen")
            .args(["-Y", "sign", "-n", "cx-test", "-f"])
            .arg(&key)
            .arg(&msg)
            .output()
            .unwrap();
        assert!(
            sign.status.success(),
            "{}",
            String::from_utf8_lossy(&sign.stderr)
        );
        let signers = dir.path().join("allowed");
        std::fs::write(&signers, format!("t {}\n", openssh_public(&public))).unwrap();
        let verify = std::process::Command::new("ssh-keygen")
            .args(["-Y", "verify", "-n", "cx-test", "-I", "t", "-f"])
            .arg(&signers)
            .arg("-s")
            .arg(dir.path().join("m.sig"))
            .stdin(std::fs::File::open(&msg).unwrap())
            .output()
            .unwrap();
        assert!(
            verify.status.success(),
            "signature from the seed verifies against the derived key"
        );
    }
}
