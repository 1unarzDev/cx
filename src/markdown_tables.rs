//! Inert, bounded table presentation; one logical output line per source line.
use ratatui::{
    style::Modifier,
    text::{Line, Span},
};
use std::collections::{HashMap, VecDeque};

const MAX_COLUMNS: usize = 32;
const MAX_ROW_BYTES: usize = 8192;
const MAX_ROWS: usize = 256;
const MAX_TABLE_BYTES: usize = 262_144;
#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

/// Preserve code-span pipes as literal data. Backtick runs match exact lengths;
/// unmatched runs remain literal. Work is linear in the bounded row length.
fn code_ranges(s: &str) -> Vec<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut runs = Vec::new();
    let mut by_length: HashMap<usize, VecDeque<usize>> = HashMap::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && bytes[at] == b'`' {
            at += 1;
        }
        by_length
            .entry(at - start)
            .or_default()
            .push_back(runs.len());
        let mut escape_at = start;
        while escape_at > 0 && bytes[escape_at - 1] == b'\\' {
            escape_at -= 1;
        }
        runs.push((start, at, (start - escape_at) % 2 == 1));
    }
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < runs.len() {
        let (start, end, escaped) = runs[index];
        let queue = by_length.get_mut(&(end - start)).unwrap();
        while queue.front().is_some_and(|v| *v <= index) {
            queue.pop_front();
        }
        if escaped {
            index += 1;
            continue;
        }
        if let Some(close) = queue.pop_front() {
            ranges.push((start, runs[close].1));
            index = close + 1;
        } else {
            index += 1;
        }
    }
    ranges
}
fn cells(line: &str) -> Option<Vec<String>> {
    if line.len() > MAX_ROW_BYTES {
        return None;
    }
    let s = line.trim();
    let ranges = code_ranges(s);
    let mut range_index = 0;
    let mut result = Vec::new();
    let mut current = String::new();
    let mut slashes = 0;
    let mut delimiters = 0;
    let mut last_delimiter = false;
    for (at, c) in s.char_indices() {
        while range_index < ranges.len() && at >= ranges[range_index].1 {
            range_index += 1;
        }
        let in_code = ranges
            .get(range_index)
            .is_some_and(|(start, end)| at >= *start && at < *end);
        if c == '|' && !in_code {
            if slashes % 2 == 1 {
                current.pop(); // Markdown escaped pipe: retain the pipe itself.
                current.push('|');
                last_delimiter = false;
            } else {
                result.push(current.trim().to_owned());
                current.clear();
                delimiters += 1;
                last_delimiter = true;
                if result.len() > MAX_COLUMNS + 1 {
                    return None;
                }
            }
        } else {
            current.push(c);
            last_delimiter = false;
        }
        slashes = if c == '\\' { slashes + 1 } else { 0 };
    }
    if delimiters == 0 {
        return None;
    }
    result.push(current.trim().to_owned());
    if s.starts_with('|') {
        result.remove(0);
    }
    if last_delimiter {
        result.pop();
    }
    if result.is_empty() || result.len() > MAX_COLUMNS {
        return None;
    }
    Some(result)
}
fn alignment(s: &str) -> Option<Align> {
    let left = s.starts_with(':');
    let right = s.ends_with(':');
    let middle = s.strip_prefix(':').unwrap_or(s);
    let middle = middle.strip_suffix(':').unwrap_or(middle);
    if middle.len() < 3 || !middle.bytes().all(|b| b == b'-') {
        return None;
    }
    Some(match (left, right) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    })
}
fn cell_width(s: &str, inline: fn(&str) -> Vec<Span<'static>>) -> usize {
    Line::from(inline(s)).width()
}
fn row_line(
    row: &[String],
    widths: &[usize],
    aligns: &[Align],
    header: bool,
    ascii: bool,
    inline: fn(&str) -> Vec<Span<'static>>,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, text) in row.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(if ascii { " | " } else { " │ " }));
        }
        let padding = widths[i].saturating_sub(cell_width(text, inline));
        let left = match aligns[i] {
            Align::Left => 0,
            Align::Right => padding,
            Align::Center => padding / 2,
        };
        spans.push(Span::raw(" ".repeat(left)));
        let mut content = inline(text);
        if header {
            for span in &mut content {
                span.style = span.style.add_modifier(Modifier::BOLD);
            }
        }
        spans.extend(content);
        spans.push(Span::raw(" ".repeat(padding - left)));
    }
    Line::from(spans)
}

