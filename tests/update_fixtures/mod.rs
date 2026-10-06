use super::*;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};
use tempfile::TempDir;

struct Fixture {
    home: TempDir,
    state: PathBuf,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        fs::set_permissions(home.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let bins = absolute_dir(home.path())
            .unwrap()
            .child(".local", true)
            .unwrap()
            .child("bin", true)
            .unwrap();
        fs::write(bins.path("cx"), b"old executable").unwrap();
        fs::set_permissions(bins.path("cx"), fs::Permissions::from_mode(0o755)).unwrap();
        let state = home.path().join(".local/state/cx");
        let source = home.path().join(".local/bin/cx");
        Self {
            home,
            state,
            source,
        }
    }
    fn check(&self, force: bool, b: &Mock) -> CheckOutcome {
        check_with(
            force,
            self.home.path(),
            &self.state,
            "0.1.0",
            &self.source,
            &Backend::fixture(b),
        )
        .unwrap()
    }
    fn ready(&self) -> UpdatePlan {
        match self.check(true, &Mock::success()) {
            CheckOutcome::Ready(p) => p,
            o => panic!("{o:?}"),
        }
    }
}
struct Mock {
    results: RefCell<VecDeque<Result<Output>>>,
    archive: Vec<u8>,
    calls: Cell<usize>,
    mutate_verify: bool,
}
fn out(code: i32, bytes: &[u8]) -> Result<Output> {
    Ok(Output {
        code,
        bytes: bytes.to_vec(),
    })
}
fn metadata(tag: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "tag_name":tag, "draft":false, "prerelease":false,
        "assets":[{"name":format!("cx-{tag}-linux-x86_64.tar.gz"),"size":100,
        "browser_download_url":format!("https://github.com/{REPO}/releases/download/{tag}/cx-{tag}-linux-x86_64.tar.gz")}]
    })).unwrap()
}
fn response(tag: &str) -> Result<Output> {
    let mut b = metadata(tag);
    b.extend(b"\n200");
    out(0, &b)
}
fn archive(members: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    for (path, kind, bytes) in members {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(*kind);
        h.set_size(bytes.len() as u64);
        h.set_mode(0o755);
        if kind.is_symlink() || kind.is_hard_link() {
            h.set_link_name("elsewhere").unwrap();
        }
        h.set_cksum();
        tar.append_data(&mut h, path, *bytes).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}
impl Mock {
    fn new(results: Vec<Result<Output>>) -> Self {
        Self {
            results: RefCell::new(results.into()),
            archive: archive(&[("cx", tar::EntryType::Regular, b"verified fixture binary")]),
            calls: Cell::new(0),
            mutate_verify: false,
        }
    }
    fn success() -> Self {
        Self::new(vec![
            response("v0.2.0"),
            out(0, b"\n200"),
            out(0, b"verified"),
            out(0, b"cx 0.2.0\n"),
        ])
    }
}
impl TestBackend for Mock {
    fn run(&self, tool: Tool, args: &[String], executable: Option<&Path>) -> Result<Output> {
        self.calls.set(self.calls.get() + 1);
        if matches!(tool, Tool::Curl) && args.iter().any(|arg| arg.ends_with(".intoto.jsonl")) {
            let at = args.iter().position(|arg| arg == "--output").unwrap();
            fs::write(&args[at + 1], b"synthetic provenance bundle").unwrap();
            return out(0, b"\n200");
        }
        match tool {
            Tool::Curl if args.iter().any(|a| a == "--output") => {
                assert!(args.contains(&"--max-filesize".into()));
                let i = args.iter().position(|a| a == "--output").unwrap();
                fs::write(&args[i + 1], &self.archive).unwrap();
                assert!(args
                    .iter()
                    .any(|a| a.starts_with("https://github.com/1unarzDev/cx/releases/download/v")));
            }
            Tool::Curl => assert!(args.contains(&API.into())),
            Tool::Gh => {
                assert_eq!(&args[0..2], &["attestation", "verify"]);
                assert_eq!(
                    &args[5..],
                    &[
                        "--repo",
                        REPO,
                        "--signer-workflow",
                        WORKFLOW,
                        "--source-ref",
                        "refs/tags/v0.2.0",
                        "--deny-self-hosted-runners",
                        "--hostname",
                        "github.com",
                        "--limit",
                        "10"
                    ]
                );
                assert_eq!(args[3], "--bundle");
                assert!(fs::read(&args[4])
                    .unwrap()
                    .starts_with(b"synthetic provenance"));
                if self.mutate_verify {
                    fs::write(&args[2], b"tampered").unwrap();
                }
            }
            Tool::Probe => {
                assert!(executable.is_some());
                assert_eq!(args, &["--version"]);
            }
        }
        self.results
            .borrow_mut()
            .pop_front()
            .expect("unexpected process")
    }
}
#[test]
fn successful_stage_atomic_install_and_rollback() {
    let f = Fixture::new();
    let plan = f.ready();
    assert_eq!(plan.version, "0.2.0");
    assert!(plan.stage.dir.path("metadata").exists());
    let installed = install_with(&plan, &f.source).unwrap();
    assert_eq!(installed, f.source);
    assert!(installed.is_absolute());
    assert_eq!(fs::read(&installed).unwrap(), b"verified fixture binary");
    assert_eq!(
        fs::read(
            installed
                .parent()
                .unwrap()
                .join(format!("cx.rollback-{}", plan.metadata.source_ino))
        )
        .unwrap(),
        b"old executable"
    );
    assert!(install_with(&plan, &f.source).is_err());
}
#[test]
fn offline_timeout_missing_curl_and_no_releases() {
    for (result, expected) in [
        (out(6, b""), "offline"),
        (out(28, b""), "offline"),
        (Err(anyhow::anyhow!("update tool timed out")), "offline"),
        (out(0, b"{}\n404"), "unavailable"),
        (
            Err(std::io::Error::from(std::io::ErrorKind::NotFound).into()),
            "unavailable",
        ),
    ] {
        let f = Fixture::new();
        let b = Mock::new(vec![result]);
        let got = f.check(true, &b);
        assert!(match expected {
            "offline" => matches!(got, CheckOutcome::Offline),
            _ => matches!(got, CheckOutcome::Unavailable(_)),
        });
        assert_eq!(fs::read(&f.source).unwrap(), b"old executable");
        assert!(matches!(f.check(false, &b), CheckOutcome::Skipped));
        assert_eq!(b.calls.get(), 1);
    }
}
#[test]
fn offline_check_recovers_without_replacing_the_current_binary() {
    let f = Fixture::new();
    let b = Mock::new(vec![out(6, b""), response("v0.1.0")]);
    assert!(matches!(f.check(false, &b), CheckOutcome::Offline));
    assert!(matches!(f.check(false, &b), CheckOutcome::Skipped));
    assert_eq!(b.calls.get(), 1, "offline backoff avoids repeated probes");
    assert!(matches!(f.check(true, &b), CheckOutcome::Current));
    assert_eq!(b.calls.get(), 2, "interactive retry bypasses backoff");
    assert_eq!(fs::read(&f.source).unwrap(), b"old executable");
}
#[test]
fn current_downgrade_and_ttl_force() {
    for tag in ["v0.1.0", "v0.0.9"] {
        let f = Fixture::new();
        let b = Mock::new(vec![response(tag), response(tag)]);
        assert!(matches!(f.check(false, &b), CheckOutcome::Current));
        assert!(matches!(f.check(false, &b), CheckOutcome::Skipped));
        assert!(matches!(f.check(true, &b), CheckOutcome::Current));
        assert_eq!(b.calls.get(), 2);
    }
}
#[test]
fn malformed_metadata_versions_and_assets() {
    for tag in [
        "v01.2.0",
        "v0.2",
        "v0.2.0-rc1",
        "v0.2.0+meta",
        "V0.2.0",
        "v0.2.0/evil",
        "v18446744073709551616.0.0",
    ] {
        assert!(release(&metadata(tag), "0.1.0").is_err());
    }
    for field in ["draft", "prerelease"] {
        let mut v: serde_json::Value = serde_json::from_slice(&metadata("v0.2.0")).unwrap();
        v[field] = true.into();
        assert!(release(&serde_json::to_vec(&v).unwrap(), "0.1.0").is_err());
    }
    let mut v: serde_json::Value = serde_json::from_slice(&metadata("v0.2.0")).unwrap();
    v["assets"][0]["browser_download_url"] = "https://evil.example/cx".into();
    assert!(release(&serde_json::to_vec(&v).unwrap(), "0.1.0").is_err());
    let f = Fixture::new();
    let b = Mock::new(vec![out(0, b"not-json\n200")]);
    assert!(matches!(f.check(true, &b), CheckOutcome::Unavailable(_)));
    assert_eq!(b.calls.get(), 1);
}
#[test]
fn missing_gh_signature_tag_and_probe_failure_never_install() {
    for failed in [
        out(1, b"rejected signer/tag"),
        Err(std::io::Error::from(std::io::ErrorKind::NotFound).into()),
    ] {
        let f = Fixture::new();
        let b = Mock::new(vec![response("v0.2.0"), out(0, b"\n200"), failed]);
        assert!(matches!(f.check(true, &b), CheckOutcome::Unavailable(_)));
        assert_eq!(b.calls.get(), 4);
        assert_eq!(fs::read(&f.source).unwrap(), b"old executable");
        assert_eq!(fs::read_dir(f.state.join("update")).unwrap().count(), 0);
    }
    for probe in [out(0, b"cx 9.9.9\n"), out(1, b"cx 0.2.0\n")] {
        let f = Fixture::new();
        let b = Mock::new(vec![
            response("v0.2.0"),
            out(0, b"\n200"),
            out(0, b""),
            probe,
        ]);
        assert!(matches!(f.check(true, &b), CheckOutcome::Unavailable(_)));
    }
    let f = Fixture::new();
    let mut b = Mock::success();
    b.mutate_verify = true;
    assert!(matches!(f.check(true, &b), CheckOutcome::Unavailable(_)));
    assert_eq!(b.calls.get(), 4);
}
#[test]
fn archive_rejects_links_extras_paths_truncation_and_bombs() {
    let f = Fixture::new();
    let dir = state_dir(f.home.path(), &f.state)
        .unwrap()
        .child("update", true)
        .unwrap();
    let mut cases = vec![
        archive(&[("cx", tar::EntryType::Symlink, b"")]),
        archive(&[("cx", tar::EntryType::Link, b"")]),
        archive(&[("cx", tar::EntryType::Directory, b"")]),
        archive(&[("other", tar::EntryType::Regular, b"x")]),
        archive(&[
            ("cx", tar::EntryType::Regular, b"x"),
            ("extra", tar::EntryType::Regular, b"x"),
        ]),
        archive(&[("cx", tar::EntryType::Regular, b"")]),
    ];
    let good = archive(&[("cx", tar::EntryType::Regular, b"binary")]);
    cases.push(good[..good.len() - 5].to_vec());
    let mut bad_crc = good.clone();
    let last = bad_crc.len() - 8;
    bad_crc[last] ^= 1;
    cases.push(bad_crc);
    // Raw header fixture through tar crate: path traversal rejected without unpacking.
    let mut raw = Vec::new();
    let mut h = tar::Header::new_gnu();
    h.as_mut_bytes()[..5].copy_from_slice(b"../cx");
    h.set_size(1);
    h.set_entry_type(tar::EntryType::Regular);
    h.set_cksum();
    raw.extend_from_slice(h.as_bytes());
    raw.push(b'x');
    raw.resize(2048, 0);
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&raw).unwrap();
    cases.push(encoder.finish().unwrap());
    // Declared oversize is rejected before allocating/decompressing its content.
    h.set_size(BINARY_LIMIT + 1);
    h.as_mut_bytes()[..5].fill(0);
    h.as_mut_bytes()[..2].copy_from_slice(b"cx");
    h.set_cksum();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(h.as_bytes()).unwrap();
    cases.push(encoder.finish().unwrap());
    let mut extra = good.clone();
    extra.extend(&good);
    cases.push(extra);
    for bytes in cases {
        let s = stage(Dir(dir.0.try_clone().unwrap())).unwrap();
        let mut a = new_file(&s.dir.path("archive")).unwrap();
        a.write_all(&bytes).unwrap();
        assert!(extract(a, &s.dir).is_err());
    }
}
#[test]
fn replaced_stage_metadata_symlinks_and_target_refused() {
    for mode in 0..7 {
        let f = Fixture::new();
        let p = f.ready();
        match mode {
            0 => fs::write(p.stage.dir.path("cx"), b"tampered").unwrap(),
            1 => {
                fs::remove_file(p.stage.dir.path("cx")).unwrap();
                fs::write(p.stage.dir.path("cx"), b"verified fixture binary").unwrap();
            }
            2 => {
                fs::remove_file(p.stage.dir.path("cx")).unwrap();
                std::os::unix::fs::symlink(&f.source, p.stage.dir.path("cx")).unwrap();
            }
            3 => fs::write(p.stage.dir.path("metadata"), b"{}").unwrap(),
            4 => {
                fs::rename(&f.source, f.source.with_extension("old")).unwrap();
                std::os::unix::fs::symlink(f.source.with_extension("old"), &f.source).unwrap();
            }
            5 => fs::set_permissions(&f.source, fs::Permissions::from_mode(0o500)).unwrap(),
            6 => {
                fs::rename(
                    p.stage.parent.path(&p.stage.name),
                    p.stage.parent.path("moved"),
                )
                .unwrap();
                fs::create_dir(p.stage.parent.path(&p.stage.name)).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(install_with(&p, &f.source).is_err(), "mode {mode}");
    }
}
#[test]
fn source_mismatch_rollback_collision_and_concurrent_locks() {
    let f = Fixture::new();
    let p = f.ready();
    let other = f.home.path().join("different-running-cx");
    fs::write(&other, b"other").unwrap();
    assert!(install_with(&p, &other).is_err());
    let root = state_dir(f.home.path(), &f.state).unwrap();
    let maintenance = lock(&root, "maintenance.lock").unwrap();
    assert!(install_with(&p, &f.source).is_err());
    drop(maintenance);
    let check = lock(&root, "update-check.lock").unwrap();
    let b = Mock::new(vec![]);
    assert!(matches!(f.check(true, &b), CheckOutcome::Skipped));
    drop(check);
    let rollback = f
        .source
        .parent()
        .unwrap()
        .join(format!("cx.rollback-{}", p.metadata.source_ino));
    fs::write(&rollback, b"existing").unwrap();
    assert!(install_with(&p, &f.source).is_err());
    assert_eq!(fs::read(&rollback).unwrap(), b"existing");
    assert_eq!(fs::read(&f.source).unwrap(), b"old executable");
}
#[test]
fn interrupted_stage_is_never_adopted_or_removed() {
    let f = Fixture::new();
    let root = state_dir(f.home.path(), &f.state).unwrap();
    let dir = root.child("update", true).unwrap();
    let stale = dir.child("stage-interrupted", true).unwrap();
    fs::write(stale.path("cx"), b"unverified").unwrap();
    let p = f.ready();
    drop(p);
    assert_eq!(fs::read(stale.path("cx")).unwrap(), b"unverified");
    assert_eq!(fs::read_dir(f.state.join("update")).unwrap().count(), 1);
}
#[test]
fn bounded_real_process_outcomes_without_network() {
    let args = |s: &str| vec!["-c".to_owned(), s.to_owned()];
    let o = bounded(
        Path::new("/bin/sh"),
        &args("printf 'cx 0.2.0\\n'; exit 7"),
        Duration::from_secs(1),
        1024,
    )
    .unwrap();
    assert_eq!(o.code, 7);
    assert_eq!(o.bytes, b"cx 0.2.0\n");
    assert!(bounded(
        Path::new("/bin/sh"),
        &args("while :; do :; done"),
        Duration::from_millis(50),
        1024
    )
    .is_err());
    assert!(bounded(Path::new("/bin/sh"),&args("while :; do printf xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; done"),Duration::from_secs(1),32).is_err());
    assert!(bounded(
        Path::new("/nonexistent/cx-fixture"),
        &[],
        Duration::from_secs(1),
        32
    )
    .is_err());
}

#[test]
fn fixture_provenance_gate_then_actual_probe_install_and_relaunch() {
    struct RealProbe(Mock);
    impl TestBackend for RealProbe {
        fn run(&self, tool: Tool, args: &[String], executable: Option<&Path>) -> Result<Output> {
            if matches!(tool, Tool::Probe) {
                assert_eq!(self.0.calls.get(), 4, "provenance must precede execution");
                bounded(executable.unwrap(), args, Duration::from_secs(1), 1024)
            } else {
                self.0.run(tool, args, executable)
            }
        }
    }
    let f = Fixture::new();
    let mut mock = Mock::new(vec![
        response("v0.2.0"),
        out(0, b"\n200"),
        out(0, b"verified"),
    ]);
    mock.archive=archive(&[("cx",tar::EntryType::Regular,b"#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'cx 0.2.0\\n'; else printf 'relaunch fixture\\n'; fi\n")]);
    let b = RealProbe(mock);
    let outcome = check_with(
        true,
        f.home.path(),
        &f.state,
        "0.1.0",
        &f.source,
        &Backend::fixture(&b),
    )
    .unwrap();
    let CheckOutcome::Ready(plan) = outcome else {
        panic!("{outcome:?}")
    };
    let installed = install_with(&plan, &f.source).unwrap();
    let result = bounded(&installed, &[], Duration::from_secs(1), 1024).unwrap();
    assert_eq!(result.code, 0);
    assert_eq!(result.bytes, b"relaunch fixture\n");
}

#[test]
fn cache_symlink_state_bin_symlink_and_hardlink_refused() {
    let f = Fixture::new();
    let root = state_dir(f.home.path(), &f.state).unwrap();
    std::os::unix::fs::symlink(&f.source, root.path("update-cache.json")).unwrap();
    let b = Mock::new(vec![response("v0.1.0")]);
    assert!(check_with(
        false,
        f.home.path(),
        &f.state,
        "0.1.0",
        &f.source,
        &Backend::fixture(&b)
    )
    .is_err());
    assert_eq!(fs::read(&f.source).unwrap(), b"old executable");
    let f = Fixture::new();
    let p = f.ready();
    fs::hard_link(&f.source, f.home.path().join("alias")).unwrap();
    assert!(install_with(&p, &f.source).is_err());
    let f = Fixture::new();
    let p = f.ready();
    let bins = f.source.parent().unwrap();
    let moved = bins.with_extension("old");
    fs::rename(bins, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, bins).unwrap();
    assert!(install_with(&p, &f.source).is_err());
    let f = Fixture::new();
    let p = f.ready();
    fs::set_permissions(
        f.source.parent().unwrap(),
        fs::Permissions::from_mode(0o500),
    )
    .unwrap();
    assert!(install_with(&p, &f.source).is_err());
    fs::set_permissions(
        f.source.parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    // Read-only foreign ownership evidence, without chown or privileged operations.
    if uid() != 0 {
        assert!(absolute_dir(Path::new("/")).is_err());
    }
}

#[test]
fn tar_terminator_truncation_and_actual_expansion_bomb() {
    let f = Fixture::new();
    let dir = state_dir(f.home.path(), &f.state)
        .unwrap()
        .child("update", true)
        .unwrap();
    let compressed = archive(&[("cx", tar::EntryType::Regular, b"binary")]);
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(compressed.as_slice())
        .read_to_end(&mut raw)
        .unwrap();
    // Valid gzip containing a tar cut before its mandatory end markers.
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&raw[..1024]).unwrap();
    let truncated = e.finish().unwrap();
    // Small compressed payload with a huge zero tail; decompression is bounded.
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(&raw).unwrap();
    let zeroes = [0; 65536];
    for _ in 0..(TAR_LIMIT / 65536 + 1) {
        e.write_all(&zeroes).unwrap();
    }
    let bomb = e.finish().unwrap();
    assert!((bomb.len() as u64) < ARCHIVE_LIMIT);
    for bytes in [truncated, bomb] {
        let s = stage(Dir(dir.0.try_clone().unwrap())).unwrap();
        let mut a = new_file(&s.dir.path("archive")).unwrap();
        a.write_all(&bytes).unwrap();
        assert!(extract(a, &s.dir).is_err());
    }
}

#[test]
fn existing_cli_lock_permissions_share_same_inode() {
    let f = Fixture::new();
    let p = f.ready();
    let root = state_dir(f.home.path(), &f.state).unwrap();
    let lock_path = root.path("maintenance.lock");
    fs::write(&lock_path, b"").unwrap();
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o644)).unwrap();
    let before = fs::metadata(&lock_path).unwrap().ino();
    let held = lock(&root, "maintenance.lock").unwrap();
    assert!(install_with(&p, &f.source).is_err());
    drop(held);
    assert!(install_with(&p, &f.source).is_ok());
    assert_eq!(fs::metadata(&lock_path).unwrap().ino(), before);
}
