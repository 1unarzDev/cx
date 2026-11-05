//! File preview rendering: text, markdown layout/links/emphasis, raster fallback and in-preview find.
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
pub(super) fn markdown_anchor_scroll(browser: &Browser, anchor: &str) -> Option<u16> {
    let lines = preview_display_lines(browser, browser.preview.as_deref()?);
    let row = *browser
        .markdown_anchor_rows
        .borrow()
        .get(&markdown_heading_slug(anchor))?;
    Some(
        Paragraph::new(lines[..row].to_vec())
            .wrap(Wrap { trim: false })
            .line_count(browser.preview_viewport.get().0.max(1))
            .min(u16::MAX as usize) as u16,
    )
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
pub(super) fn preview_inline(text: &str) -> Vec<Span<'static>> {
    preview_inline_tagged(text, false)
}
pub(super) fn preview_inline_tagged(text: &str, tagged: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut at = 0;
    for link in crate::markdown_links::links(text) {
        spans.extend(preview_emphasis(&text[at..link.start]));
        let mut style = tint(Color::Cyan).add_modifier(Modifier::UNDERLINED);
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
    let body_width = width - prefix as u16;
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
                        spans.push(Span::styled(rendered, tint(Color::Magenta)));
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
                "`" => tint(Color::Yellow),
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
                        Span::styled(if ascii() { "+-- " } else { "┌─ " }, muted()),
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
                    return Line::from(Span::styled(if ascii() { "+--" } else { "└─" }, muted()));
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
pub(super) fn markdown_display_lines(browser: &Browser, text: &str) -> Vec<Line<'static>> {
    let width = browser.preview_viewport.get().0;
    if let Some((revision, cached_width, source, lines)) = browser.markdown_cache.borrow().as_ref()
    {
        if *revision == browser.preview_revision && *cached_width == width && source == text {
            return lines.clone();
        }
    }
    let lines = markdown_layout_lines(browser, text, false);
    *browser.markdown_hit_lines.borrow_mut() = markdown_layout_lines(browser, text, true);
    *browser.markdown_cache.borrow_mut() = Some((
        browser.preview_revision,
        width,
        text.to_owned(),
        lines.clone(),
    ));
    lines
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
                                lines[row + 1] =
                                    Line::from(Span::styled(rendered, tint(Color::Magenta)));
                            } else {
                                lines[row] =
                                    Line::from(Span::styled(rendered, tint(Color::Magenta)));
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
pub(super) fn preview_display_lines(browser: &Browser, text: &str) -> Vec<Line<'static>> {
    let rich = browser.preview_rich.as_ref();
    if rich.is_some_and(|p| p.kind == "markdown") {
        let mut lines = markdown_display_lines(browser, text);
        if ascii() || no_color() {
            for line in &mut lines {
                line.style.fg = None;
                line.style.bg = None;
                for span in &mut line.spans {
                    span.style.fg = None;
                    span.style.bg = None;
                }
            }
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
pub(super) fn preview_max_scroll(lines: &[Line<'static>], width: u16, height: u16) -> u16 {
    Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
        .saturating_sub(usize::from(height))
        .min(u16::MAX as usize) as u16
}
pub(super) fn preview_matches(lines: &[Line<'static>], query: &str) -> Vec<(usize, Vec<usize>)> {
    let needle: Vec<_> = query.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut result = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        let text: Vec<_> = line
            .spans
            .iter()
            .flat_map(|s| s.content.chars())
            .enumerate()
            .flat_map(|(i, c)| c.to_lowercase().map(move |c| (c, i)))
            .collect();
        // Prefer every contiguous occurrence; otherwise use one fuzzy subsequence per line.
        let mut exact = false;
        for (start, window) in text.windows(needle.len()).enumerate() {
            if window.iter().map(|(c, _)| *c).eq(needle.iter().copied()) {
                result.push((
                    row,
                    text[start..start + needle.len()]
                        .iter()
                        .map(|(_, i)| *i)
                        .collect(),
                ));
                exact = true;
            }
        }
        if !exact {
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
                result.push((row, positions));
            }
        }
    }
    result
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
                        style = style.fg(if flags[position] == 2 {
                            Color::Cyan
                        } else {
                            Color::Yellow
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
pub(super) fn render_preview(
    frame: &mut Frame,
    area: Rect,
    browser: &Browser,
    text: &str,
    focused: bool,
) {
    render_preview_with_native(frame, area, browser, text, focused, None)
}
pub(super) fn render_preview_with_native(
    frame: &mut Frame,
    area: Rect,
    browser: &Browser,
    text: &str,
    focused: bool,
    native: Option<&mut crate::terminal_preview::NativePreview>,
) {
    let rich = browser.preview_rich.as_ref();
    let pdf_title = rich.filter(|p| p.kind == "pdf").map(|p| {
        format!(
            "PDF · {}/{}{}",
            p.page,
            p.pages.map(|n| n.to_string()).unwrap_or("?".into()),
            browser
                .preview_pending_page
                .map(|page| format!(" · {} page {}", if ascii() { "~" } else { "◌" }, page))
                .unwrap_or_default()
        )
    });
    let title = pdf_title
        .as_deref()
        .or_else(|| rich.map(|p| p.title.as_str()))
        .filter(|t| !t.is_empty())
        .unwrap_or("Preview");
    let border = block(
        format!(
            "{} · Escape back",
            fit_label(title, usize::from(area.width.saturating_sub(22)))
        ),
        focused,
    );
    let inner = border.inner(area);
    frame.render_widget(border, area);
    if native.is_some_and(|native| native.render(frame, inner)) {
        return;
    }
    if !ascii() && !no_color() {
        if let Some((w, h, bytes)) = rich.and_then(|p| p.raster.as_ref()) {
            frame.render_widget(Paragraph::new(raster_lines(*w, *h, bytes, inner)), inner);
            return;
        }
    }
    let fallback;
    let text = if rich.is_some_and(|p| p.raster.is_some()) {
        fallback = format!(
            "{}\n{}\nColor thumbnail unavailable in ASCII/monochrome mode.",
            title, text
        );
        fallback.as_str()
    } else {
        text
    };
    if browser
        .preview_viewport
        .replace((inner.width, inner.height))
        != (inner.width, inner.height)
        && !browser.preview_find.matches.is_empty()
    {
        browser.preview_find.reveal.set(true);
    }
    let mut lines = preview_display_lines(browser, text);
    let mut find = browser.preview_find.clone();
    find.matches = preview_matches(&lines, &find.query);
    find.selected = find.selected.min(find.matches.len().saturating_sub(1));
    let max = preview_max_scroll(&lines, inner.width, inner.height);
    let mut scroll = browser.preview_scroll.get().min(max);
    if browser.preview_find.reveal.replace(false) {
        if let Some((line, positions)) = find.matches.get(find.selected) {
            let before = Paragraph::new(lines[..*line].to_vec())
                .wrap(Wrap { trim: false })
                .line_count(inner.width.max(1));
            // Prefix line_count uses Ratatui's exact wrapping, including Unicode widths.
            let prefix: String = lines[*line]
                .spans
                .iter()
                .flat_map(|s| s.content.chars())
                .take(positions.first().copied().unwrap_or(0) + 1)
                .collect();
            let within = Paragraph::new(prefix)
                .wrap(Wrap { trim: false })
                .line_count(inner.width.max(1))
                .saturating_sub(1);
            scroll = (before + within).min(usize::from(max)) as u16;
        }
    }
    browser.preview_scroll.set(scroll);
    if max > 0 {
        let position = format!(" {}% ", u32::from(scroll) * 100 / u32::from(max));
        let width = (position.len() as u16).min(area.width.saturating_sub(2));
        if width > 0 && area.height > 0 {
            frame.render_widget(
                Paragraph::new(Span::styled(position, muted())),
                Rect::new(
                    area.right().saturating_sub(width + 1),
                    area.bottom() - 1,
                    width,
                    1,
                ),
            );
        }
    }
    browser.preview_link_cells.borrow_mut().clear();
    if rich.is_some_and(|p| p.kind == "markdown") && !markdown_preview_links(text).is_empty() {
        // Hit-test the unhighlighted presentation. Search underlines are not links.
        let mut buffer = ratatui::buffer::Buffer::empty(inner);
        ratatui::widgets::Widget::render(
            Paragraph::new(browser.markdown_hit_lines.borrow().clone())
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            inner,
            &mut buffer,
        );
        let mut cells = browser.preview_link_cells.borrow_mut();
        for y in inner.y..inner.bottom() {
            for x in inner.x..inner.right() {
                let cell = &buffer[(x, y)];
                if cell.modifier.contains(Modifier::UNDERLINED) {
                    cells.push((x, y, cell.bg));
                }
            }
        }
    }
    highlight_preview_matches(&mut lines, &find);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        inner,
    );
}

impl App {
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
        }
        if !self.send(device, Operation::Preview { path }) {
            if let Some(b) = &mut self.browser {
                b.preview_pending_page = None;
                b.preview = Some("Preview queue busy · Escape and retry".into());
            }
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
            b.preview_find.matches = preview_matches(
                &preview_display_lines(b, b.preview.as_deref().unwrap_or("")),
                &self.text,
            );
            b.preview_find.selected = 0;
            b.preview_find.reveal.set(true);
        }
    }
    pub(super) fn next_preview_match(&mut self, direction: isize) {
        if let Some(b) = &mut self.browser {
            b.preview_find.matches = preview_matches(
                &preview_display_lines(b, b.preview.as_deref().unwrap_or("")),
                &b.preview_find.query,
            );
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
