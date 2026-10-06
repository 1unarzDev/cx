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
}
