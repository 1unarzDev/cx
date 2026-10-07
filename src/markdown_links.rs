//! Bounded inert inline links. Targets remain data until an explicit viewer action.
use std::path::{Path, PathBuf};

/// Join a file link to the document directory without viewer filesystem access.
/// Preserve parent traversal: only the execution host can resolve symlinks correctly.
/// Fragments are percent-decoded UTF-8; Unix paths may contain non-UTF-8 bytes.
pub fn resolve_file_link(
    document: &Path,
    href: &str,
) -> anyhow::Result<Option<(PathBuf, Option<String>)>> {
    anyhow::ensure!(href.len() <= 4096, "file link exceeds 4096 bytes");
    anyhow::ensure!(
        !href.chars().any(char::is_control),
        "control character in link"
    );
    let scheme = href
        .split(['/', '?', '#'])
        .next()
        .and_then(|first| first.split_once(':').map(|(scheme, _)| scheme));
    if let Some(scheme) = scheme {
        if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
            return Ok(None);
        }
        anyhow::bail!("unsupported URI scheme");
    }
    anyhow::ensure!(!href.starts_with("//"), "URI authority is not a file path");
    let (path, fragment) = href
        .split_once('#')
        .map_or((href, None), |(p, f)| (p, Some(f)));
    anyhow::ensure!(
        !path.contains('?'),
        "queries are not supported for file links"
    );
    let fragment = fragment
        .map(|fragment| -> anyhow::Result<String> {
            anyhow::ensure!(fragment.len() <= 1024, "fragment exceeds 1024 bytes");
            Ok(String::from_utf8(decode_link_component(fragment)?)?)
        })
        .transpose()?;
    let path = decode_link_component(path)?;
    anyhow::ensure!(!path.starts_with(b"//"), "URI authority is not a file path");
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(path))
    };
    #[cfg(not(unix))]
    let path = PathBuf::from(String::from_utf8(path)?);
    let resolved = if path.as_os_str().is_empty() {
        document.to_path_buf()
    } else if path.is_absolute() {
        path
    } else {
        document
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(path)
    };
    Ok(Some((resolved, fragment)))
}

fn decode_link_component(component: &str) -> anyhow::Result<Vec<u8>> {
    let mut result = Vec::with_capacity(component.len());
    let mut bytes = component.bytes();
    while let Some(byte) = bytes.next() {
        result.push(if byte == b'%' {
            let high = bytes.next().and_then(|b| (b as char).to_digit(16));
            let low = bytes.next().and_then(|b| (b as char).to_digit(16));
            match (high, low) {
                (Some(high), Some(low)) => (high * 16 + low) as u8,
                _ => anyhow::bail!("invalid percent escape"),
            }
        } else {
            byte
        });
    }
    anyhow::ensure!(
        !result.iter().any(|b| b.is_ascii_control()),
        "control character in link"
    );
    for chunk in result.utf8_chunks() {
        anyhow::ensure!(
            !chunk.valid().chars().any(char::is_control),
            "control character in link"
        );
    }
    Ok(result)
}

