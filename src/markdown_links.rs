//! Bounded inert inline links. Targets remain data until an explicit viewer action.
#[derive(Clone, Debug)]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub label: String,
    pub target: String,
}
pub fn web_target(s: &str) -> bool {
    s.len() <= 2048
        && (s.starts_with("https://") || s.starts_with("http://"))
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
