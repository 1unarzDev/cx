//! Bundled, inert source highlighting. No language process execution; the only file
//! read is the user's own yazi syntect theme (`use_theme`), once, when there is one.
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use std::{
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Color as TokenColor, HighlightState, ThemeSet},
    parsing::{ParseState, SyntaxSet},
};

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);
/// The user's yazi `syntect_theme` (a .tmTheme), when Files found one: previews then
/// use its colours and font styles, as yazi does. Backgrounds are never painted.
static USER_THEME: std::sync::OnceLock<syntect::highlighting::Theme> = std::sync::OnceLock::new();
pub fn use_theme(path: &std::path::Path) {
    if USER_THEME.get().is_some() {
        return;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() < 4 << 20) {
        if let Some(theme) = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| tm_theme(&t))
        {
            let _ = USER_THEME.set(theme);
        }
    }
}
/// The parts of a .tmTheme plist previews use: the default foreground and each rule's
/// scope, foreground and fontStyle (a small reader: syntect's plist loader is not built in).
fn tm_theme(text: &str) -> Option<syntect::highlighting::Theme> {
    use std::str::FromStr;
    use syntect::highlighting::{FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem};
    let color = |s: &str| -> Option<TokenColor> {
        let h = s.trim().strip_prefix('#')?;
        let v = u32::from_str_radix(h.get(..6)?, 16).ok()?;
        Some(TokenColor {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
            a: 255,
        })
    };
    // (key, string) pairs in document order.
    let mut pairs = Vec::new();
    let mut rest = text;
    while let Some(k) = rest.find("<key>") {
        let after = &rest[k + 5..];
        let end = after.find("</key>")?;
        let key = &after[..end];
        let tail = &after[end + 6..];
        let value = tail
            .trim_start()
            .strip_prefix("<string>")
            .and_then(|v| v.find("</string>").map(|e| &v[..e]));
        pairs.push((key, value));
        rest = tail;
    }
    let mut theme = Theme::default();
    let mut current: Option<(String, StyleModifier)> = None;
    for (key, value) in pairs {
        match (key, value) {
            ("scope", Some(scope)) => {
                if let Some((s, m)) = current.take() {
                    if let Ok(sel) = ScopeSelectors::from_str(&s) {
                        theme.scopes.push(ThemeItem {
                            scope: sel,
                            style: m,
                        });
                    }
                }
                current = Some((scope.to_owned(), StyleModifier::default()));
            }
            ("foreground", Some(v)) => match current.as_mut() {
                Some((_, m)) => m.foreground = color(v),
                None => theme.settings.foreground = color(v),
            },
            ("fontStyle", Some(v)) => {
                if let Some((_, m)) = current.as_mut() {
                    let mut f = FontStyle::empty();
                    for w in v.split_whitespace() {
                        match w {
                            "bold" => f |= FontStyle::BOLD,
                            "italic" => f |= FontStyle::ITALIC,
                            "underline" => f |= FontStyle::UNDERLINE,
                            _ => {}
                        }
                    }
                    m.font_style = Some(f);
                }
            }
            _ => {}
        }
    }
    if let Some((s, m)) = current.take() {
        if let Ok(sel) = ScopeSelectors::from_str(&s) {
            theme.scopes.push(ThemeItem {
                scope: sel,
                style: m,
            });
        }
    }
    (!theme.scopes.is_empty()).then_some(theme)
}
fn user_style(style: syntect::highlighting::Style) -> Style {
    use syntect::highlighting::FontStyle;
    let c = style.foreground;
    let mut s = Style::default().fg(Color::Rgb(c.r, c.g, c.b));
    if style.font_style.contains(FontStyle::BOLD) {
        s = s.add_modifier(ratatui::style::Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        s = s.add_modifier(ratatui::style::Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        s = s.add_modifier(ratatui::style::Modifier::UNDERLINED);
    }
    s
}
fn palette(color: TokenColor) -> Color {
    // Map trusted grammar theme roles onto the viewer's ANSI palette. Never set background.
    match (color.r, color.g, color.b) {
        (180, 142, 173) => Color::Magenta,
        (163, 190, 140) => Color::Green,
        (191, 97, 106) => Color::Red,
        (235, 203, 139) | (208, 135, 112) => Color::Yellow,
        // (Bright blue: plain ANSI blue is near unreadable on dark terminals.)
        (143, 161, 179) => Color::LightBlue,
        (150, 181, 180) => Color::Cyan,
        (101, 115, 126) => Color::DarkGray,
        _ => Color::Reset,
    }
}
pub fn highlight(text: &str, path: &str) -> Vec<Line<'static>> {
    highlight_with_budget(text, path, Duration::from_millis(500), None).0
}
type State = (HighlightState, ParseState);
/// Parser states where recent streamed chunks ended (path, byte offset, state). Bounded;
/// one stream at a time is the norm, a few keep concurrent callers (tests) apart.
static STREAMED: Mutex<Vec<(String, u64, State)>> = Mutex::new(Vec::new());
/// One streamed chunk covering bytes `start..end` of `path`: it continues the parser
/// state the previous chunk ended with, so a comment or string spanning the boundary
/// stays one construct.
pub fn highlight_chunk(text: &str, path: &str, start: u64, end: u64) -> Vec<Line<'static>> {
    let streamed = || STREAMED.lock().unwrap_or_else(|e| e.into_inner());
    let resume = {
        let mut states = streamed();
        let at = states
            .iter()
            .position(|(p, at, _)| p == path && *at == start);
        at.filter(|_| start > 0).map(|i| states.swap_remove(i).2)
    };
    let (lines, state) = highlight_with_budget(text, path, Duration::from_millis(500), resume);
    if let Some(state) = state {
        let mut states = streamed();
        states.retain(|(p, ..)| p != path);
        if states.len() == 4 {
            states.remove(0);
        }
        states.push((path.to_owned(), end, state));
    }
    lines
}
/// The highlighted lines, and the parser state at the end unless highlighting stopped.
fn highlight_with_budget(
    text: &str,
    path: &str,
    budget: Duration,
    resume: Option<State>,
) -> (Vec<Line<'static>>, Option<State>) {
    let syntaxes = &*SYNTAXES;
    let file = std::path::Path::new(path);
    let syntax = file
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| syntaxes.find_syntax_by_extension(name))
        .or_else(|| {
            file.extension()
                .and_then(|ext| ext.to_str())
                .and_then(|ext| {
                    syntaxes
                        .find_syntax_by_extension(ext)
                        .or_else(|| syntaxes.find_syntax_by_token(ext))
                })
        })
        .or_else(|| syntaxes.find_syntax_by_first_line(text.lines().next().unwrap_or("")))
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());
    let user = USER_THEME.get();
    let theme = user.unwrap_or(&THEMES.themes["base16-ocean.dark"]);
    let mut parser = match resume {
        Some((highlight, parse)) => HighlightLines::from_state(theme, highlight, parse),
        None => HighlightLines::new(syntax, theme),
    };
    // The clock starts after the first line, which pays the one-time lazy
    // regex compilation; a zero budget still means no highlighting at all.
    let mut started: Option<Instant> = None;
    let mut stopped = budget.is_zero();
    let lines = text
        .split_inclusive('\n')
        .map(|line| {
            // Very long lines and exhausted budgets degrade to literal text for the rest.
            // The UI caches this result; rendering never reparses source.
            stopped |= line.len() > 2048 || started.is_some_and(|at| at.elapsed() > budget);
            if stopped {
                return Line::raw(line.trim_end_matches('\n').to_owned());
            }
            let tokens = parser.highlight_line(line, syntaxes);
            started.get_or_insert_with(Instant::now);
            match tokens {
                Ok(tokens) => Line::from(
                    tokens
                        .into_iter()
                        .map(|(style, token)| {
                            Span::styled(
                                token.trim_end_matches('\n').to_owned(),
                                if user.is_some() {
                                    user_style(style)
                                } else {
                                    Style::default().fg(palette(style.foreground))
                                },
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
                Err(_) => {
                    stopped = true;
                    Line::raw(line.trim_end_matches('\n').to_owned())
                }
            }
        })
        .collect();
    (lines, (!stopped).then(|| parser.state()))
}

#[derive(Debug)]
pub struct FencedBlock {
    pub opening: usize,
    pub closing: Option<usize>,
    pub language: String,
}
/// Identify matching fences once; an unclosed fence owns the remaining source.
pub fn fenced_blocks(text: &str) -> Vec<FencedBlock> {
    let source = text.lines().collect::<Vec<_>>();
    let mut result = Vec::new();
    let mut at = 0;
    while at < source.len() {
        let line = source[at].trim_start();
        let marker = line.chars().next().unwrap_or(' ');
        let count = line.chars().take_while(|c| *c == marker).count();
        if !matches!(marker, '`' | '~') || count < 3 {
            at += 1;
            continue;
        }
        let language = line[count..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .chars()
            .take(64)
            .collect();
        let opening = at;
        at += 1;
        while at < source.len() {
            let close = source[at].trim_start();
            let close_count = close.chars().take_while(|c| *c == marker).count();
            if close_count >= count && close[close_count..].trim().is_empty() {
                break;
            }
            at += 1;
        }
        result.push(FencedBlock {
            opening,
            closing: (at < source.len()).then_some(at),
            language,
        });
        at += 1;
    }
    result
}
fn fence_language(language: &str) -> String {
    let language = language.to_lowercase();
    let token = match language.as_str() {
        "python" | "python3" => "py",
        "javascript" | "node" | "nodejs" => "js",
        "typescript" => "ts",
        "shell" | "bash" | "zsh" | "sh" => "sh",
        "c++" => "cpp",
        "c#" | "csharp" => "cs",
        "yml" => "yaml",
        "text" | "plaintext" | "console" | "" => "txt",
        other => other,
    };
    if SYNTAXES
        .find_syntax_by_extension(token)
        .or_else(|| SYNTAXES.find_syntax_by_token(token))
        .is_some()
    {
        token.to_owned()
    } else {
        "txt".into()
    }
}
/// Per-line overrides for fenced code; unknown languages remain literal.
pub fn fenced_lines(text: &str) -> Vec<Option<Line<'static>>> {
    let source = text.lines().collect::<Vec<_>>();
    let mut result = vec![None; source.len()];
    let started = Instant::now();
    for block in fenced_blocks(text) {
        let start = block.opening + 1;
        let end = block.closing.unwrap_or(source.len());
        let body = source[start..end].join("\n");
        let budget = Duration::from_millis(1000)
            .saturating_sub(started.elapsed())
            .min(Duration::from_millis(500));
        let path = format!("fence.{}", fence_language(&block.language));
        for (index, line) in highlight_with_budget(&body, &path, budget, None)
            .0
            .into_iter()
            .enumerate()
        {
            if start + index < end {
                result[start + index] = Some(line);
            }
        }
        // Preserve empty final lines too: syntax conversion must not swallow rows.
        for index in start..end {
            if result[index].is_none() {
                result[index] = Some(Line::raw(source[index].to_owned()));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn languages_color_tokens_without_changing_source_or_background() {
        for (path, source) in [
            ("a.rs", "fn main() { let n = 42; let s = \"hello\"; }\n"),
            ("a.py", "def greet():\n    return \"hello\" # comment\n"),
            ("a.json", "{\"hello\": 42}\n"),
            ("a.cpp", "int main() { return 42; }\n"),
            ("a.sh", "echo \"hello\" # comment\n"),
        ] {
            let lines = highlight(source, path);
            let spans = lines
                .iter()
                .flat_map(|line| line.spans.iter())
                .collect::<Vec<_>>();
            assert!(
                spans
                    .iter()
                    .any(|span| span.style.fg.is_some_and(|color| color != Color::Reset)),
                "{path}"
            );
            assert!(spans.iter().all(|span| span.style.bg.is_none()));
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line
                        .spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n"),
                source.trim_end_matches('\n')
            );
        }
    }
    #[test]
    fn fish_typescript_toml_and_docker_use_bundled_grammars() {
        for (path, source) in [
            ("a.fish", "set name \"hello\"\n"),
            ("a.ts", "const name: string = \"hello\";\n"),
            ("a.toml", "name = \"hello\"\n"),
            ("Dockerfile", "FROM ubuntu:24.04\n"),
        ] {
            let lines = highlight(source, path);
            assert!(
                lines
                    .iter()
                    .flat_map(|line| line.spans.iter())
                    .any(|span| span.style.fg.is_some_and(|color| color != Color::Reset)),
                "{path}"
            );
        }
    }
    #[test]
    fn streamed_chunks_continue_a_comment_across_the_boundary() {
        let first = "int a = 1;\n/* opened here\n";
        let second = "int still_comment = 2;\nend */\nint after = 3;\n";
        let path = "chunk-continuation-test.c";
        let whole = highlight(&format!("{first}{second}"), path);
        let end = first.len() as u64;
        assert_eq!(highlight_chunk(first, path, 0, end), whole[..2]);
        let continued = highlight_chunk(second, path, end, end + second.len() as u64);
        assert_eq!(
            continued,
            whole[2..],
            "the comment continues into the next chunk"
        );
        // A chunk that does not start where the last one ended starts fresh.
        assert_eq!(highlight_chunk(second, path, 1, 2), highlight(second, path));
    }
    #[test]
    fn exhausted_budget_retains_literal_source_without_styles() {
        let source = "let value = \"hello\";\nlet next = 2;\n";
        let lines = highlight_with_budget(source, "sample.rs", Duration::ZERO, None).0;
        assert_eq!(lines.len(), 2);
        assert!(lines
            .iter()
            .flat_map(|line| &line.spans)
            .all(|span| span.style == Style::default()));
        assert_eq!(lines[0].spans[0].content, "let value = \"hello\";");
    }
    #[test]
    fn first_line_compile_cost_does_not_spend_the_budget() {
        // The first line pays syntect's one-time lazy regex compilation (slow in
        // debug builds and on loaded hosts); only later lines draw on the budget.
        let source = "let value = \"hello\";\nlet next = 2;\n";
        let lines = highlight_with_budget(source, "sample.rs", Duration::from_nanos(1), None).0;
        assert!(lines[0].spans.iter().any(|s| s.style != Style::default()));
    }
    #[test]
    fn multiline_source_retains_token_colors_beyond_first_line() {
        let source = (0..20)
            .map(|i| format!("let item_{i} = \"hello\"; // inert source\n"))
            .collect::<String>();
        // Test parser continuity independently of permitted wall-clock fallback under load.
        let lines = highlight_with_budget(&source, "sample.rs", Duration::MAX, None).0;
        assert_eq!(lines.len(), 20);
        assert!(lines
            .last()
            .unwrap()
            .spans
            .iter()
            .any(|span| span.style.fg == Some(Color::Green)));
    }
    #[test]
    fn fence_aliases_mismatched_markers_and_unknown_languages() {
        let source = "~~~python3\nreturn \"hello\"\n```\n~~~\n````javascript\nconst name = \"world\";\n```\n````\n```not-a-language\nreturn \"literal\"\n```";
        let blocks = fenced_blocks(source);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].closing, Some(3));
        assert_eq!(blocks[1].closing, Some(7));
        let lines = fenced_lines(source);
        for row in [1, 5] {
            assert!(lines[row]
                .as_ref()
                .unwrap()
                .spans
                .iter()
                .any(|s| s.style.fg == Some(Color::Green)));
        }
        assert!(lines[9]
            .as_ref()
            .unwrap()
            .spans
            .iter()
            .all(|s| s.style.fg.is_none() || s.style.fg == Some(Color::Reset)));
        assert_eq!(fenced_blocks("```rust\nlet x = 1;\n")[0].closing, None);
    }

    #[test]
    fn fenced_markdown_languages_preserve_prose_and_source() {
        let source = "# Heading\n```rust\nlet name = \"hello\";\n```\nProse\n~~~python\nreturn \"world\"\n~~~";
        let result = fenced_lines(source);
        assert_eq!(result.len(), 8);
        for index in [0, 1, 3, 4, 5, 7] {
            assert!(result[index].is_none());
        }
        for index in [2, 6] {
            let line = result[index].as_ref().unwrap();
            assert!(line
                .spans
                .iter()
                .any(|span| span.style.fg == Some(Color::Green)));
            assert_eq!(
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>(),
                source.lines().nth(index).unwrap()
            );
        }
    }
    #[test]
    fn unknown_and_long_lines_are_literal_and_bounded() {
        let source = "x".repeat(32000);
        let lines = highlight(&source, "a.unknown");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, source);
        assert!(lines[0].spans[0].style.bg.is_none());
    }
}