#[derive(Clone, Debug)]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub label: String,
    pub target: String,
}
pub fn web_target(s: &str) -> bool {
    s.len() <= 2048
        && s.split_once("://").is_some_and(|(scheme, _)| {
            scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")
        })
        && s.split_once("://")
            .is_some_and(|(_, host)| !host.is_empty() && !host.starts_with('/'))
        && !s.chars().any(|c| c.is_control() || c.is_whitespace())
}
pub fn links(s: &str) -> Vec<Link> {
    let mut result = Vec::new();
    let mut at = 0;
    let bytes = s.as_bytes();
    while at < bytes.len() && result.len() < 128 {
        if bytes[at] == b'`' {
            let run = bytes[at..].iter().take_while(|c| **c == b'`').count();
            if let Some(end) = s[at + run..].find(&"`".repeat(run)) {
                at += run + end + run;
                continue;
            }
            break;
        }
        if bytes[at] != b'[' || (at > 0 && (bytes[at - 1] == b'!' || bytes[at - 1] == b'\\')) {
            at += 1;
            continue;
        }
        let Some(label_end) = bytes[at + 1..]
            .windows(2)
            .take(1024)
            .position(|pair| pair == b"](")
            .map(|i| at + 1 + i)
        else {
            at += 1;
            continue;
        };
        if label_end - at > 1024 || s[at + 1..label_end].contains(['[', '\n']) {
            at += 1;
            continue;
        }
        let start = label_end + 2;
        let mut end = start;
        let mut depth = 0;
        while end < bytes.len() && end - start <= 2048 {
            match bytes[end] {
                b'(' => depth += 1,
                b')' if depth == 0 => break,
                b')' => depth -= 1,
                b'\n' => break,
                _ => (),
            }
            end += 1;
        }
        if bytes.get(end) != Some(&b')') {
            at += 1;
            continue;
        }
        let target = s[start..end].trim();
        // Optional quoted title is presentation data, never part of the opener argv.
        let target = target
            .strip_prefix('<')
            .and_then(|s| s.split_once('>').map(|(s, _)| s))
            .unwrap_or_else(|| target.split_once(" \"").map(|(s, _)| s).unwrap_or(target));
        result.push(Link {
            start: at,
            end: end + 1,
            label: s[at + 1..label_end].to_owned(),
            target: target.to_owned(),
        });
        at = end + 1;
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_relative_absolute_and_same_document_links() {
        let document = Path::new("/remote/docs/guide.md");
        for (href, path, fragment) in [
            ("./child.md", "/remote/docs/child.md", None),
            (
                "../other/./intro.md#heading%20one",
                "/remote/docs/../other/./intro.md",
                Some("heading one"),
            ),
            ("/other/../root.md", "/other/../root.md", None),
            ("#section", "/remote/docs/guide.md", Some("section")),
            ("", "/remote/docs/guide.md", None),
            ("#", "/remote/docs/guide.md", Some("")),
            ("../../../file.md", "/remote/docs/../../../file.md", None),
            ("a%20b%23c%3Fd.md", "/remote/docs/a b#c?d.md", None),
        ] {
            assert_eq!(
                resolve_file_link(document, href).unwrap(),
                Some((PathBuf::from(path), fragment.map(str::to_owned))),
                "{href}"
            );
        }
        assert_eq!(
            resolve_file_link(Path::new("docs/guide.md"), "../../intro.md").unwrap(),
            Some((PathBuf::from("docs/../../intro.md"), None))
        );
    }

    #[cfg(unix)]
    #[test]
    fn parent_traversal_preserves_execution_host_symlink_semantics() {
        let root = tempfile::tempdir().unwrap();
        let docs = root.path().join("docs");
        let elsewhere = root.path().join("elsewhere");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::create_dir_all(elsewhere.join("sub")).unwrap();
        std::os::unix::fs::symlink(elsewhere.join("sub"), docs.join("jump")).unwrap();
        std::fs::write(docs.join("page.md"), "wrong").unwrap();
        std::fs::write(elsewhere.join("page.md"), "right").unwrap();
        let (path, _) = resolve_file_link(&docs.join("index.md"), "jump/../page.md")
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "right");
    }

    #[test]
    fn excludes_web_and_rejects_unsafe_or_invalid_file_links() {
        let document = Path::new("/remote/guide.md");
        for href in [
            "https://example.com/a?q=1#x",
            "http://example.com",
            "HTTPS://example.com",
        ] {
            assert_eq!(resolve_file_link(document, href).unwrap(), None);
        }
        for href in [
            "file:///tmp/a",
            "ssh:host",
            "javascript:alert(1)",
            "//host/path",
            "%2f%2fhost/path",
            "a?query",
            "a?query#anchor",
            "a%",
            "a%2",
            "a%GG",
            "a#bad%",
            "a\0",
            "a\n",
            "a%00",
            "a%1b",
            "a%7F",
            "a%C2%85",
            "a%FF%C2%85",
            "a#%FF",
        ] {
            assert!(resolve_file_link(document, href).is_err(), "{href:?}");
        }
        assert!(resolve_file_link(document, &"a".repeat(4097)).is_err());
        assert!(resolve_file_link(document, &format!("#{}", "a".repeat(1025))).is_err());
        assert!(resolve_file_link(document, &"a".repeat(4096)).is_ok());
        assert!(resolve_file_link(document, &format!("#{}", "a".repeat(1024))).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn decodes_non_utf8_unix_path_bytes_without_touching_filesystem() {
        use std::os::unix::ffi::OsStrExt;
        let (path, fragment) = resolve_file_link(
            Path::new("/unmounted/remote/guide.md"),
            "../%FF%FE.md#caf%C3%A9",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            path.as_os_str().as_bytes(),
            b"/unmounted/remote/../\xff\xfe.md"
        );
        assert_eq!(fragment.as_deref(), Some("café"));
    }

    #[test]
    fn links_are_inert_bounded_and_skip_code_images() {
        let found=links("`[code](https://no)` ![image](https://no) [文](https://a/b_(c)) [two](https://b \"Title\")");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].label, "文");
        assert_eq!(found[0].target, "https://a/b_(c)");
        assert_eq!(found[1].target, "https://b");
        assert!(!web_target("javascript:alert(1)"));
        assert!(!web_target("https://a\x1b]8;;evil"));
        assert!(!web_target("file:///tmp/evil"));
        assert!(!web_target("https://a b"));
        assert!(web_target("https://example.org"));
    }
}
