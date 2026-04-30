//! Markdown → ratatui `Line` renderer.
//!
//! Supports:
//!   - Headings H1–H3 (`#`, `##`, `###`)
//!   - Bold (`**text**`), italic (`*text*`), bold-italic (`***text***`)
//!   - Inline code (`` `code` ``)
//!   - Fenced code blocks (``` ``` ```)
//!   - Blockquotes (`> `)
//!   - Unordered lists (`-`, `*`, `+`)
//!   - Ordered lists (`1.`, `2.`, …)
//!   - Horizontal rules (`---`, `***`, `___`)
//!   - Links — renders display text only (`[text](url)`)
//!   - Plain paragraph text (word-wrapped)

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

// ============================================================================
// Theme Styles (local — mirrors theme.rs but owned here for independence)
// ============================================================================

const S_NORMAL:    Style = Style::new().fg(Color::White);
const S_H1:        Style = Style::new().fg(Color::Cyan)  .add_modifier(Modifier::BOLD);
const S_H2:        Style = Style::new().fg(Color::LightCyan).add_modifier(Modifier::BOLD);
const S_H3:        Style = Style::new().fg(Color::Cyan);
const S_BOLD:      Style = Style::new().fg(Color::White) .add_modifier(Modifier::BOLD);
const S_ITALIC:    Style = Style::new().fg(Color::White) .add_modifier(Modifier::ITALIC);
const S_BOLD_ITAL: Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD).add_modifier(Modifier::ITALIC);
const S_CODE:      Style = Style::new().fg(Color::Yellow);
const S_CODE_BG:   Style = Style::new().fg(Color::Yellow).bg(Color::Black);
const S_BLOCKQUOTE:Style = Style::new().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
const S_LIST_BULLET:Style= Style::new().fg(Color::Cyan);
const S_RULE:      Style = Style::new().fg(Color::DarkGray);
const S_LINK_TEXT: Style = Style::new().fg(Color::LightBlue).add_modifier(Modifier::UNDERLINED);

// ============================================================================
// Public API
// ============================================================================

/// Convert a markdown string into a vector of ratatui `Line`s, word-wrapped
/// to `width` columns. The result can be fed directly into `Paragraph::new`.
pub fn render_markdown<'a>(md: &'a str, width: usize) -> Vec<Line<'a>> {
    let mut out: Vec<Line<'static>> = Vec::new();

    let mut iter = md.lines().peekable();

    while let Some(raw) = iter.next() {
        let trimmed = raw.trim_end();

        // ── Fenced code block ───────────────────────────────────────────
        if trimmed.starts_with("```") {
            let lang = trimmed.trim_start_matches('`').trim();
            let label = if lang.is_empty() { "code".to_string() } else { lang.to_string() };
            out.push(Line::from(vec![
                Span::styled("╭─ ".to_string(), S_RULE),
                Span::styled(label, S_CODE),
                Span::styled(" ─".to_string(), S_RULE),
            ]));
            loop {
                match iter.next() {
                    None => break,
                    Some(l) if l.trim_end().starts_with("```") => break,
                    Some(l) => {
                        // Pad/truncate to width for visual consistency.
                        let display = format!("│ {}", l);
                        out.push(Line::from(Span::styled(display, S_CODE_BG)));
                    }
                }
            }
            out.push(Line::from(Span::styled("╰─".to_string(), S_RULE)));
            continue;
        }

        // ── Horizontal rule ─────────────────────────────────────────────
        if is_hr(trimmed) {
            let rule: String = "─".repeat(width.min(80));
            out.push(Line::from(Span::styled(rule, S_RULE)));
            continue;
        }

        // ── Heading ──────────────────────────────────────────────────────
        if let Some(heading) = parse_heading(trimmed) {
            out.push(heading);
            continue;
        }

        // ── Blockquote ───────────────────────────────────────────────────
        if trimmed.starts_with("> ") || trimmed == ">" {
            let content = trimmed.trim_start_matches('>').trim_start();
            let inner_w = width.saturating_sub(3);
            for (i, chunk) in word_wrap(content, inner_w).into_iter().enumerate() {
                let bar = if i == 0 { "▌ " } else { "  " };
                let mut spans = vec![Span::styled(bar.to_string(), S_LIST_BULLET)];
                spans.extend(inline_spans(&chunk));
                // Re-style inline spans with blockquote style if they are plain.
                let styled: Vec<Span<'static>> = spans.into_iter().map(|s| {
                    if s.style == S_NORMAL { Span::styled(s.content, S_BLOCKQUOTE) } else { s }
                }).collect();
                out.push(Line::from(styled));
            }
            continue;
        }

        // ── Unordered list ───────────────────────────────────────────────
        if let Some(content) = parse_unordered(trimmed) {
            let inner_w = width.saturating_sub(4);
            for (i, chunk) in word_wrap(content, inner_w).into_iter().enumerate() {
                let prefix = if i == 0 { "  • " } else { "    " };
                let mut spans = vec![Span::styled(prefix, S_LIST_BULLET)];
                spans.extend(inline_spans(&chunk));
                out.push(Line::from(spans));
            }
            continue;
        }

        // ── Ordered list ─────────────────────────────────────────────────
        if let Some((num, content)) = parse_ordered(trimmed) {
            let inner_w = width.saturating_sub(5);
            for (i, chunk) in word_wrap(content, inner_w).into_iter().enumerate() {
                let prefix = if i == 0 { format!(" {}. ", num) } else { "    ".to_string() };
                let mut spans = vec![Span::styled(prefix, S_LIST_BULLET)];
                spans.extend(inline_spans(&chunk));
                out.push(Line::from(spans));
            }
            continue;
        }

        // ── Empty line ────────────────────────────────────────────────────
        if trimmed.is_empty() {
            out.push(Line::default());
            continue;
        }

        // ── Normal paragraph (word-wrapped, inline styling) ──────────────
        let inner_w = width.saturating_sub(2);
        for chunk in word_wrap(trimmed, inner_w) {
            let spans = inline_spans(&chunk);
            out.push(Line::from(spans));
        }
    }

    out
}