/// Detect a header/separator/body table at `start`. Returns consumed source lines
/// and equally many logical `Line`s. The caller must wrap lines without truncation.
/// Malformed or over-budget rows remain ordinary Markdown outside this renderer.
#[cfg(test)]
fn render(
    source: &[&str],
    start: usize,
    width: usize,
    ascii: bool,
) -> Option<(usize, Vec<Line<'static>>)> {
    render_inline(source, start, width, ascii, |text| {
        vec![Span::raw(text.to_owned())]
    })
}
pub fn render_inline(
    source: &[&str],
    start: usize,
    width: usize,
    ascii: bool,
    inline: fn(&str) -> Vec<Span<'static>>,
) -> Option<(usize, Vec<Line<'static>>)> {
    let header_line = *source.get(start)?;
    let separator_line = *source.get(start.checked_add(1)?)?;
    let header = cells(header_line)?;
    let separator = cells(separator_line)?;
    if header.len() != separator.len() {
        return None;
    }
    let aligns = separator
        .iter()
        .map(|v| alignment(v))
        .collect::<Option<Vec<_>>>()?;
    let mut bytes = header_line.len() + separator_line.len();
    let mut rows = vec![header];
    let mut consumed = 2;
    while consumed < MAX_ROWS {
        let Some(line) = source.get(start + consumed) else {
            break;
        };
        if bytes.saturating_add(line.len()) > MAX_TABLE_BYTES {
            break;
        }
        let Some(row) = cells(line) else {
            break;
        };
        if row.len() != rows[0].len() {
            break;
        }
        bytes += line.len();
        rows.push(row);
        consumed += 1;
    }
    let mut widths = vec![0; rows[0].len()];
    for row in &rows {
        for (i, text) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell_width(text, inline));
        }
    }
    let wide = widths.iter().sum::<usize>() + widths.len().saturating_sub(1) * 3 <= width.min(4096);
    let mut lines = Vec::with_capacity(consumed);
    if wide {
        lines.push(row_line(&rows[0], &widths, &aligns, true, ascii, inline));
        lines.push(Line::raw(
            widths
                .iter()
                .map(|w| {
                    if ascii {
                        "-".repeat(*w)
                    } else {
                        "─".repeat(*w)
                    }
                })
                .collect::<Vec<_>>()
                .join(if ascii { "-+-" } else { "─┼─" }),
        ));
        lines.extend(
            rows.iter()
                .skip(1)
                .map(|row| row_line(row, &widths, &aligns, false, ascii, inline)),
        );
    } else {
        let mut header = Vec::new();
        for (i, cell) in rows[0].iter().enumerate() {
            if i > 0 {
                header.push(Span::raw(" | "));
            }
            header.extend(inline(cell).into_iter().map(|mut span| {
                span.style = span.style.add_modifier(Modifier::BOLD);
                span
            }));
        }
        lines.push(Line::from(header));
        // Keep a source-index placeholder for the separator without consuming
        // scarce narrow-screen space with a long synthetic rule.
        lines.push(Line::raw(if ascii { "---" } else { "───" }));
        for row in rows.iter().skip(1) {
            let mut spans = Vec::new();
            for (i, text) in row.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw("; "));
                }
                spans.extend(inline(&rows[0][i]).into_iter().map(|mut span| {
                    span.style = span.style.add_modifier(Modifier::BOLD);
                    span
                }));
                spans.push(Span::raw(": "));
                spans.extend(inline(text));
            }
            lines.push(Line::from(spans));
        }
    }
    Some((consumed, lines))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }
    #[test]
    fn wide_unicode_alignment_and_source_indices() {
        let source = [
            "before",
            "| Item | Count | Place |",
            "| :--- | ---: | :---: |",
            "| 日本 | 2 | é |",
            "| abc | 123 | town |",
            "after",
        ];
        let (count, lines) = render(&source, 1, 80, false).unwrap();
        assert_eq!(count, 4);
        assert_eq!(lines.len(), count);
        assert_eq!(text(&lines[2]), "日本 │     2 │   é  ");
        assert_eq!(lines[0].width(), lines[2].width());
        assert!(lines[0]
            .spans
            .iter()
            .any(|v| v.style.add_modifier.contains(Modifier::BOLD)));
    }
    #[test]
    fn narrow_labels_keep_all_cells_and_logical_lines() {
        let source = [
            "| City | Memo |",
            "| --- | --- |",
            "| 東京 | recording with many words |",
        ];
        let (count, lines) = render(&source, 0, 12, true).unwrap();
        assert_eq!(count, 3);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            text(&lines[2]),
            "City: 東京; Memo: recording with many words"
        );
    }
    #[test]
    fn escaped_and_inline_code_pipes_are_literal() {
        let source = [
            "| A | B |",
            "| --- | --- |",
            r"| a\|b | `x|y` |",
            r"| even\\ | ``x`|y`` |",
        ];
        let (count, lines) = render(&source, 0, 100, true).unwrap();
        assert_eq!(count, 4);
        assert!(text(&lines[2]).contains("a|b"));
        assert!(text(&lines[2]).contains("`x|y`"));
        assert!(text(&lines[3]).contains(r"even\\"));
        assert!(text(&lines[3]).contains("``x`|y``"));
        assert_eq!(
            cells("a `unmatched | b").unwrap(),
            vec!["a `unmatched", "b"]
        );
        assert_eq!(cells(r"a \`text | b`").unwrap(), vec![r"a \`text", "b`"]);
    }
    #[test]
    fn mismatch_and_non_tables_stay_unconsumed() {
        assert!(render(&["a | b", "--- | --"], 0, 80, false).is_none());
        assert!(render(&["a | b", "::--- | ---"], 0, 80, false).is_none());
        assert!(render(&["a | b", "---"], 0, 80, false).is_none());
        assert!(render(&["heading", "---"], 0, 80, false).is_none());
        let (count, lines) =
            render(&["a | b", "--- | ---", "1 | 2 | extra"], 0, 80, false).unwrap();
        assert_eq!(count, 2);
        assert_eq!(lines.len(), 2);
        assert!(render(&["| a |", "| --- |"], usize::MAX, 80, false).is_none());
    }
    #[test]
    fn combining_and_emoji_terminal_cells() {
        assert_eq!(cell_width("e\u{301}", |s| vec![Span::raw(s.to_owned())]), 1);
        assert_eq!(cell_width("💻", |s| vec![Span::raw(s.to_owned())]), 2);
        let (_, lines) = render(&["a | b", "--- | ---", "e\u{301} | 💻"], 0, 6, true).unwrap();
        assert_eq!(text(&lines[2]), "e\u{301} | 💻");
        assert_eq!(lines[2].width(), 6);
    }
    #[test]
    fn bounds_and_empty_cells() {
        let long = "x".repeat(MAX_ROW_BYTES + 1);
        assert!(cells(&long).is_none());
        let many = vec!["x"; MAX_COLUMNS + 1].join("|");
        assert!(cells(&many).is_none());
        assert_eq!(cells("| | x | |").unwrap(), vec!["", "x", ""]);
        let mut source = vec!["a | b", "--- | ---"];
        source.extend(std::iter::repeat_n("1 | 2", MAX_ROWS));
        let (count, lines) = render(&source, 0, 80, false).unwrap();
        assert_eq!(count, MAX_ROWS);
        assert_eq!(lines.len(), count);
    }
}
