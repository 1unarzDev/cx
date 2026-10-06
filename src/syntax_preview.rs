//! Bundled, inert source highlighting. No file access or language process execution.
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Color as TokenColor, ThemeSet},
    parsing::SyntaxSet,
};

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);
fn palette(color: TokenColor) -> Color {
    // Map trusted grammar theme roles onto the viewer's ANSI palette. Never set background.
    match (color.r, color.g, color.b) {
        (180, 142, 173) => Color::Magenta,
        (163, 190, 140) => Color::Green,
        (191, 97, 106) => Color::Red,
        (235, 203, 139) | (208, 135, 112) => Color::Yellow,
        (143, 161, 179) => Color::Blue,
        (150, 181, 180) => Color::Cyan,
        (101, 115, 126) => Color::DarkGray,
        _ => Color::Reset,
    }
}
pub fn highlight(text: &str, path: &str) -> Vec<Line<'static>> {
    let syntaxes = &*SYNTAXES;
    let file = std::path::Path::new(path);
    let syntax = file
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| syntaxes.find_syntax_by_extension(name))
        .or_else(|| {
            file.extension()
                .and_then(|ext| ext.to_str())
                .and_then(|ext| syntaxes.find_syntax_by_extension(ext))
        })
        .or_else(|| syntaxes.find_syntax_by_first_line(text.lines().next().unwrap_or("")))
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());
    let theme = &THEMES.themes["base16-ocean.dark"];
    let mut parser = HighlightLines::new(syntax, theme);
    let started = Instant::now();
    let mut stopped = false;
    text.split_inclusive('\n')
        .map(|line| {
            // Very long lines and exhausted budgets degrade to literal text for the rest.
            // The UI caches this result; rendering never reparses source.
            stopped |= line.len() > 2048 || started.elapsed() > Duration::from_millis(30);
            if stopped {
                return Line::raw(line.trim_end_matches('\n').to_owned());
            }
            match parser.highlight_line(line, syntaxes) {
                Ok(tokens) => Line::from(
                    tokens
                        .into_iter()
                        .map(|(style, token)| {
                            Span::styled(
                                token.trim_end_matches('\n').to_owned(),
                                Style::default().fg(palette(style.foreground)),
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
        .collect()
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
    fn unknown_and_long_lines_are_literal_and_bounded() {
        let source = "x".repeat(32000);
        let lines = highlight(&source, "a.unknown");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, source);
        assert!(lines[0].spans[0].style.bg.is_none());
    }
}
