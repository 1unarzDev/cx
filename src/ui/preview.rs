//! File preview: open/page/find state and rendering of text, markdown layout/links/emphasis and raster fallback.
use super::*;

pub(super) fn markdown_heading_slug(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter_map(|c| {
            if c.is_whitespace() {
                Some('-')
            } else if c.is_alphanumeric() || c == '_' || c == '-' {
                Some(c)
            } else {
                None
            }
        })
        .collect()
}
pub(super) fn markdown_anchor_scroll(browser: &Browser, anchor: &str) -> Option<u32> {
    let slug = markdown_heading_slug(anchor);
    with_doc(browser, |doc| {
        let row = *browser.markdown_anchor_rows.borrow().get(&slug)?;
        Some(doc.row_of(row) as u32)
    })
    .flatten()
}
pub(super) fn markdown_preview_links(source: &str) -> Vec<crate::markdown_links::Link> {
    let safe = safe_text(source);
    let fences = crate::syntax_preview::fenced_blocks(&safe);
    let mut links = Vec::new();
    for (index, line) in source.lines().enumerate() {
        if fences
            .iter()
            .any(|f| index >= f.opening && index <= f.closing.unwrap_or(usize::MAX))
        {
            continue;
        }
        links.extend(crate::markdown_links::links(line));
        if links.len() >= 128 {
            links.truncate(128);
            break;
        }
    }
    links
}
pub(super) fn markdown_link_tag(target: &str) -> Color {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(target.as_bytes());
    Color::Rgb(hash[0], hash[1], hash[2])
}
pub(super) fn preview_inline_tagged(text: &str, tagged: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut at = 0;
    for link in crate::markdown_links::links(text) {
        spans.extend(preview_emphasis(&text[at..link.start]));
        let mut style = info().add_modifier(Modifier::UNDERLINED);
        // Tags exist only in a private hit-test layout, never the visible frame.
        if tagged {
            style.bg = Some(markdown_link_tag(&link.target));
        }
        for mut span in preview_emphasis(&link.label) {
            span.style = span.style.patch(style);
            spans.push(span);
        }
        at = link.end;
    }
    spans.extend(preview_emphasis(&text[at..]));
    spans
}
// Wrap the body using Ratatui's own Unicode/word layout, then restore the
// list prefix. Continuation rows have the exact same content starting column.
pub(super) fn hanging_lines(line: &Line<'static>, prefix: usize, width: u16) -> Vec<Line<'static>> {
    if prefix >= usize::from(width) || prefix == 0 {
        return vec![line.clone()];
    }
    let (leading, body) = split_prefix(line, prefix);
    let body_width = width - prefix as u16;
    // Most list rows fit: the same row without a render pass (the wrap below costs a
    // buffer per line, most of a 1 MiB document's layout).
    let fits = body.iter().map(Span::width).sum::<usize>() <= usize::from(body_width);
    if fits
        && !body
            .first()
            .is_some_and(|s| s.content.starts_with(char::is_whitespace))
    {
        let mut spans = leading;
        spans.extend(body);
        if let Some(last) = spans.last_mut() {
            last.content = last.content.trim_end().to_owned().into();
        }
        return vec![Line::from(spans)];
    }
    wrap_hanging(line, leading, body, prefix, body_width)
}
/// The first `prefix` columns' spans and the rest.
fn split_prefix(line: &Line<'static>, prefix: usize) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let mut leading = Vec::new();
    let mut body = Vec::new();
    let mut remaining = prefix;
    for span in &line.spans {
        let take = remaining.min(span.content.chars().count());
        let split = span
            .content
            .char_indices()
            .nth(take)
            .map(|(i, _)| i)
            .unwrap_or(span.content.len());
        if split > 0 {
            leading.push(Span::styled(span.content[..split].to_owned(), span.style));
        }
        if split < span.content.len() {
            body.push(Span::styled(span.content[split..].to_owned(), span.style));
        }
        remaining -= take;
    }
    (leading, body)
}
/// `hanging_lines` for a body wider than its column: ratatui's word wrap, read back.
fn wrap_hanging(
    line: &Line<'static>,
    leading: Vec<Span<'static>>,
    body: Vec<Span<'static>>,
    prefix: usize,
    body_width: u16,
) -> Vec<Line<'static>> {
    let paragraph = Paragraph::new(Line::from(body)).wrap(Wrap { trim: true });
    let height = paragraph.line_count(body_width).max(1);
    if height > 8192 {
        return vec![line.clone()];
    }
    let rect = Rect::new(0, 0, body_width, height as u16);
    let mut buffer = ratatui::buffer::Buffer::empty(rect);
    ratatui::widgets::Widget::render(paragraph, rect, &mut buffer);
    (0..height as u16)
        .map(|y| {
            let mut spans = if y == 0 {
                leading.clone()
            } else {
                vec![Span::raw(" ".repeat(prefix))]
            };
            let mut x = 0;
            while x < body_width {
                let cell = &buffer[(x, y)];
                let symbol = cell.symbol();
                let step = Span::raw(symbol).width().max(1) as u16;
                if let Some(last) = spans.last_mut().filter(|s| s.style == cell.style()) {
                    last.content.to_mut().push_str(symbol);
                } else {
                    spans.push(Span::styled(symbol.to_owned(), cell.style()));
                }
                x = x.saturating_add(step);
            }
            if let Some(last) = spans.last_mut() {
                last.content = last.content.trim_end().to_owned().into();
            }
            Line::from(spans)
        })
        .collect()
}
pub(super) fn list_prefix(source: &str) -> Option<usize> {
    let trimmed = source.trim_start_matches(' ');
    let indent = source.len() - trimmed.len();
    if ["- ", "* ", "+ "].iter().any(|p| trimmed.starts_with(p)) {
        return Some(indent + 2);
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits)
        && (trimmed[digits..].starts_with(". ") || trimmed[digits..].starts_with(") "))
    {
        return Some(indent + digits + 2);
    }
    None
}
pub(super) fn preview_emphasis(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut at = 0;
    let mut plain = 0;
    let mut slashes = 0;
    while at < text.len() {
        let rest = &text[at..];
        if rest.starts_with('`') {
            let run = rest.bytes().take_while(|b| *b == b'`').count();
            if let Some(end) = rest[run..].find(&"`".repeat(run)) {
                at += run + end + run;
                slashes = 0;
                continue;
            }
            break;
        }
        let escaped = slashes % 2 == 1;
        let delimiter = if !escaped && rest.starts_with("$$") {
            Some(("$$", "$$"))
        } else if !escaped && rest.starts_with('$') {
            Some(("$", "$"))
        } else if !escaped && rest.starts_with("\\(") {
            Some(("\\(", "\\)"))
        } else if !escaped && rest.starts_with("\\[") {
            Some(("\\[", "\\]"))
        } else {
            None
        };
        if let Some((open, close)) = delimiter {
            let after = &rest[open.len()..];
            if let Some(end) = after.match_indices(close).find_map(|(end, _)| {
                (after[..end]
                    .bytes()
                    .rev()
                    .take_while(|b| *b == b'\\')
                    .count()
                    % 2
                    == 0)
                    .then_some(end)
            }) {
                let expression = &after[..end];
                if end == 0
                    || (open == "$"
                        && (expression.starts_with(char::is_whitespace)
                            || expression.ends_with(char::is_whitespace)
                            || after[end + close.len()..]
                                .starts_with(|c: char| c.is_ascii_digit())))
                {
                    at += open.len();
                    slashes = 0;
                    continue;
                }
                if !ascii() {
                    if let Some(rendered) = crate::markdown_math::render_math(expression) {
                        spans.extend(preview_emphasis_only(&text[plain..at]));
                        spans.push(Span::styled(rendered, tok(Tok::Code)));
                        at += open.len() + end + close.len();
                        plain = at;
                        slashes = 0;
                        continue;
                    }
                }
                // Unsupported math remains literal, including delimiters and scripts.
                spans.extend(preview_emphasis_only(&text[plain..at]));
                let length = open.len() + end + close.len();
                spans.push(Span::raw(rest[..length].to_owned()));
                at += length;
                plain = at;
                slashes = 0;
                continue;
            }
        }
        let ch = text[at..].chars().next().unwrap();
        slashes = if ch == '\\' { slashes + 1 } else { 0 };
        at += ch.len_utf8();
    }
    spans.extend(preview_emphasis_only(&text[plain..]));
    spans
}
pub(super) fn preview_emphasis_only(text: &str) -> Vec<Span<'static>> {
    // A deliberately small prose renderer: no HTML execution, links or image fetches.
    let mut spans = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let offset = text.len() - rest.len();
        let found = rest.char_indices().find(|(at, c)| {
            if *c != '_' {
                return *c == '`' || *c == '*';
            }
            let absolute = offset + at;
            let previous = text[..absolute].chars().next_back();
            let escaped = text[..absolute]
                .chars()
                .rev()
                .take_while(|c| *c == '\\')
                .count()
                % 2
                == 1;
            let length = if rest[*at..].starts_with("__") { 2 } else { 1 };
            !escaped
                && !previous.is_some_and(|c| c.is_alphanumeric() || c == '_')
                && rest[*at + length..]
                    .chars()
                    .next()
                    .is_some_and(|c| !c.is_whitespace() && c != '_')
        });
        let Some((at, marker)) = found else {
            spans.push(Span::raw(rest.to_owned()));
            break;
        };
        if at > 0 {
            spans.push(Span::raw(rest[..at].to_owned()));
        }
        let token = if marker == '*' && rest[at..].starts_with("**") {
            "**"
        } else if marker == '*' {
            "*"
        } else if marker == '_' && rest[at..].starts_with("__") {
            "__"
        } else if marker == '_' {
            "_"
        } else {
            "`"
        };
        let after = &rest[at + token.len()..];
        let end = if marker == '_' {
            after.match_indices(token).find_map(|(end, _)| {
                let before = after[..end].chars().next_back();
                let next = after[end + token.len()..].chars().next();
                let escaped = after[..end]
                    .chars()
                    .rev()
                    .take_while(|c| *c == '\\')
                    .count()
                    % 2
                    == 1;
                (!escaped
                    && before.is_some_and(|c| !c.is_whitespace() && c != '_')
                    && !next.is_some_and(|c| c.is_alphanumeric() || c == '_'))
                .then_some(end)
            })
        } else {
            after.find(token)
        };
        if let Some(end) = end {
            let style = match token {
                "`" => tok(Tok::Code),
                "**" | "__" => Style::default().add_modifier(Modifier::BOLD),
                _ => Style::default().add_modifier(Modifier::ITALIC),
            };
            spans.push(Span::styled(after[..end].to_owned(), style));
            rest = &after[end + token.len()..];
        } else if marker == '_' {
            spans.push(Span::raw(token.to_owned()));
            rest = after;
        } else {
            spans.push(Span::raw(rest[at..].to_owned()));
            break;
        }
    }
    spans
}
pub(super) fn preview_lines(text: &str, kind: &str) -> Vec<Line<'static>> {
    preview_lines_tagged(text, kind, false)
}
pub(super) fn preview_lines_tagged(text: &str, kind: &str, tagged: bool) -> Vec<Line<'static>> {
    let safe = safe_text(text);
    let code = if kind == "markdown" {
        crate::syntax_preview::fenced_lines(&safe)
    } else {
        Vec::new()
    };
    let fences = if kind == "markdown" {
        crate::syntax_preview::fenced_blocks(&safe)
    } else {
        Vec::new()
    };
    let openings: HashMap<_, _> = fences.iter().map(|f| (f.opening, f)).collect();
    let closings: BTreeSet<_> = fences.iter().filter_map(|f| f.closing).collect();
    safe.lines()
        .enumerate()
        .map(|(index, line)| {
            if kind == "markdown" {
                if let Some(Some(highlighted)) = code.get(index) {
                    let mut spans = vec![Span::styled(if ascii() { "| " } else { "│ " }, muted())];
                    spans.extend(highlighted.spans.clone());
                    return Line::from(spans);
                }
                if let Some(fence) = openings.get(&index) {
                    return Line::from(vec![
                        Span::styled(if ascii() { "+-- " } else { "╭─ " }, muted()),
                        Span::styled(
                            if fence.language.is_empty() {
                                "code".into()
                            } else {
                                fence.language.clone()
                            },
                            accent().add_modifier(Modifier::BOLD),
                        ),
                    ]);
                }
                if closings.contains(&index) {
                    return Line::from(Span::styled(if ascii() { "+--" } else { "╰─" }, muted()));
                }
                let hashes = line.chars().take_while(|c| *c == '#').count();
                if (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ') {
                    let style = accent().add_modifier(Modifier::BOLD);
                    let mut spans = preview_inline_tagged(&line[hashes + 1..], tagged);
                    for span in &mut spans {
                        span.style = style.patch(span.style);
                    }
                    return Line::from(spans);
                }
                if ["- ", "* ", "+ "]
                    .iter()
                    .any(|p| line.trim_start().starts_with(p))
                {
                    let prefix = line.len() - line.trim_start().len();
                    let mut spans = vec![Span::styled(
                        format!("{}{} ", " ".repeat(prefix), if ascii() { "-" } else { "•" }),
                        accent(),
                    )];
                    spans.extend(preview_inline_tagged(&line.trim_start()[2..], tagged));
                    return Line::from(spans);
                }
                return Line::from(preview_inline_tagged(line, tagged));
            }
            if kind == "code" {
                let trimmed = line.trim_start();
                let style = if trimmed.starts_with("//") || trimmed.starts_with('#') {
                    muted()
                } else if [
                    "fn ",
                    "pub ",
                    "let ",
                    "use ",
                    "def ",
                    "class ",
                    "import ",
                    "const ",
                    "function ",
                ]
                .iter()
                .any(|word| trimmed.starts_with(word))
                {
                    accent()
                } else {
                    Style::default()
                };
                return Line::from(Span::styled(line.to_owned(), style));
            }
            Line::raw(line.to_owned())
        })
        .collect()
}
pub(super) fn raster_lines(w: usize, h: usize, bytes: &[u8], area: Rect) -> Vec<Line<'static>> {
    let available_w = usize::from(area.width);
    let available_h = usize::from(area.height) * 2;
    if available_w == 0 || available_h == 0 {
        return vec![];
    }
    let scale = (available_w as f64 / w as f64).min(available_h as f64 / h as f64);
    let width = ((w as f64 * scale).floor() as usize).max(1);
    let height = ((h as f64 * scale).floor() as usize).max(1);
    let pixel = |x: usize, y: usize| -> Option<Color> {
        if y >= height {
            return None;
        }
        let at = ((y * h / height) * w + x * w / width) * 4;
        (bytes[at + 3] >= 128).then_some(Color::Rgb(bytes[at], bytes[at + 1], bytes[at + 2]))
    };
    (0..height.div_ceil(2))
        .map(|row| {
            Line::from(
                (0..width)
                    .map(|x| match (pixel(x, row * 2), pixel(x, row * 2 + 1)) {
                        (Some(top), Some(bottom)) => {
                            Span::styled("▀", Style::default().fg(top).bg(bottom))
                        }
                        (Some(top), None) => Span::styled("▀", Style::default().fg(top)),
                        (None, Some(bottom)) => Span::styled("▄", Style::default().fg(bottom)),
                        (None, None) => Span::raw(" "),
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}
pub(super) fn markdown_layout_lines(
    browser: &Browser,
    text: &str,
    tagged: bool,
) -> Vec<Line<'static>> {
    let width = browser.preview_viewport.get().0;
    let mut lines = if tagged {
        preview_lines_tagged(text, "markdown", true)
    } else {
        browser
            .preview_rich
            .as_ref()
            .and_then(|p| p.styled.clone())
            .unwrap_or_else(|| preview_lines(text, "markdown"))
    };
    let safe = safe_text(text);
    let source: Vec<_> = safe.lines().collect();
    let fences = crate::syntax_preview::fenced_blocks(&safe);
    let mut at = 0;
    let mut protected = BTreeSet::new();
    let mut fence = fences.iter().peekable();
    while at < source.len() {
        if let Some(block) = fence.peek().filter(|block| block.opening == at) {
            let end = block.closing.map(|i| i + 1).unwrap_or(source.len());
            protected.extend(at..end);
            at = end;
            fence.next();
            continue;
        }
        // A table body must never consume an upcoming fenced block.
        let table_end = fence
            .peek()
            .map(|block| block.opening)
            .unwrap_or(source.len());
        if let Some((consumed, rendered)) = crate::markdown_tables::render_inline(
            &source[..table_end],
            at,
            if width == 0 { 80 } else { usize::from(width) },
            ascii(),
            |text| preview_inline_tagged(text, tagged),
        ) {
            if consumed > 0 && rendered.len() == consumed && at + consumed <= lines.len() {
                protected.extend(at..at + consumed);
                lines.splice(at..at + consumed, rendered);
                at += consumed;
                continue;
            }
        }
        at += 1;
    }
    let mut row = 0;
    while row < source.len() {
        let opening = source[row].trim();
        let closing = match opening {
            "$$" => Some("$$"),
            "\\[" => Some("\\]"),
            _ => None,
        };
        if !protected.contains(&row) {
            if let Some(closing) = closing {
                if let Some(end) = (row + 1..source.len())
                    .take(128)
                    .find(|i| protected.contains(i) || source[*i].trim() == closing)
                {
                    if !protected.contains(&end) {
                        for index in row..=end {
                            lines[index] = Line::raw(source[index].to_owned());
                        }
                        protected.extend(row..=end);
                        let expression = source[row + 1..end].join(" ");
                        if let Some(rendered) = (!ascii())
                            .then(|| crate::markdown_math::render_math(&expression))
                            .flatten()
                        {
                            protected.extend(row..=end);
                            lines[row] = Line::from(Span::styled(
                                if ascii() { "Math" } else { "∷ Math" },
                                muted(),
                            ));
                            for line in &mut lines[row + 1..=end] {
                                *line = Line::raw("");
                            }
                            if row + 1 < end {
                                lines[row + 1] = Line::from(Span::styled(rendered, tok(Tok::Code)));
                            } else {
                                lines[row] = Line::from(Span::styled(rendered, tok(Tok::Code)));
                            }
                            row = end + 1;
                            continue;
                        }
                    }
                }
            }
        }
        row += 1;
    }
    let mut wrapped = Vec::new();
    let mut anchors = BTreeMap::new();
    let mut duplicates = HashMap::<String, usize>::new();
    let mut indent = 0;
    for (index, mut line) in lines.into_iter().enumerate() {
        let raw = source.get(index).copied().unwrap_or("");
        let header = raw.trim_start();
        let hashes = header.chars().take_while(|c| *c == '#').count();
        if !protected.contains(&index)
            && (1..=6).contains(&hashes)
            && header.as_bytes().get(hashes) == Some(&b' ')
        {
            let text = line
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>();
            let base = markdown_heading_slug(text.trim_end_matches('#').trim());
            let count = duplicates.entry(base.clone()).or_default();
            let slug = if *count == 0 {
                base.clone()
            } else {
                format!("{base}-{count}")
            };
            anchors.insert(slug, wrapped.len());
            *count += 1;
        }
        if protected.contains(&index)
            || raw.trim().is_empty()
            || raw.trim_start().starts_with(['#', '>'])
        {
            indent = 0;
            wrapped.push(line);
            continue;
        }
        if let Some(prefix) = list_prefix(raw) {
            indent = prefix;
        } else if indent > 0 {
            let spaces = raw.len() - raw.trim_start_matches(' ').len();
            if spaces < indent {
                line.spans.insert(0, Span::raw(" ".repeat(indent - spaces)));
            }
        }
        wrapped.extend(hanging_lines(
            &line,
            indent,
            if width == 0 { 80 } else { width },
        ));
    }
    if !tagged {
        *browser.markdown_anchor_rows.borrow_mut() = anchors;
    }
    wrapped
}
/// Logical display lines for `text`, without caching (used when `text` is not the
/// browser's current preview, e.g. by tests probing an alternative source).
fn uncached_display_lines(browser: &Browser, text: &str) -> Vec<Line<'static>> {
    let rich = browser.preview_rich.as_ref();
    if rich.is_some_and(|p| p.kind == "markdown") {
        let mut lines = markdown_layout_lines(browser, text, false);
        if ascii() || no_color() {
            strip_colors(&mut lines);
        }
        lines
    } else if !ascii() && !no_color() {
        rich.and_then(|p| p.styled.clone())
            .unwrap_or_else(|| preview_lines(text, rich.map(|p| p.kind.as_str()).unwrap_or("text")))
    } else {
        safe_text(text)
            .lines()
            .map(|s| Line::raw(s.to_owned()))
            .collect()
    }
}
fn strip_colors(lines: &mut [Line<'static>]) {
    for line in lines {
        line.style.fg = None;
        line.style.bg = None;
        for span in &mut line.spans {
            span.style.fg = None;
            span.style.bg = None;
        }
    }
}
#[cfg(test)]
pub(super) fn preview_display_lines(browser: &Browser, text: &str) -> Vec<Line<'static>> {
    if browser
        .preview
        .as_deref()
        .is_some_and(|p| std::ptr::eq(p, text))
    {
        if let Some(lines) = with_doc(browser, |doc| doc.lines.clone()) {
            return lines;
        }
    }
    uncached_display_lines(browser, text)
}
pub(super) fn preview_max_scroll(lines: &[Line<'static>], width: u16, height: u16) -> u16 {
    Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
        .saturating_sub(usize::from(height))
        .min(u16::MAX as usize) as u16
}
/// Every contiguous occurrence of `needle` (lowercase chars) in one line, else one
/// fuzzy subsequence; positions are character indices into the line.
pub(super) fn line_matches(chars: impl Iterator<Item = char>, needle: &[char]) -> Vec<Vec<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    let text: Vec<_> = chars
        .enumerate()
        .flat_map(|(i, c)| c.to_lowercase().map(move |c| (c, i)))
        .collect();
    let mut result = Vec::new();
    for (start, window) in text.windows(needle.len()).enumerate() {
        if window.iter().map(|(c, _)| *c).eq(needle.iter().copied()) {
            result.push(
                text[start..start + needle.len()]
                    .iter()
                    .map(|(_, i)| *i)
                    .collect(),
            );
        }
    }
    if result.is_empty() {
        let mut positions = Vec::new();
        let mut next = 0;
        for (c, i) in text {
            if c == needle[next] {
                positions.push(i);
                next += 1;
                if next == needle.len() {
                    break;
                }
            }
        }
        if next == needle.len() {
            result.push(positions);
        }
    }
    result
}
pub(super) fn preview_matches(lines: &[Line<'static>], query: &str) -> Vec<(usize, Vec<usize>)> {
    let needle: Vec<_> = query.chars().flat_map(char::to_lowercase).collect();
    let mut result = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        for positions in line_matches(line.spans.iter().flat_map(|s| s.content.chars()), &needle) {
            result.push((row, positions));
        }
    }
    result
}

/// What a cached preview layout was built from. Pointers identify the exact source
/// and highlighted lines, so replacing either (even without a revision bump) rebuilds.
#[derive(Clone, PartialEq)]
struct DocKey {
    revision: u64,
    text: (usize, usize),
    kind: String,
    styled: (usize, usize),
    plain: bool,
}
fn doc_key(browser: &Browser) -> Option<DocKey> {
    let text = browser.preview.as_deref()?;
    let rich = browser.preview_rich.as_ref();
    Some(DocKey {
        revision: browser.preview_revision,
        text: (text.as_ptr() as usize, text.len()),
        kind: rich.map(|p| p.kind.clone()).unwrap_or_default(),
        styled: rich
            .and_then(|p| p.styled.as_ref())
            .map_or((0, 0), |s| (s.as_ptr() as usize, s.len())),
        plain: ascii() || no_color(),
    })
}
/// Width the preview is laid out for: the last rendered viewport, else 80 columns.
fn layout_width(browser: &Browser) -> u16 {
    match browser.preview_viewport.get().0 {
        0 => 80,
        width => width,
    }
}
fn wrapped_rows(line: &Line<'static>, width: u16) -> usize {
    if line.width() <= usize::from(width) {
        1
    } else {
        Paragraph::new(line.clone())
            .wrap(Wrap { trim: false })
            .line_count(width)
            .max(1)
    }
}

/// Search matches per wrapped line: (line, match columns).
type SearchHits = std::sync::Arc<Vec<(usize, Vec<usize>)>>;
/// Retained preview layout: display lines, the visual row where each starts at the
/// current width, search matches per query and whether link hit-testing is needed.
/// Scrolling renders only the visible slice; nothing here is recomputed per frame.
#[derive(Clone)]
pub(super) struct DocCache {
    key: DocKey,
    width: u16,
    lines: Vec<Line<'static>>,
    /// Tagged markdown layout for link hit-testing (empty when the preview has no links).
    /// The link-tagged layout, built on the first click (not on paint: it is a second
    /// full layout).
    hit_lines: Option<Vec<Line<'static>>>,
    /// `starts[i]` is the first visual row of line `i`; `starts[lines.len()]` is the total.
    starts: Vec<usize>,
    /// Lines of the source text (markdown lays out into a different number of lines).
    source_lines: usize,
    search: Option<(String, SearchHits)>,
    links: bool,
    /// (scroll, area) of the last link hit map, so unchanged frames skip the rebuild.
    link_frame: std::cell::Cell<Option<(u32, Rect)>>,
    /// The frame `Browser::preview_link_cells` was computed for.
    cells_frame: std::cell::Cell<Option<(u32, Rect)>>,
}
impl DocCache {
    fn build(browser: &Browser, key: DocKey, text: &str) -> Self {
        let markdown = key.kind == "markdown";
        let width = layout_width(browser);
        let links = markdown && !markdown_preview_links(text).is_empty();
        let lines = uncached_display_lines(browser, text);
        let mut doc = Self {
            key,
            width,
            lines,
            hit_lines: None,
            starts: Vec::new(),
            source_lines: text.lines().count(),
            search: None,
            links,
            link_frame: std::cell::Cell::new(None),
            cells_frame: std::cell::Cell::new(None),
        };
        doc.measure();
        doc
    }
    fn measure(&mut self) {
        self.starts.clear();
        self.starts.reserve(self.lines.len() + 1);
        let mut row = 0;
        for line in &self.lines {
            self.starts.push(row);
            row += wrapped_rows(line, self.width);
        }
        self.starts.push(row);
    }
    /// Lines appended to a non-markdown preview: extend lines, rows and any search.
    fn append(&mut self, key: DocKey, more: Vec<Line<'static>>) {
        let first = self.lines.len();
        let mut row = self.total_rows();
        self.starts.pop();
        for line in &more {
            self.starts.push(row);
            row += wrapped_rows(line, self.width);
        }
        self.starts.push(row);
        self.source_lines += more.len();
        self.lines.extend(more);
        if let Some((query, matches)) = &mut self.search {
            let added = preview_matches(&self.lines[first..], query);
            std::sync::Arc::make_mut(matches)
                .extend(added.into_iter().map(|(r, p)| (r + first, p)));
        }
        self.key = key;
        self.link_frame.set(None);
    }
    pub(super) fn total_rows(&self) -> usize {
        self.starts.last().copied().unwrap_or(0)
    }
    /// Visual row where logical line `line` starts.
    pub(super) fn row_of(&self, line: usize) -> usize {
        self.starts
            .get(line.min(self.lines.len()))
            .copied()
            .unwrap_or(0)
    }
    pub(super) fn max_scroll(&self, height: u16) -> u32 {
        self.total_rows()
            .saturating_sub(usize::from(height))
            .min(u32::MAX as usize) as u32
    }
    pub(super) fn matches(&mut self, query: &str) -> std::sync::Arc<Vec<(usize, Vec<usize>)>> {
        if let Some((cached, matches)) = &self.search {
            if cached == query {
                return matches.clone();
            }
        }
        let matches = std::sync::Arc::new(preview_matches(&self.lines, query));
        self.search = Some((query.to_owned(), matches.clone()));
        matches
    }
    /// The logical line range covering visual rows `scroll..scroll+height`, and the
    /// rows of the first line that lie above the viewport.
    fn window(&self, scroll: usize, height: usize) -> (usize, usize, usize) {
        let first = self
            .starts
            .partition_point(|&start| start <= scroll)
            .saturating_sub(1)
            .min(self.lines.len());
        let mut last = first;
        while last < self.lines.len() && self.starts[last] < scroll + height {
            last += 1;
        }
        let skip = scroll.saturating_sub(self.starts.get(first).copied().unwrap_or(0));
        (first, last, skip)
    }
}
/// Run `f` on the up-to-date layout of the browser's current preview.
pub(super) fn with_doc<R>(browser: &Browser, f: impl FnOnce(&mut DocCache) -> R) -> Option<R> {
    let key = doc_key(browser)?;
    let width = layout_width(browser);
    let mut slot = browser.doc.borrow_mut();
    let fresh = match slot.as_mut() {
        Some(doc) if doc.key == key && doc.width == width => true,
        // Only markdown lays out to the width; other kinds just re-measure rows.
        Some(doc) if doc.key == key && key.kind != "markdown" => {
            doc.width = width;
            doc.measure();
            doc.link_frame.set(None);
            true
        }
        _ => false,
    };
    if !fresh {
        drop(slot);
        let doc = DocCache::build(browser, key, browser.preview.as_deref().unwrap_or(""));
        slot = browser.doc.borrow_mut();
        *slot = Some(doc);
    }
    slot.as_mut().map(f)
}
/// The link cells (x, y, tag) of the preview frame last drawn, from the link-tagged
/// layout, which is built on the first call for a document (a click), not on paint.
pub(super) fn link_cells(browser: &Browser) -> Vec<(u16, u16, Color)> {
    with_doc(browser, |doc| {
        let Some((scroll, inner)) = doc.link_frame.get() else {
            return Vec::new();
        };
        let stale = doc.cells_frame.get() != Some((scroll, inner))
            || browser.preview_link_cells.borrow().is_empty();
        if stale {
            doc.cells_frame.set(Some((scroll, inner)));
            let text = browser.preview.as_deref().unwrap_or("");
            let (first, last, skip) = doc.window(scroll as usize, usize::from(inner.height));
            let hit = doc
                .hit_lines
                .get_or_insert_with(|| markdown_layout_lines(browser, text, true));
            // Hit-test the unhighlighted presentation. Search underlines are not links.
            let mut buffer = ratatui::buffer::Buffer::empty(inner);
            let end = last.min(hit.len());
            ratatui::widgets::Widget::render(
                Paragraph::new(hit[first.min(end)..end].to_vec())
                    .wrap(Wrap { trim: false })
                    .scroll((skip.min(u16::MAX as usize) as u16, 0)),
                inner,
                &mut buffer,
            );
            let mut cells = browser.preview_link_cells.borrow_mut();
            cells.clear();
            for y in inner.y..inner.bottom() {
                for x in inner.x..inner.right() {
                    let cell = &buffer[(x, y)];
                    if cell.modifier.contains(Modifier::UNDERLINED) {
                        cells.push((x, y, cell.bg));
                    }
                }
            }
        }
        browser.preview_link_cells.borrow().clone()
    })
    .unwrap_or_default()
}
/// Search matches of the current preview for `query` (cached per layout and query).
pub(super) fn doc_matches(browser: &Browser, query: &str) -> Vec<(usize, Vec<usize>)> {
    // No query matches nothing: never lay the document out for it (a reply would lay it
    // out at the last viewport's width, and the first frame again at its own).
    if query.is_empty() {
        return Vec::new();
    }
    with_doc(browser, |doc| doc.matches(query).as_ref().clone()).unwrap_or_default()
}
/// The preview's source line count: kept by its current layout, else (before the layout
/// is built) counted from the text.
pub(super) fn source_lines(browser: &Browser) -> usize {
    let key = doc_key(browser);
    if let Some(doc) = browser
        .doc
        .borrow()
        .as_ref()
        .filter(|d| Some(&d.key) == key.as_ref())
    {
        return doc.source_lines;
    }
    browser.preview.as_deref().map_or(0, |t| t.lines().count())
}
/// Largest scroll offset of the current preview at the last rendered viewport.
pub(super) fn doc_max_scroll(browser: &Browser) -> u32 {
    let height = browser.preview_viewport.get().1.max(1);
    with_doc(browser, |doc| doc.max_scroll(height)).unwrap_or(0)
}
pub(super) fn highlight_preview_matches(lines: &mut [Line<'static>], find: &PreviewFind) {
    let mut matches = find.matches.iter().enumerate().peekable();
    for (row, line) in lines.iter_mut().enumerate() {
        if !matches.peek().is_some_and(|(_, (r, _))| *r == row) {
            continue;
        }
        let mut flags = vec![0u8; line.spans.iter().map(|s| s.content.chars().count()).sum()];
        while matches.peek().is_some_and(|(_, (r, _))| *r == row) {
            let (which, (_, positions)) = matches.next().unwrap();
            for &position in positions {
                if let Some(flag) = flags.get_mut(position) {
                    *flag = (*flag).max(if which == find.selected { 2 } else { 1 });
                }
            }
        }
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut position = 0;
        for span in std::mem::take(&mut line.spans) {
            for c in span.content.chars() {
                let mut style = span.style;
                if flags[position] > 0 {
                    style = style.add_modifier(Modifier::UNDERLINED);
                    if !ascii() && !no_color() {
                        style = style.patch(if flags[position] == 2 {
                            accent()
                        } else {
                            tok(Tok::Match)
                        });
                    }
                    if flags[position] == 2 {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                }
                if let Some(last) = spans.last_mut().filter(|last| last.style == style) {
                    last.content.to_mut().push(c);
                } else {
                    spans.push(Span::styled(c.to_string(), style));
                }
                position += 1;
            }
        }
        line.spans = spans;
    }
}
/// The preview without its frame, drawn into `inner` (the design's preview surface).
pub(super) fn render_preview_body(
    frame: &mut Frame,
    inner: Rect,
    browser: &Browser,
    text: &str,
    native: Option<&mut crate::terminal_preview::NativePreview>,
) {
    let rich = browser.preview_rich.as_ref();
    if native.is_some_and(|native| native.render(frame, inner)) {
        return;
    }
    if !ascii() && !no_color() {
        if let Some((w, h, bytes)) = rich.and_then(|p| p.raster.as_ref()) {
            frame.render_widget(Paragraph::new(raster_lines(*w, *h, bytes, inner)), inner);
            return;
        }
    }
    if rich.is_some_and(|p| p.raster.is_some()) {
        let fallback = format!(
            "{}\n{}\nColor thumbnail unavailable in ASCII/monochrome mode.",
            rich.map(|p| p.title.as_str()).unwrap_or("Preview"),
            text
        );
        frame.render_widget(Paragraph::new(fallback).wrap(Wrap { trim: false }), inner);
        return;
    }
    if browser
        .preview_viewport
        .replace((inner.width, inner.height))
        != (inner.width, inner.height)
        && !browser.preview_find.matches.is_empty()
    {
        browser.preview_find.reveal.set(true);
    }
    let cached = browser
        .preview
        .as_deref()
        .is_some_and(|p| std::ptr::eq(p, text));
    if !cached {
        // A caller-supplied text (not the browser's preview): lay it out directly.
        frame.render_widget(
            Paragraph::new(uncached_display_lines(browser, text)).wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let query = browser.preview_find.query.clone();
    let reveal = browser.preview_find.reveal.replace(false);
    let height = usize::from(inner.height);
    let rendered = with_doc(browser, |doc| {
        let matches = doc.matches(&query);
        let selected = browser
            .preview_find
            .selected
            .min(matches.len().saturating_sub(1));
        let max = doc.max_scroll(inner.height);
        let mut scroll = browser.preview_scroll.get().min(max);
        if reveal {
            if let Some((line, positions)) = matches.get(selected) {
                let before = doc.row_of(*line);
                // Prefix line_count uses Ratatui's exact wrapping, including Unicode widths.
                let prefix: String = doc.lines[*line]
                    .spans
                    .iter()
                    .flat_map(|s| s.content.chars())
                    .take(positions.first().copied().unwrap_or(0) + 1)
                    .collect();
                let within = Paragraph::new(prefix)
                    .wrap(Wrap { trim: false })
                    .line_count(inner.width.max(1))
                    .saturating_sub(1);
                scroll = ((before + within) as u64).min(u64::from(max)) as u32;
            }
        }
        browser.preview_scroll.set(scroll);
        let (first, last, skip) = doc.window(scroll as usize, height);
        let mut lines = doc.lines[first..last].to_vec();
        // Only the matches inside the window are highlighted; indices stay global for
        // the selected-match check.
        let begin = matches.partition_point(|(row, _)| *row < first);
        let end = matches.partition_point(|(row, _)| *row < last);
        let find = PreviewFind {
            query: query.clone(),
            matches: matches[begin..end]
                .iter()
                .map(|(row, positions)| (row - first, positions.clone()))
                .collect(),
            selected: selected.wrapping_sub(begin),
            reveal: std::cell::Cell::new(false),
        };
        highlight_preview_matches(&mut lines, &find);
        // Link cells are laid out when a click needs them (link_cells).
        doc.link_frame.set(doc.links.then_some((scroll, inner)));
        (lines, skip, scroll, max)
    });
    let Some((lines, skip, _, _)) = rendered else {
        return;
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((skip.min(u16::MAX as usize) as u16, 0)),
        inner,
    );
    protect_content(frame.buffer_mut(), inner);
}

/// Bytes per streamed preview request (the helper's `ReadChunk` maximum).
pub(super) const STREAM_CHUNK: u32 = 128 * 1024;
/// How much of a text/code file a preview streams; markdown is laid out whole, so less.
const STREAM_LIMIT: u64 = 4 * 1024 * 1024;
const STREAM_MARKDOWN_LIMIT: u64 = 1024 * 1024;
/// Loaded in the background right after opening; beyond this, chunks load on scroll.
const STREAM_READ_AHEAD: u64 = 1024 * 1024;

/// A text preview being extended beyond the helper's first window with `ReadChunk`
/// (identity-checked reads of the same regular file).
#[derive(Clone)]
pub(super) struct PreviewStream {
    path: String,
    identity: String,
    /// Next raw byte offset; everything before it is in the preview text.
    next: u64,
    size: u64,
    limit: u64,
    inflight: bool,
    done: bool,
    /// Stopped on a read failure or an undecodable reply, not at the end or the limit.
    failed: bool,
    /// Markdown is laid out whole, so its chunks collect here and replace the helper's
    /// window once, when the stream ends (one layout instead of one per chunk).
    pending: String,
    /// Keep the view pinned to the end while chunks arrive (End/G).
    pub(super) follow: bool,
}
/// The preview meta's cue when the preview shows only part of its file, so the line
/// End/G lands on is not mistaken for the file's last.
pub(super) fn partial_cue(b: &Browser) -> Option<String> {
    match &b.stream {
        // Nothing streamed: the helper's window is shown (a first line longer than a chunk).
        Some(s) if s.done && s.next == 0 => Some("partial".to_owned()),
        Some(s) if s.done && s.next < s.size => Some(format!(
            "{} {} of {}",
            if s.failed { "stopped at" } else { "first" },
            human_size(s.next),
            human_size(s.size)
        )),
        Some(_) => None,
        None => b
            .preview_rich
            .as_ref()
            .is_some_and(|p| p.truncated)
            .then(|| "partial".to_owned()),
    }
}
/// A streamed continuation prepared by the request worker (decode, normalize, highlight).
#[derive(Clone)]
pub(super) struct PreviewChunk {
    /// Raw bytes covered by `text`: whole lines unless `eof`. Zero when a line is longer
    /// than the chunk: the stream ends there rather than break the line (or a character).
    pub(super) consumed: u64,
    eof: bool,
    pub(super) text: String,
    pub(super) lines: Option<Vec<Line<'static>>>,
}
/// Worker-side: turn a preview `ReadChunk` reply into display-ready text off the UI
/// thread. Only whole lines are consumed, so the next request starts on a line.
pub(super) fn stream_chunk(op: &Operation, value: &Value) -> Option<RichPreview> {
    use base64::Engine;
    let Operation::ReadChunk {
        path,
        limit,
        offset,
        ..
    } = op
    else {
        return None;
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value["data"].as_str()?)
        .ok()?;
    if bytes.len() > *limit as usize {
        return None;
    }
    let eof = bytes.len() < *limit as usize;
    let cut = if eof {
        bytes.len()
    } else {
        bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1)
    };
    let text = safe_text(&crate::files::preview_text(&bytes[..cut]));
    let code =
        crate::files::decode_path(path).is_ok_and(|p| crate::files::preview_kind(&p) == "code");
    let lines = code.then(|| {
        crate::syntax_preview::highlight_chunk(&text, path, *offset, *offset + cut as u64)
    });
    Some(RichPreview {
        kind: "chunk".into(),
        title: String::new(),
        raster: None,
        styled: None,
        truncated: false,
        page: 1,
        pages: None,
        chunk: Some(PreviewChunk {
            consumed: cut as u64,
            eof,
            text,
            lines,
        }),
    })
}

/// Worker-side: the source highlighting of a code preview, delivered after its plain
/// first paint as a `"highlight"` reply whose chunk carries the text it highlights.
pub(super) fn highlighted_preview(text: String, path: &str) -> RichPreview {
    let lines = crate::syntax_preview::highlight(&text, path);
    RichPreview {
        kind: "highlight".into(),
        title: String::new(),
        raster: None,
        styled: None,
        truncated: false,
        page: 1,
        pages: None,
        chunk: Some(PreviewChunk {
            consumed: 0,
            eof: true,
            text,
            lines: Some(lines),
        }),
    }
}
/// Installs late highlighting on a still-plain code preview of exactly that text.
/// Returns false when the preview moved on (other file, new text, streamed chunk).
pub(super) fn apply_highlight(b: &mut Browser, chunk: PreviewChunk) -> bool {
    let Some(rich) = b
        .preview_rich
        .as_mut()
        .filter(|r| r.kind == "code" && r.styled.is_none())
    else {
        return false;
    };
    if b.preview.as_deref() != Some(chunk.text.as_str()) {
        return false;
    }
    rich.styled = chunk.lines;
    b.preview_revision = b.preview_revision.wrapping_add(1);
    true
}

/// Add one streamed text/code chunk to the preview (offset 0 replaces the helper's window).
fn append_chunk(b: &mut Browser, offset: u64, chunk: PreviewChunk, kind: &str) {
    let code = kind == "code";
    if offset == 0 {
        b.preview = Some(chunk.text);
        if let Some(rich) = b.preview_rich.as_mut() {
            rich.styled = if code { chunk.lines } else { None };
        }
        b.preview_revision = b.preview_revision.wrapping_add(1);
        return;
    }
    let added: Vec<Line<'static>> = if ascii() || no_color() {
        chunk
            .text
            .lines()
            .map(|l| Line::raw(l.to_owned()))
            .collect()
    } else {
        match chunk.lines {
            Some(lines) if code => lines,
            _ => preview_lines(&chunk.text, kind),
        }
    };
    let old_key = doc_key(b);
    if let Some(text) = b.preview.as_mut() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&chunk.text);
    }
    b.preview_revision = b.preview_revision.wrapping_add(1);
    let new_key = doc_key(b);
    // The retained layout owns streamed lines. Only without one do they extend `styled`,
    // which the next layout is built from: keeping both would hold the document twice.
    let mut doc = b.doc.borrow_mut();
    match (old_key, new_key, doc.as_mut()) {
        (Some(old), Some(new), Some(doc)) if doc.key == old => doc.append(new, added),
        _ if code => {
            if let Some(styled) = b.preview_rich.as_mut().and_then(|p| p.styled.as_mut()) {
                styled.extend(added);
            }
        }
        _ => {}
    }
}

impl App {
    /// A `"highlight"` follow-up reply: style the main or peek preview it belongs to.
    pub(super) fn apply_highlight_reply(&mut self, reply: Reply) {
        let Some(chunk) = reply.preview.and_then(|p| p.chunk) else {
            return;
        };
        let Some(b) = self.browser.as_mut().filter(|b| b.device == reply.device) else {
            return;
        };
        if !apply_highlight(b, chunk.clone()) {
            if let Some(peek) = b.peek.as_deref_mut() {
                apply_highlight(peek, chunk);
            }
        }
    }
    /// After a text preview arrives truncated, stream the rest of the file in chunks.
    pub(super) fn start_preview_stream(&mut self, value: &Value) {
        let Some(b) = self.browser.as_mut() else {
            return;
        };
        b.stream = None;
        let kind = b
            .preview_rich
            .as_ref()
            .map(|p| p.kind.clone())
            .unwrap_or_default();
        if value["truncated"] != Value::Bool(true)
            || value["binary"] == Value::Bool(true)
            || !matches!(kind.as_str(), "text" | "code" | "markdown")
            || b.container.is_some()
        {
            return;
        }
        let Some(path) = b.preview_path.clone() else {
            return;
        };
        let Some(entry) = b
            .entries
            .iter()
            .find(|e| e.path == path && e.kind == "file")
        else {
            return;
        };
        let Some(identity) = entry.identity.clone() else {
            return;
        };
        b.stream = Some(PreviewStream {
            path,
            identity,
            next: 0,
            size: entry.size,
            limit: if kind == "markdown" {
                STREAM_MARKDOWN_LIMIT
            } else {
                STREAM_LIMIT
            },
            inflight: false,
            done: false,
            failed: false,
            pending: String::new(),
            follow: false,
        });
        self.pump_preview_stream();
    }
    /// Request the next chunk when the view nears the loaded end (or follows End).
    pub(super) fn pump_preview_stream(&mut self) {
        let Some(b) = self.browser.as_ref() else {
            return;
        };
        let Some(stream) = b.stream.as_ref() else {
            return;
        };
        if stream.inflight
            || stream.done
            || b.preview_path.as_ref() != Some(&stream.path)
            || b.preview.is_none()
        {
            return;
        }
        let height = u64::from(b.preview_viewport.get().1.max(1));
        let near_end = stream.next < STREAM_READ_AHEAD
            || stream.follow
            || with_doc(b, |doc| doc.total_rows() as u64)
                .is_some_and(|total| u64::from(b.preview_scroll.get()) + 4 * height >= total);
        if !near_end {
            return;
        }
        let op = Operation::ReadChunk {
            path: stream.path.clone(),
            offset: stream.next,
            limit: STREAM_CHUNK,
            identity: stream.identity.clone(),
        };
        let device = b.device;
        if self.send_explicit(device, op) {
            if let Some(stream) = self.browser.as_mut().and_then(|b| b.stream.as_mut()) {
                stream.inflight = true;
            }
        }
    }
    pub(super) fn apply_preview_chunk(
        &mut self,
        device: usize,
        path: &str,
        offset: u64,
        result: Result<Value>,
        prepared: Option<RichPreview>,
    ) {
        let Some(b) = self.browser.as_mut().filter(|b| b.device == device) else {
            return;
        };
        let Some(stream) = b
            .stream
            .as_mut()
            .filter(|s| s.path == path && s.next == offset && s.inflight)
        else {
            return;
        };
        stream.inflight = false;
        let (chunk, warning) = match (&result, prepared.and_then(|p| p.chunk)) {
            (Ok(_), Some(chunk)) => (Some(chunk).filter(|c| c.consumed > 0), None),
            (Ok(_), None) => {
                stream.failed = true;
                (None, None)
            }
            (Err(e), _) => {
                stream.failed = true;
                let message = format!(
                    "Preview stopped at {} · {}",
                    human_size(stream.next),
                    safe_text(&format!("{e:#}"))
                );
                (None, Some(message))
            }
        };
        let kind = b
            .preview_rich
            .as_ref()
            .map(|p| p.kind.clone())
            .unwrap_or_default();
        match chunk {
            None => stream.done = true,
            Some(chunk) => {
                stream.next += chunk.consumed;
                stream.done =
                    chunk.eof || stream.next >= stream.limit || stream.next >= stream.size;
                if kind == "markdown" {
                    stream.pending.push_str(&chunk.text);
                } else {
                    append_chunk(b, offset, chunk, &kind);
                }
            }
        }
        let stream = b.stream.as_mut().unwrap();
        if stream.done && !stream.pending.is_empty() {
            b.preview = Some(std::mem::take(&mut stream.pending));
            if let Some(rich) = b.preview_rich.as_mut() {
                rich.styled = None;
            }
            b.preview_revision = b.preview_revision.wrapping_add(1);
        }
        let follow = b.stream.as_ref().is_some_and(|s| s.follow);
        if !b.preview_find.query.is_empty() {
            let query = b.preview_find.query.clone();
            b.preview_find.matches = doc_matches(b, &query);
        }
        if follow {
            b.preview_scroll.set(u32::MAX);
        }
        if let Some(message) = warning {
            self.set_notice_as(NoticeKind::Warning, message);
        }
        self.pump_preview_stream();
    }
    pub(super) fn pdf_preview_active(&self) -> bool {
        self.view == View::Files
            && self.focus == Focus::Workspace
            && self.browser.as_ref().is_some_and(|b| {
                b.preview.is_some()
                    && b.preview_rich
                        .as_ref()
                        .is_some_and(|p| p.kind == "pdf" && p.raster.is_some())
            })
    }
    pub(super) fn open_preview(&mut self, device: usize, path: String) {
        self.generation += 1;
        if let Some(b) = &mut self.browser {
            b.preview_path = Some(path.clone());
            b.preview_restore = None;
            b.preview_anchor = None;
            b.preview_link_cells.borrow_mut().clear();
            b.preview_requested_page = 1;
            b.preview_pending_page = Some(1);
            b.preview_rich = None;
            b.preview_scroll.set(0);
            b.preview_find = PreviewFind::default();
            b.preview = Some("Loading preview…".into());
            b.stream = None;
            b.doc.borrow_mut().take();
            // The inline peek already holds this file: show it now instead of re-reading.
            let loaded = b.peek.as_mut().filter(|p| {
                p.preview_path.as_ref() == Some(&path) && p.preview_pending_page.is_none()
            });
            if let Some(value) = loaded
                .and_then(|p| p.peek_value.take())
                .filter(|v| !v.is_null())
            {
                let rich = b.peek.as_mut().and_then(|p| p.preview_rich.take());
                b.peek = None;
                let shown = matches!(
                    super::apply::files::apply_preview(b, &path, 1, rich, &value),
                    super::apply::files::PreviewApplied::Shown { .. }
                );
                if shown {
                    self.start_preview_stream(&value);
                }
                return;
            }
        }
        // The peek is still reading this file (a PDF page takes seconds): its reply
        // opens the preview instead of a second read queued behind it.
        if self
            .browser
            .as_ref()
            .is_some_and(|b| b.peek_inflight.iter().any(|(p, _)| *p == path))
        {
            return;
        }
        if !self.send(device, Operation::Preview { path }) {
            if let Some(b) = &mut self.browser {
                b.preview_pending_page = None;
                b.preview = Some("Preview queue busy · Escape and retry".into());
            }
        }
    }
    /// Enter waited on the peek's read of `path` (see `open_preview`): now that it has
    /// answered, show it, or read again when it failed or was cancelled.
    pub(super) fn open_waiting_preview(&mut self, device: usize, path: String) {
        let waiting = self.browser.as_ref().is_some_and(|b| {
            b.device == device
                && b.preview_path.as_ref() == Some(&path)
                && b.preview_pending_page == Some(1)
        });
        if waiting {
            self.open_preview(device, path);
        }
    }
    pub(super) fn start_pdf_page_request(&mut self) {
        let Some(b) = self.browser.as_ref() else {
            return;
        };
        if b.preview_pending_page.is_some()
            || b.preview_rich
                .as_ref()
                .is_some_and(|p| p.page == b.preview_requested_page)
        {
            return;
        }
        let device = b.device;
        let path = b
            .preview_path
            .clone()
            .or_else(|| browser_entries(b).get(b.selected).map(|e| e.path.clone()));
        let Some(path) = path else {
            return;
        };
        let page = b.preview_requested_page.max(1);
        if self.devices[device].target.is_some()
            && !self
                .providers
                .get(&device)
                .is_some_and(|(caps, _)| caps.iter().any(|c| c == "pdf-pages-v1"))
        {
            self.providers.remove(&device);
            self.check_providers(device);
            self.set_notice("PDF pages need a current device helper · checking update".into());
            return;
        }
        if self.send(
            device,
            Operation::PreviewPage {
                path: path.clone(),
                page,
            },
        ) {
            if let Some(b) = &mut self.browser {
                b.preview_path = Some(path);
                b.preview_pending_page = Some(page);
            }
        } else {
            self.set_notice_as(
                NoticeKind::Warning,
                "Preview queue busy · retry shortly".into(),
            );
        }
    }
    pub(super) fn text_preview_active(&self) -> bool {
        self.view == View::Files
            && self.browser.as_ref().is_some_and(|b| {
                b.preview.is_some()
                    && b.preview_pending_page.is_none()
                    && b.preview_rich
                        .as_ref()
                        .is_none_or(|p| p.raster.is_none() && p.kind != "pdf")
            })
    }
    pub(super) fn update_preview_search(&mut self) {
        if let Some(b) = &mut self.browser {
            b.preview_find.query = self.text.clone();
            b.preview_find.matches = doc_matches(b, &self.text);
            b.preview_find.selected = 0;
            b.preview_find.reveal.set(true);
        }
    }
    pub(super) fn next_preview_match(&mut self, direction: isize) {
        if let Some(b) = &mut self.browser {
            b.preview_find.matches = doc_matches(b, &b.preview_find.query);
            let count = b.preview_find.matches.len();
            if count > 0 {
                b.preview_find.selected = b.preview_find.selected.min(count - 1);
                b.preview_find.selected = (b.preview_find.selected as isize + direction)
                    .rem_euclid(count as isize) as usize;
                b.preview_find.reveal.set(true);
            }
        }
    }
    pub(super) fn markdown_preview_active(&self) -> bool {
        self.text_preview_active()
            && self.browser.as_ref().is_some_and(|b| {
                b.preview_rich
                    .as_ref()
                    .is_some_and(|p| p.kind == "markdown")
            })
    }
    pub(super) fn open_markdown_link(&mut self, link: &crate::markdown_links::Link) {
        if crate::markdown_links::web_target(&link.target) {
            if std::env::var_os("DISPLAY").is_none()
                && std::env::var_os("WAYLAND_DISPLAY").is_none()
            {
                self.set_notice_as(
                    NoticeKind::Warning,
                    "No graphical browser on this viewer · inspect the URL with o".into(),
                );
                return;
            }
            match std::process::Command::new("xdg-open")
                .arg(&link.target)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    self.set_notice_as(NoticeKind::Success, "Link sent to viewer browser".into());
                    self.dialog = None;
                }
                Err(_) => self.set_notice_as(
                    NoticeKind::Warning,
                    "Viewer browser unavailable · inspect the URL with o".into(),
                ),
            }
            return;
        }
        let Some(b) = self.browser.as_ref() else {
            return;
        };
        let Some(document) = b.preview_path.as_ref() else {
            self.set_notice_as(
                NoticeKind::Warning,
                "Document path unavailable · reopen this file".into(),
            );
            return;
        };
        let resolved = crate::files::decode_path(document)
            .and_then(|p| crate::markdown_links::resolve_file_link(&p, &link.target));
        let (path, anchor) = match resolved {
            Ok(Some(destination)) => destination,
            Ok(None) => {
                self.set_notice_as(
                    NoticeKind::Warning,
                    "Only web URLs and file links can open".into(),
                );
                return;
            }
            Err(error) => {
                self.set_notice_as(NoticeKind::Error, safe_text(&error.to_string()));
                return;
            }
        };
        let device = b.device;
        let same = crate::files::decode_path(document).ok().as_ref() == Some(&path);
        if same && anchor.is_some() {
            self.dialog = None;
            let b = self.browser.as_mut().unwrap();
            if let Some(scroll) = markdown_anchor_scroll(b, anchor.as_deref().unwrap()) {
                b.preview_scroll.set(scroll);
                b.preview_find.reveal.set(false);
            } else {
                self.set_notice_as(
                    NoticeKind::Warning,
                    "Heading not found in this preview".into(),
                );
            }
            return;
        }
        self.dialog = None;
        if link
            .target
            .split('#')
            .next()
            .is_some_and(|p| p.ends_with('/'))
        {
            self.open_browser(device, crate::files::encode_path(&path));
            return;
        }
        let b = self.browser.as_mut().unwrap();
        if let Some(previous) = b.preview_path.clone() {
            if b.preview_history.len() >= 16 {
                b.preview_history.remove(0);
            }
            b.preview_history
                .push((previous, b.preview_scroll.get(), b.preview_find.clone()));
        }
        self.open_preview(device, crate::files::encode_path(&path));
        self.browser.as_mut().unwrap().preview_anchor = anchor;
    }
    pub(super) fn show_preview_links(&mut self) {
        let links = self
            .browser
            .as_ref()
            .and_then(|b| b.preview.as_ref())
            .map(|s| markdown_preview_links(s))
            .unwrap_or_default();
        if links.is_empty() {
            self.set_notice_as(
                NoticeKind::Warning,
                "No Markdown links in this preview".into(),
            );
            return;
        }
        self.dialog_selected = 0;
        self.dialog_scroll = 0;
        self.dialog_detail_focus = false;
        self.dialog = Some(Dialog::Links(links));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fast path for a row that fits draws exactly what the wrap pass draws.
    #[test]
    fn a_hanging_row_that_fits_draws_like_the_wrapped_one() {
        let code = tok(Tok::Code);
        let rows = [
            Line::from(vec![
                Span::raw("- list item 12 with "),
                Span::styled("code", code),
            ]),
            Line::from(vec![Span::raw("  "), Span::raw("continued words  ")]),
            Line::from(vec![
                Span::styled("1. ", muted()),
                Span::raw("漢字 and *emphasis*"),
            ]),
        ];
        let draw = |lines: Vec<Line<'static>>| {
            let rect = Rect::new(0, 0, 60, lines.len() as u16);
            let mut buffer = ratatui::buffer::Buffer::empty(rect);
            ratatui::widgets::Widget::render(Paragraph::new(lines), rect, &mut buffer);
            buffer
        };
        for (line, prefix) in rows.iter().zip([2, 2, 3]) {
            let fast = hanging_lines(line, prefix, 60);
            assert_eq!(fast.len(), 1);
            let (leading, body) = split_prefix(line, prefix);
            let slow = wrap_hanging(line, leading, body, prefix, 60 - prefix as u16);
            assert_eq!(draw(fast), draw(slow), "{line:?}");
        }
    }
}
