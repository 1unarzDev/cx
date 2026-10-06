#![allow(dead_code)]
#[path = "../src/files.rs"]
mod files;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/store.rs"]
mod store;
#[path = "../src/transfers.rs"]
mod transfers;
#[path = "../src/transport.rs"]
mod transport;
static MUTATE_SOURCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static MUTATE_DESTINATION: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
fn dispatch(op: model::Operation) -> anyhow::Result<serde_json::Value> {
    use std::sync::atomic::Ordering;
    if let model::Operation::Remove { path, .. } = &op {
        if path.ends_with("cut-mutating-source") && MUTATE_SOURCE.swap(false, Ordering::SeqCst) {
            std::fs::write(files::decode_path(path)?, b"changed before remove")?;
        }
    }
    if let model::Operation::ReadChunk { path, .. } = &op {
        if path.ends_with("cut-mutating-destination")
            && MUTATE_DESTINATION.swap(false, Ordering::SeqCst)
        {
            std::fs::write(files::decode_path(path)?, b"changed destination")?;
        }
    }
    files::handle(&op)
}
#[test]
fn integrated_tree_and_resume() {
    let t = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_STATE_HOME", t.path().join("state"));
    let source = t.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(source.join("nested")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            source.join("nested"),
            std::fs::Permissions::from_mode(0o750),
        )
        .unwrap();
    }
    std::fs::write(source.join("hello\u{1b}[2J.txt"), b"hello world").unwrap();
    std::fs::write(source.join("nested/zero"), b"").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        std::fs::write(
            source.join(std::ffi::OsString::from_vec(b"binary-\xff".to_vec())),
            b"opaque filename",
        )
        .unwrap();
        std::os::unix::fs::symlink("../../outside", source.join("link")).unwrap();
    }
    let destination = t.path().join("destination");
    std::fs::create_dir(&destination).unwrap();
    let device = store::local_device();
    let spec = model::TransferSpec {
        source: device.clone(),
        source_path: source.to_str().unwrap().into(),
        destination: device,
        destination_path: destination.to_str().unwrap().into(),
        conflict: "skip".into(),
        key: "tree".into(),
        cut: false,
        source_identity: None,
    };
    transfers::worker(&spec).unwrap();
    assert_eq!(
        std::fs::read(destination.join("source/hello\u{1b}[2J.txt")).unwrap(),
        b"hello world"
    );
    assert_eq!(
        std::fs::metadata(destination.join("source/nested/zero"))
            .unwrap()
            .len(),
        0
    );
    #[cfg(unix)]
    assert_eq!(
        std::fs::read_link(destination.join("source/link")).unwrap(),
        std::path::Path::new("../../outside")
    );
    assert_eq!(transfers::jobs().unwrap()["jobs"][0]["status"], "complete");
    transfers::worker(&spec).unwrap();
    // Receiver independently verifies prefixes/checksum, retries, and changed identities.
    let target = t.path().join("partial-target");
    let op = model::Operation::ReceivePrepare {
        path: target.to_str().unwrap().into(),
        key: "receive".into(),
        source_identity: "identity-1".into(),
        total: 6,
        conflict: "skip".into(),
        mode: 0o600,
    };
    let state = dispatch(op.clone()).unwrap();
    assert_eq!(state["bytes"], 0);
    let chunk = model::Operation::ReceiveChunk {
        key: "receive".into(),
        offset: 0,
        data: "YWJj".into(),
    };
    dispatch(chunk.clone()).unwrap();
    dispatch(chunk).unwrap();
    assert!(dispatch(model::Operation::ReceiveChunk {
        key: "receive".into(),
        offset: 0,
        data: "YmFk".into()
    })
    .is_err());
    assert_eq!(dispatch(op).unwrap()["bytes"], 3);
    dispatch(model::Operation::ReceiveChunk {
        key: "receive".into(),
        offset: 3,
        data: "ZGVm".into(),
    })
    .unwrap();
    assert!(dispatch(model::Operation::ReceiveFinalize {
        key: "receive".into(),
        sha256: "0".repeat(64)
    })
    .is_err());
    use sha2::{Digest, Sha256};
    let checksum = format!("{:x}", Sha256::digest(b"abcdef"));
    dispatch(model::Operation::ReceiveFinalize {
        key: "receive".into(),
        sha256: checksum.clone(),
    })
    .unwrap();
    dispatch(model::Operation::ReceiveFinalize {
        key: "receive".into(),
        sha256: checksum,
    })
    .unwrap();
    assert_eq!(std::fs::read(target).unwrap(), b"abcdef");
    // Real filesystem pagination beyond the browser's first bounded page.
    let large = t.path().join("large");
    std::fs::create_dir(&large).unwrap();
    for index in 0..1005 {
        std::fs::write(large.join(format!("entry-{index}")), b"").unwrap();
    }
    let first = dispatch(model::Operation::ListPage {
        path: large.to_str().unwrap().into(),
        offset: 0,
        limit: 1000,
    })
    .unwrap();
    assert_eq!(first["entries"].as_array().unwrap().len(), 1000);
    assert_eq!(first["next_offset"], 1000);
    let second = dispatch(model::Operation::ListPage {
        path: large.to_str().unwrap().into(),
        offset: 1000,
        limit: 1000,
    })
    .unwrap();
    assert_eq!(second["entries"].as_array().unwrap().len(), 5);
    assert!(second["next_offset"].is_null());
    assert!(dispatch(model::Operation::ListPage {
        path: large.to_str().unwrap().into(),
        offset: 0,
        limit: 0
    })
    .is_err());
    // Source version changes are rejected before sending bytes.
    let changed = t.path().join("changed");
    std::fs::write(&changed, b"before").unwrap();
    let metadata = dispatch(model::Operation::FileInfo {
        path: changed.to_str().unwrap().into(),
    })
    .unwrap();
    std::fs::write(&changed, b"after changed").unwrap();
    assert!(dispatch(model::Operation::ReadChunk {
        path: changed.to_str().unwrap().into(),
        offset: 0,
        limit: 1024,
        identity: metadata["identity"].as_str().unwrap().into()
    })
    .is_err());
    #[cfg(unix)]
    {
        let link = t.path().join("owned-link");
        std::fs::write(&link, b"existing").unwrap();
        let operation = model::Operation::ReceiveSymlink {
            path: link.to_str().unwrap().into(),
            target: "../../external".into(),
            conflict: "rename".into(),
            key: "link-rename".into(),
        };
        let result = dispatch(operation.clone()).unwrap();
        assert_eq!(dispatch(operation).unwrap()["path"], result["path"]);
        assert_eq!(std::fs::read(&link).unwrap(), b"existing");
        assert_eq!(
            std::fs::read_link(t.path().join("owned-link.copy-1")).unwrap(),
            std::path::Path::new("../../external")
        );
        assert!(!t.path().join("owned-link.copy-2").exists());
        dispatch(model::Operation::ReceiveSymlink {
            path: link.to_str().unwrap().into(),
            target: "external".into(),
            conflict: "overwrite".into(),
            key: "link-overwrite".into(),
        })
        .unwrap();
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            std::path::Path::new("external")
        );
    }
    let mut into_self = spec.clone();
    into_self.key = "into-self".into();
    into_self.destination_path = source.join("child").to_str().unwrap().into();
    assert!(transfers::worker(&into_self).is_err());
    assert!(!source.join("child").exists());
    // Changed endpoints under an existing key fail before launching a service.
    let state = store::ensure().unwrap().join("transfers");
    let saved = state.join("collision.spec.json");
    let mut collision = spec.clone();
    collision.key = "collision".into();
    std::fs::write(saved, serde_json::to_vec(&collision).unwrap()).unwrap();
    collision.destination_path = t.path().join("other").to_str().unwrap().into();
    assert!(transfers::start(&collision).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(destination.join("source/nested"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o750
        );
        let selected = t.path().join("selected");
        let outside = t.path().join("outside");
        let moved = t.path().join("moved");
        std::fs::create_dir(&selected).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let operation = model::Operation::ReceivePrepare {
            path: selected.join("file").to_str().unwrap().into(),
            key: "swap".into(),
            source_identity: "source".into(),
            total: 3,
            conflict: "overwrite".into(),
            mode: 0o600,
        };
        dispatch(operation).unwrap();
        std::fs::rename(&selected, &moved).unwrap();
        std::os::unix::fs::symlink(&outside, &selected).unwrap();
        assert!(dispatch(model::Operation::ReceiveChunk {
            key: "swap".into(),
            offset: 0,
            data: "YWJj".into()
        })
        .is_err());
        assert!(dispatch(model::Operation::Mkdir {
            path: selected.join("injected").to_str().unwrap().into()
        })
        .is_err());
        assert!(!outside.join("file").exists());
        assert!(!outside.join("injected").exists());
        std::fs::remove_file(&selected).unwrap();
        std::fs::rename(&moved, &selected).unwrap();
        dispatch(model::Operation::ReceiveChunk {
            key: "swap".into(),
            offset: 0,
            data: "YWJj".into(),
        })
        .unwrap();
    }
    #[cfg(unix)]
    {
        let dir = t.path().join("mode-owned");
        let moved = t.path().join("mode-moved");
        std::fs::create_dir(&dir).unwrap();
        let metadata = dispatch(model::Operation::FileInfo {
            path: dir.to_str().unwrap().into(),
        })
        .unwrap();
        std::fs::rename(&dir, &moved).unwrap();
        std::fs::create_dir(&dir).unwrap();
        assert!(dispatch(model::Operation::SetPermissions {
            path: dir.to_str().unwrap().into(),
            mode: 0o700,
            expected_identity: Some(metadata["identity"].as_str().unwrap().into())
        })
        .is_err());
    }
    // Distinct long keys must create independent rename receipts.
    let collision_source = t.path().join("key-collision-source");
    let collision_destination = t.path().join("key-collision-destination");
    std::fs::write(&collision_source, b"source").unwrap();
    std::fs::write(&collision_destination, b"original").unwrap();
    let mut copy = spec.clone();
    copy.source_path = collision_source.to_str().unwrap().into();
    copy.destination_path = collision_destination.to_str().unwrap().into();
    copy.conflict = "rename".into();
    copy.key = format!("{}a", "k".repeat(40));
    transfers::worker(&copy).unwrap();
    copy.key = format!("{}b", "k".repeat(40));
    transfers::worker(&copy).unwrap();
    assert_eq!(
        std::fs::read(t.path().join("key-collision-destination.copy-1")).unwrap(),
        b"source"
    );
    assert_eq!(
        std::fs::read(t.path().join("key-collision-destination.copy-2")).unwrap(),
        b"source"
    );

    // Actual local atomic move of a directory preserves inode and contents.
    let moving = t.path().join("moving-directory");
    let moved = t.path().join("moved-directory");
    std::fs::create_dir(&moving).unwrap();
    std::fs::write(moving.join("payload"), b"contents").unwrap();
    let mut cut = spec.clone();
    cut.cut = true;
    cut.key = "atomic-dir".into();
    cut.source_path = moving.to_str().unwrap().into();
    cut.destination_path = moved.to_str().unwrap().into();
    cut.conflict = "rename".into();
    transfers::worker(&cut).unwrap();
    assert!(!moving.exists());
    assert_eq!(std::fs::read(moved.join("payload")).unwrap(), b"contents");
    // A restarted viewer loads the durable spec; completed move is idempotent.
    std::fs::write(
        state.join("atomic-dir.spec.json"),
        serde_json::to_vec(&cut).unwrap(),
    )
    .unwrap();
    assert_eq!(
        transfers::retry("atomic-dir").unwrap()["status"],
        "complete"
    );
    assert!(transfers::retry("../escape").is_err());
    assert!(transfers::retry("unknown").is_err());
    // Lost move response before persisting completion reconciles the exact inode.
    let atomic_job = state.join("atomic-dir.job.json");
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&atomic_job).unwrap()).unwrap();
    receipt["status"] = "failed".into();
    receipt["entries"][0]["status"] = "moving".into();
    std::fs::write(&atomic_job, serde_json::to_vec(&receipt).unwrap()).unwrap();
    transfers::worker(&cut).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&atomic_job).unwrap()).unwrap()
            ["status"],
        "complete"
    );

    // Same-host conflict choices never remove existing content on refusal.
    let local_source = t.path().join("local-move-conflict-source");
    let local_target = t.path().join("local-move-conflict-target");
    std::fs::write(&local_source, b"source bytes").unwrap();
    std::fs::write(&local_target, b"destination bytes").unwrap();
    cut.key = "local-move-overwrite-refused".into();
    cut.source_path = local_source.to_str().unwrap().into();
    cut.destination_path = local_target.to_str().unwrap().into();
    cut.conflict = "overwrite".into();
    assert!(transfers::worker(&cut).is_err());
    assert_eq!(std::fs::read(&local_source).unwrap(), b"source bytes");
    assert_eq!(std::fs::read(&local_target).unwrap(), b"destination bytes");
    cut.key = "local-move-conflict-rename".into();
    cut.conflict = "rename".into();
    transfers::worker(&cut).unwrap();
    assert!(!local_source.exists());
    assert_eq!(
        std::fs::read(t.path().join("local-move-conflict-target.copy-1")).unwrap(),
        b"source bytes"
    );
    #[cfg(unix)]
    {
        let link = t.path().join("moving-symlink");
        let target = t.path().join("moved-symlink");
        std::os::unix::fs::symlink("nonexistent-relative", &link).unwrap();
        cut.key = "local-move-symlink".into();
        cut.source_path = link.to_str().unwrap().into();
        cut.destination_path = target.to_str().unwrap().into();
        transfers::worker(&cut).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(
            std::fs::read_link(target).unwrap(),
            std::path::Path::new("nonexistent-relative")
        );
    }
    let changed_selection = t.path().join("changed-clipboard-source");
    std::fs::write(&changed_selection, b"before").unwrap();
    let selected = dispatch(model::Operation::FileInfo {
        path: changed_selection.to_str().unwrap().into(),
    })
    .unwrap();
    copy.key = "changed-clipboard-identity".into();
    copy.source_path = changed_selection.to_str().unwrap().into();
    copy.source_identity = selected["identity"].as_str().map(str::to_owned);
    copy.destination_path = t
        .path()
        .join("identity-must-not-copy")
        .to_str()
        .unwrap()
        .into();
    std::fs::write(&changed_selection, b"after changed").unwrap();
    assert!(transfers::worker(&copy).is_err());
    assert!(!t.path().join("identity-must-not-copy").exists());

    // Emulated distinct endpoints use actual filesystem receiver/delete adapters,
    // without SSH or live host mutations. Cross-host cut does not touch skips.
    let original = t.path().join("cut-source");
    let target = t.path().join("cut-destination");
    std::fs::write(&original, b"to move").unwrap();
    std::fs::write(&target, b"keep existing").unwrap();
    cut.key = "cut-skip".into();
    cut.source_path = original.to_str().unwrap().into();
    cut.destination_path = target.to_str().unwrap().into();
    cut.destination.account = "fixture-other-account".into();
    cut.conflict = "skip".into();
    assert!(transfers::worker(&cut).is_err());
    assert_eq!(std::fs::read(&original).unwrap(), b"to move");
    assert_eq!(std::fs::read(&target).unwrap(), b"keep existing");
    cut.key = "cut-rename".into();
    cut.conflict = "rename".into();
    transfers::worker(&cut).unwrap();
    assert!(!original.exists());
    assert_eq!(
        std::fs::read(t.path().join("cut-destination.copy-1")).unwrap(),
        b"to move"
    );

    // Replacement during guarded source deletion is retained.
    let original = t.path().join("cut-mutating-source");
    std::fs::write(&original, b"original bytes").unwrap();
    cut.key = "cut-source-race".into();
    cut.source_path = original.to_str().unwrap().into();
    cut.destination_path = t
        .path()
        .join("cut-verified-destination")
        .to_str()
        .unwrap()
        .into();
    MUTATE_SOURCE.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(transfers::worker(&cut).is_err());
    assert_eq!(std::fs::read(&original).unwrap(), b"changed before remove");
    // Modified final destination cannot authorize destructive source cleanup.
    let original = t.path().join("cut-retained-source");
    std::fs::write(&original, b"original bytes").unwrap();
    cut.key = "cut-destination-race".into();
    cut.source_path = original.to_str().unwrap().into();
    cut.destination_path = t
        .path()
        .join("cut-mutating-destination")
        .to_str()
        .unwrap()
        .into();
    MUTATE_DESTINATION.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(transfers::worker(&cut).is_err());
    assert_eq!(std::fs::read(&original).unwrap(), b"original bytes");
    // Unsupported cross-endpoint directory cut rejects before making destination.
    cut.key = "cut-directory-unsupported".into();
    cut.source_path = source.to_str().unwrap().into();
    cut.destination_path = t
        .path()
        .join("unsupported-move-target")
        .to_str()
        .unwrap()
        .into();
    assert!(transfers::worker(&cut).is_err());
    assert!(!t.path().join("unsupported-move-target").exists());

    // Dead queued records become durably resumable, never perpetually working.
    let mut orphan = receipt;
    orphan["key"] = "orphaned-queue".into();
    orphan["status"] = "queued".into();
    orphan["updated"] = 0.into();
    orphan["entries"] = serde_json::json!([]);
    let orphan_path = state.join("orphaned-queue.job.json");
    std::fs::write(&orphan_path, serde_json::to_vec(&orphan).unwrap()).unwrap();
    let jobs = transfers::jobs().unwrap();
    assert_eq!(
        jobs["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["key"] == "orphaned-queue")
            .unwrap()["status"],
        "incomplete"
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(orphan_path).unwrap()).unwrap()
            ["status"],
        "incomplete"
    );
}

#[path = "../src/auth.rs"]
mod auth;