// ============================================================================
// Block-level parsers
// ============================================================================

fn is_hr(s: &str) -> bool {
    let stripped: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (stripped.chars().all(|c| c == '-') || stripped.chars().all(|c| c == '*') || stripped.chars().all(|c| c == '_'))
        && stripped.len() >= 3
}

fn parse_heading(s: &str) -> Option<Line<'static>> {
    let (level, rest) = if s.starts_with("### ") { (3, &s[4..]) }
        else if s.starts_with("## ")  { (2, &s[3..]) }
        else if s.starts_with("# ")   { (1, &s[2..]) }
        else { return None; };

    let (prefix, style) = match level {
        1 => ("█ ", S_H1),
        2 => ("▌ ", S_H2),
        _ => ("░ ", S_H3),
    };

    let mut spans = vec![Span::styled(prefix, style)];
    // Inline styling inside headings too.
    for span in inline_spans(rest) {
        spans.push(Span::styled(span.content, style));
    }
    Some(Line::from(spans))
}

fn parse_unordered(s: &str) -> Option<&str> {
    if let Some(rest) = s.strip_prefix("- ").or_else(|| s.strip_prefix("* ")).or_else(|| s.strip_prefix("+ ")) {
        Some(rest)
    } else {
        None
    }
}

fn parse_ordered(s: &str) -> Option<(usize, &str)> {
    let dot = s.find('.')?;
    let num_str = &s[..dot];
    if num_str.chars().all(|c| c.is_ascii_digit()) && !num_str.is_empty() {
        let n: usize = num_str.parse().ok()?;
        let rest = s[dot + 1..].trim_start();
        Some((n, rest))
    } else {
        None
    }
}

// ============================================================================
// Inline renderer
// ============================================================================

