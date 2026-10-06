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
fn dispatch(op: model::Operation) -> anyhow::Result<serde_json::Value> {
    files::handle(&op)
}
#[test]
fn integrated_tree_and_resume() {
    let t = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_STATE_HOME", t.path().join("state"));
    let source = t.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(source.join("nested")).unwrap();
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
}
