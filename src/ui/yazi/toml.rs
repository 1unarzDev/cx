//! A tiny TOML subset reader for the user's yazi files (theme.toml, keymap.toml,
//! yazi.toml): `[section]`, `[[array.of.tables]]`, `key = value` with strings,
//! integers, booleans, arrays (multi-line) and inline tables, plus `#` comments.
//! Anything it cannot read is skipped, never fatal: a broken file means defaults.
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Arr(Vec<Value>),
    Table(BTreeMap<String, Value>),
}

impl Value {
    pub(crate) fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Table(t) => t.get(key),
            _ => None,
        }
    }
    pub(crate) fn arr(&self) -> &[Value] {
        match self {
            Value::Arr(a) => a,
            _ => &[],
        }
    }
    /// `a.b.c` lookup through nested tables.
    pub(crate) fn path(&self, dotted: &str) -> Option<&Value> {
        dotted.split('.').try_fold(self, |v, k| v.get(k))
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    /// Spaces and tabs (and, when `lines`, newlines and comments too).
    fn ws(&mut self, lines: bool) {
        while let Some(c) = self.peek() {
            match c {
                b' ' | b'\t' | b'\r' => self.i += 1,
                b'\n' if lines => self.i += 1,
                b'#' => {
                    while self.peek().is_some_and(|c| c != b'\n') {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
    }
    fn skip_line(&mut self) {
        while let Some(c) = self.peek() {
            self.i += 1;
            if c == b'\n' {
                break;
            }
        }
    }
    fn key(&mut self) -> Option<String> {
        self.ws(false);
        match self.peek()? {
            b'"' | b'\'' => self.string(),
            _ => {
                let start = self.i;
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                {
                    self.i += 1;
                }
                (self.i > start)
                    .then(|| String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
            }
        }
    }
    /// A dotted key: `a.b.c`.
    fn dotted(&mut self) -> Option<Vec<String>> {
        let mut parts = vec![self.key()?];
        loop {
            self.ws(false);
            if self.peek() != Some(b'.') {
                return Some(parts);
            }
            self.i += 1;
            parts.push(self.key()?);
        }
    }
    fn string(&mut self) -> Option<String> {
        let quote = self.peek()?;
        self.i += 1;
        let mut out = Vec::new();
        while let Some(c) = self.peek() {
            if c == b'\n' {
                // Unterminated: the next line is read on its own.
                return None;
            }
            self.i += 1;
            if c == quote {
                return Some(String::from_utf8_lossy(&out).into_owned());
            }
            if c == b'\\' && quote == b'"' {
                let e = self.peek()?;
                self.i += 1;
                match e {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'u' | b'U' => {
                        let n = if e == b'u' { 4 } else { 8 };
                        let hex = std::str::from_utf8(self.s.get(self.i..self.i + n)?).ok()?;
                        self.i += n;
                        let ch = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
                        out.extend_from_slice(ch.to_string().as_bytes());
                    }
                    other => out.push(other),
                }
            } else {
                out.push(c);
            }
        }
        None
    }
    fn value(&mut self) -> Option<Value> {
        self.ws(false);
        match self.peek()? {
            b'"' | b'\'' => self.string().map(Value::Str),
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws(true);
                    match self.peek()? {
                        b']' => {
                            self.i += 1;
                            return Some(Value::Arr(items));
                        }
                        b',' => self.i += 1,
                        _ => items.push(self.value()?),
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut table = BTreeMap::new();
                loop {
                    self.ws(true);
                    match self.peek()? {
                        b'}' => {
                            self.i += 1;
                            return Some(Value::Table(table));
                        }
                        b',' => self.i += 1,
                        _ => {
                            let key = self.dotted()?;
                            self.ws(false);
                            if self.peek()? != b'=' {
                                return None;
                            }
                            self.i += 1;
                            let v = self.value()?;
                            insert(&mut table, &key, v);
                        }
                    }
                }
            }
            _ => {
                let start = self.i;
                while self.peek().is_some_and(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'+' | b'_' | b'.')
                }) {
                    self.i += 1;
                }
                let word = std::str::from_utf8(&self.s[start..self.i]).ok()?;
                match word {
                    "true" => Some(Value::Bool(true)),
                    "false" => Some(Value::Bool(false)),
                    _ => word.replace('_', "").parse().ok().map(Value::Int),
                }
            }
        }
    }
}

fn insert(table: &mut BTreeMap<String, Value>, key: &[String], v: Value) {
    let Some((last, path)) = key.split_last() else {
        return;
    };
    let mut t = table;
    for k in path {
        let slot = t
            .entry(k.clone())
            .or_insert_with(|| Value::Table(BTreeMap::new()));
        if !matches!(slot, Value::Table(_)) {
            *slot = Value::Table(BTreeMap::new());
        }
        let Value::Table(next) = slot else { return };
        t = next;
    }
    t.insert(last.clone(), v);
}

/// The table at `path`, created as needed; for an array of tables, its last table.
fn table_at<'a>(
    root: &'a mut BTreeMap<String, Value>,
    path: &[String],
) -> &'a mut BTreeMap<String, Value> {
    let mut t = root;
    for k in path {
        let slot = t
            .entry(k.clone())
            .or_insert_with(|| Value::Table(BTreeMap::new()));
        if let Value::Arr(items) = slot {
            if !matches!(items.last(), Some(Value::Table(_))) {
                items.push(Value::Table(BTreeMap::new()));
            }
            let Some(Value::Table(last)) = items.last_mut() else {
                unreachable!()
            };
            t = last;
            continue;
        }
        if !matches!(slot, Value::Table(_)) {
            *slot = Value::Table(BTreeMap::new());
        }
        let Value::Table(next) = slot else {
            unreachable!()
        };
        t = next;
    }
    t
}

/// Parse a document; unreadable lines are skipped.
pub(crate) fn parse(text: &str) -> Value {
    let mut root = BTreeMap::new();
    let mut current: Vec<String> = Vec::new();
    let mut p = P {
        s: text.as_bytes(),
        i: 0,
    };
    loop {
        p.ws(true);
        let Some(c) = p.peek() else { break };
        if c == b'[' {
            p.i += 1;
            let array = p.peek() == Some(b'[');
            if array {
                p.i += 1;
            }
            let Some(path) = p.dotted() else {
                p.skip_line();
                continue;
            };
            if array {
                let (last, parent) = path.split_last().unwrap();
                let parent = table_at(&mut root, parent);
                let slot = parent
                    .entry(last.clone())
                    .or_insert_with(|| Value::Arr(Vec::new()));
                if !matches!(slot, Value::Arr(_)) {
                    *slot = Value::Arr(Vec::new());
                }
                if let Value::Arr(items) = slot {
                    items.push(Value::Table(BTreeMap::new()));
                }
            }
            current = path;
            p.skip_line();
            continue;
        }
        let parsed = p.dotted().and_then(|key| {
            p.ws(false);
            (p.peek()? == b'=').then_some(())?;
            p.i += 1;
            Some((key, p.value()?))
        });
        match parsed {
            Some((key, v)) => insert(table_at(&mut root, &current), &key, v),
            None => p.skip_line(),
        }
    }
    Value::Table(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_theme_and_keymap_shapes() {
        let doc = parse(
            r##"
# comment
[app]
overall = { fg = "#b4c7ed", bg = "#0d0e12" } # trailing
[mgr]
marker_symbol = "│"
[which]
cols = 3
[confirm]
btn_labels = [ "  [Y]es  ", "  (N)o  " ]
[icon]
conds = [
	# Special files
	{ if = "orphan", text = ""},
	{ if = "dir & hovered", text = "", fg = "#b4c7ed"},
]
[[mgr.prepend_keymap]]
on = [ "M" ]
run = "plugin mount"
desc = "Mount manager"
[[mgr.prepend_keymap]]
on = [ "<C-n>" ]
run = 'shell -- dragon-drop -x -i -T %s'
"##,
        );
        assert_eq!(
            doc.path("app.overall.bg").and_then(Value::str),
            Some("#0d0e12")
        );
        assert_eq!(
            doc.path("mgr.marker_symbol").and_then(Value::str),
            Some("│")
        );
        assert_eq!(doc.path("which.cols"), Some(&Value::Int(3)));
        assert_eq!(doc.path("confirm.btn_labels").unwrap().arr().len(), 2);
        let conds = doc.path("icon.conds").unwrap().arr();
        assert_eq!(conds.len(), 2);
        assert_eq!(
            conds[1].get("if").and_then(Value::str),
            Some("dir & hovered")
        );
        let keys = doc.path("mgr.prepend_keymap").unwrap().arr();
        assert_eq!(keys.len(), 2);
        assert_eq!(
            keys[1].get("run").and_then(Value::str),
            Some("shell -- dragon-drop -x -i -T %s")
        );
        assert_eq!(keys[0].get("on").unwrap().arr()[0].str(), Some("M"));
    }

    #[test]
    fn garbage_is_skipped_not_fatal() {
        let doc = parse("[a\nx = \nfoo = \"unterminated\n[b]\nk = 1\n= 2\n{{{\n");
        assert_eq!(doc.path("b.k"), Some(&Value::Int(1)));
    }
}
