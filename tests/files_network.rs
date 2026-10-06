#[path = "../src/files.rs"]
mod files;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/network.rs"]
mod network;

#[cfg(unix)]
mod actions {
    use super::{files, model::Operation};
    use std::{
        fs,
        os::unix::{ffi::OsStringExt, fs::symlink},
    };
    use tempfile::tempdir;
    fn token(path: &std::path::Path) -> String {
        files::handle(&Operation::FileInfo {
            path: files::encode_path(path),
        })
        .unwrap()["identity"]
            .as_str()
            .unwrap()
            .into()
    }
    #[test]
    fn rename_never_overwrites_and_rejects_changed_selection() {
        let t = tempdir().unwrap();
        let a = t.path().join("a");
        let b = t.path().join("b");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        let old = token(&a);
        let rename = |name: &str| Operation::Rename {
            path: files::encode_path(&a),
            name: name.into(),
            expected_identity: Some(old.clone()),
        };
        assert!(files::handle(&rename("b")).is_err());
        assert!(files::handle(&rename("../escaped")).is_err());
        assert_eq!(fs::read(&b).unwrap(), b"b");
        fs::rename(&a, t.path().join("original")).unwrap();
        fs::write(&a, b"replacement").unwrap();
        assert!(files::handle(&rename("new")).is_err());
        assert!(a.exists());
    }
    #[test]
    fn rename_opaque_source_and_literal_protocol_prefix_name() {
        let t = tempdir().unwrap();
        let a = t
            .path()
            .join(std::ffi::OsString::from_vec(vec![b'-', 0xff]));
        fs::write(&a, b"payload").unwrap();
        files::handle(&Operation::Rename {
            path: files::encode_path(&a),
            name: "cx-bytes:literal".into(),
            expected_identity: Some(token(&a)),
        })
        .unwrap();
        assert_eq!(
            fs::read(t.path().join("cx-bytes:literal")).unwrap(),
            b"payload"
        );
    }
    #[test]
    fn removal_does_not_follow_symlinks_and_removes_a_tree() {
        let t = tempdir().unwrap();
        let outside = t.path().join("outside");
        let selected = t.path().join("selected");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"keep").unwrap();
        fs::create_dir_all(selected.join("nested")).unwrap();
        fs::write(selected.join("nested/file"), b"remove").unwrap();
        symlink(&outside, selected.join("link")).unwrap();
        files::handle(&Operation::Remove {
            path: files::encode_path(&selected),
            expected_identity: Some(token(&selected)),
        })
        .unwrap();
        assert!(!selected.exists());
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
        let link = t.path().join("link");
        symlink(&outside, &link).unwrap();
        files::handle(&Operation::Remove {
            path: files::encode_path(&link),
            expected_identity: Some(token(&link)),
        })
        .unwrap();
        assert!(outside.exists());
    }
    #[test]
    fn changed_identity_and_symlink_ancestors_prevent_deletion() {
        let t = tempdir().unwrap();
        let parent = t.path().join("parent");
        let moved = t.path().join("moved");
        let outside = t.path().join("outside");
        fs::create_dir(&parent).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(parent.join("f"), b"original").unwrap();
        fs::write(outside.join("f"), b"keep").unwrap();
        let old = token(&parent.join("f"));
        fs::rename(&parent, &moved).unwrap();
        symlink(&outside, &parent).unwrap();
        assert!(files::handle(&Operation::Remove {
            path: files::encode_path(&parent.join("f")),
            expected_identity: Some(old.clone())
        })
        .is_err());
        assert_eq!(fs::read(outside.join("f")).unwrap(), b"keep");
        assert!(files::handle(&Operation::Remove {
            path: files::encode_path(&outside.join("f")),
            expected_identity: Some(old)
        })
        .is_err());
        assert!(files::handle(&Operation::Remove {
            path: "/".into(),
            expected_identity: None
        })
        .is_err());
        assert!(files::handle(&Operation::Remove {
            path: "~".into(),
            expected_identity: None
        })
        .is_err());
    }
    #[test]
    fn listing_exposes_hidden_identity_and_safe_rename_seed() {
        let t = tempdir().unwrap();
        fs::write(t.path().join(".hidden"), b"a").unwrap();
        fs::write(t.path().join("bad\u{1b}"), b"b").unwrap();
        let listing = files::handle(&Operation::List {
            path: files::encode_path(t.path()),
        })
        .unwrap();
        let entries = listing["entries"].as_array().unwrap();
        let hidden = entries.iter().find(|e| e["name"] == ".hidden").unwrap();
        assert_eq!(hidden["hidden"], true);
        assert_eq!(hidden["rename_name"], ".hidden");
        assert!(hidden["identity"].is_string());
        let bad = entries
            .iter()
            .find(|e| e["name"].as_str().unwrap().starts_with("bad"))
            .unwrap();
        assert!(bad["rename_name"].is_null());
    }
    #[test]
    fn move_directory_preserves_tree_and_refuses_existing_destination() {
        let t = tempdir().unwrap();
        let a = t.path().join("a");
        let dest = t.path().join("dest");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&dest).unwrap();
        fs::write(a.join("file"), b"payload").unwrap();
        let old = token(&a);
        let to = dest.join("moved");
        files::handle(&Operation::Move {
            path: files::encode_path(&a),
            destination: files::encode_path(&to),
            expected_identity: Some(old),
        })
        .unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read(to.join("file")).unwrap(), b"payload");
        fs::create_dir(&a).unwrap();
        assert!(files::handle(&Operation::Move {
            path: files::encode_path(&a),
            destination: files::encode_path(&to),
            expected_identity: Some(token(&a))
        })
        .is_err());
        assert!(a.exists());
    }
}