/// Parse inline markdown (bold, italic, code, links) into styled spans.
fn inline_spans(s: &str) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut plain = String::new();

    macro_rules! flush {
        () => {
            if !plain.is_empty() {
                out.push(Span::styled(plain.clone(), S_NORMAL));
                plain.clear();
            }
        };
    }

    while i < len {
        // ── Link: [text](url) ────────────────────────────────────────────
        if chars[i] == '[' {
            if let Some((text, skip)) = try_parse_link(&chars[i..]) {
                flush!();
                out.push(Span::styled(format!("[{}]", text), S_LINK_TEXT));
                i += skip;
                continue;
            }
        }

        // ── Inline code: `…` ─────────────────────────────────────────────
        if chars[i] == '`' {
            if let Some((code, skip)) = try_parse_delimited(&chars[i..], '`', '`') {
                flush!();
                out.push(Span::styled(code, S_CODE));
                i += skip;
                continue;
            }
        }

        // ── Bold-italic: ***…*** ─────────────────────────────────────────
        if i + 2 < len && chars[i] == '*' && chars[i+1] == '*' && chars[i+2] == '*' {
            if let Some((text, skip)) = try_parse_multi(&chars[i..], "***") {
                flush!();
                out.push(Span::styled(text, S_BOLD_ITAL));
                i += skip;
                continue;
            }
        }

        // ── Bold: **…** or __…__ ─────────────────────────────────────────
        if i + 1 < len && ((chars[i] == '*' && chars[i+1] == '*') || (chars[i] == '_' && chars[i+1] == '_')) {
            let marker = if chars[i] == '*' { "**" } else { "__" };
            if let Some((text, skip)) = try_parse_multi(&chars[i..], marker) {
                flush!();
                out.push(Span::styled(text, S_BOLD));
                i += skip;
                continue;
            }
        }

        // ── Italic: *…* or _…_ ──────────────────────────────────────────
        if chars[i] == '*' || chars[i] == '_' {
            let marker = if chars[i] == '*' { "*" } else { "_" };
            if let Some((text, skip)) = try_parse_multi(&chars[i..], marker) {
                flush!();
                out.push(Span::styled(text, S_ITALIC));
                i += skip;
                continue;
            }
        }

        plain.push(chars[i]);
        i += 1;
    }

    flush!();
    if out.is_empty() {
        out.push(Span::styled(String::new(), S_NORMAL));
    }
    out
}

// ── Inline parsers ───────────────────────────────────────────────────────────

/// Try to parse `[text](url)` at the start of `chars`. Returns (text, chars_consumed).
fn try_parse_link(chars: &[char]) -> Option<(String, usize)> {
    if chars[0] != '[' { return None; }
    let close_bracket = chars.iter().position(|&c| c == ']')?;
    let text: String = chars[1..close_bracket].iter().collect();
    // Expect `(` right after `]`
    if chars.get(close_bracket + 1) != Some(&'(') { return None; }
    let open_paren = close_bracket + 1;
    let close_paren = chars[open_paren..].iter().position(|&c| c == ')')? + open_paren;
    Some((text, close_paren + 1))
}

/// Try to parse a single-char delimited span (e.g. backtick code).
fn try_parse_delimited(chars: &[char], open: char, close: char) -> Option<(String, usize)> {
    if chars[0] != open { return None; }
    let end = chars[1..].iter().position(|&c| c == close)? + 1;
    let text: String = chars[1..end].iter().collect();
    Some((text, end + 1))
}

/// Try to parse a multi-char marker delimited span (`**`, `***`, `_`, `__`).
fn try_parse_multi(chars: &[char], marker: &str) -> Option<(String, usize)> {
    let m: Vec<char> = marker.chars().collect();
    let ml = m.len();
    if chars.len() < ml * 2 { return None; }
    // Verify opening marker.
    if &chars[..ml] != m.as_slice() { return None; }
    // Find closing marker (not overlapping opening).
    let mut j = ml;
    while j + ml <= chars.len() {
        if &chars[j..j+ml] == m.as_slice() {
            // Ensure we're not just matching the open marker again at position 0.
            if j > 0 {
                let text: String = chars[ml..j].iter().collect();
                return Some((text, j + ml));
            }
        }
        j += 1;
    }
    None
}

// ============================================================================
// Word wrap (preserves existing newlines)
// ============================================================================

fn word_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 { return vec![String::new()]; }
    let mut out = Vec::new();
    for raw_line in text.split('\n') {
        if raw_line.is_empty() { out.push(String::new()); continue; }
        let mut cur = String::new();
        for word in raw_line.split_whitespace() {
            let wl = word.chars().count();
            if wl > width {
                if !cur.is_empty() { out.push(std::mem::take(&mut cur)); }
                let mut chunk = String::new();
                for ch in word.chars() {
                    if chunk.chars().count() >= width { out.push(std::mem::take(&mut chunk)); }
                    chunk.push(ch);
                }
                cur = chunk;
            } else if cur.is_empty() {
                cur.push_str(word);
            } else if cur.chars().count() + 1 + wl <= width {
                cur.push(' '); cur.push_str(word);
            } else {
                out.push(std::mem::take(&mut cur));
                cur.push_str(word);
            }
        }
        if !cur.is_empty() { out.push(cur); }
    }
    if out.is_empty() { out.push(String::new()); }
    out
}