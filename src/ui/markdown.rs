//! Markdown → ratatui `Line` renderer using `markdown` crate.
//!
//! Uses the `markdown` crate (wooorm/markdown-rs) for parsing,
//! with a custom tree walker to convert AST to ratatui styled lines.

use markdown::mdast::{Node, Text, ListItem};
use markdown::{ParseOptions, to_mdast};

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::ui::utils::word_wrap;

// ============================================================================
// Theme Styles
// ============================================================================

const S_NORMAL:     Style = Style::new().fg(Color::White);
const S_H1:         Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const S_H2:         Style = Style::new().fg(Color::LightCyan).add_modifier(Modifier::BOLD);
const S_H3:         Style = Style::new().fg(Color::Cyan);
const S_BOLD:       Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
const S_ITALIC:     Style = Style::new().fg(Color::White).add_modifier(Modifier::ITALIC);
const S_BOLD_ITAL:  Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD).add_modifier(Modifier::ITALIC);
const S_CODE:       Style = Style::new().fg(Color::Yellow);
const S_CODE_BG:    Style = Style::new().fg(Color::Yellow).bg(Color::Black);
const S_BLOCKQUOTE: Style = Style::new().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
const S_THINKING:   Style = Style::new().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
const S_LIST_BULLET:Style = Style::new().fg(Color::Cyan);
const S_RULE:       Style = Style::new().fg(Color::DarkGray);
const S_LINK_TEXT:  Style = Style::new().fg(Color::LightBlue).add_modifier(Modifier::UNDERLINED);

// ============================================================================
// Public API
// ============================================================================

/// Convert a markdown string into a vector of `Line`s, word-wrapped to `width`.
pub fn render_markdown(md: &str, width: usize) -> Vec<Line> {
    let mut out = Vec::new();
    let mut rest = md;
    
    // Handle thinking blocks first (non-standard)
    while !rest.is_empty() {
        if let Some(open) = rest.find("<thinking>") {
            if open > 0 {
                render_normal_segment(&rest[..open], width, &mut out);
            }
            let after_open = &rest[open + "<thinking>".len()..];
            if let Some(close) = after_open.find("</thinking>") {
                render_thinking_segment(&after_open[..close], &mut out);
                rest = &after_open[close + "</thinking>".len()..];
            } else {
                render_thinking_segment(after_open, &mut out);
                rest = "";
            }
        } else {
            render_normal_segment(rest, width, &mut out);
            rest = "";
        }
    }
    
    out
}

fn render_thinking_segment(content: &str, out: &mut Vec<Line>) {
    for raw_line in content.lines() {
        let t = raw_line.trim_end();
        if !t.is_empty() {
            out.push(Line::from(Span::styled(t.to_string(), S_THINKING)));
        }
    }
    if !content.trim().is_empty() {
        out.push(Line::default());
    }
}

fn render_normal_segment(content: &str, width: usize, out: &mut Vec<Line>) {
    let ast = to_mdast(content, &ParseOptions::default()).unwrap_or_else(|_| {
        // Fallback: create a root with the raw content as text
        Node::Root(markdown::mdast::Root {
            children: vec![Node::Text(Text { value: content.to_string(), position: None })],
            position: None,
        })
    });
    
    walk_node(&ast, width, out);
}

fn walk_node(node: &Node, width: usize, out: &mut Vec<Line>) {
    // Get children from Root
    if let Some(children) = node.children() {
        for child in children {
            walk_block(child, width, out);
        }
    }
}

fn walk_block(node: &Node, width: usize, out: &mut Vec<Line>) {
    match node {
        Node::Heading(heading) => {
            let text = get_node_text(node);
            let (prefix, style) = match heading.depth {
                1 => ("█ ", S_H1),
                2 => ("▌ ", S_H2),
                _ => ("░ ", S_H3),
            };
            let spans = inline_spans_from_node(&text.to_string());
            let mut result = vec![Span::styled(prefix, style)];
            result.extend(spans);
            out.push(Line::from(result));
        }
        
        Node::Code(code) => {
            out.push(Line::from(vec![
                Span::styled("╭─ ".to_string(), S_RULE),
                Span::styled("code".to_string(), S_CODE),
                Span::styled(" ─".to_string(), S_RULE),
            ]));
            for line in code.value.lines() {
                out.push(Line::from(Span::styled(format!("│ {}", line), S_CODE_BG)));
            }
            out.push(Line::from(Span::styled("╰─".to_string(), S_RULE)));
        }
        
        Node::ThematicBreak(_) => {
            out.push(Line::from(Span::styled("─".repeat(width.min(80)), S_RULE)));
        }
        
        Node::Blockquote(_) => {
            let text = get_node_text(node);
            let inner_w = width.saturating_sub(3);
            for (i, chunk) in word_wrap(&text, inner_w).into_iter().enumerate() {
                let bar = if i == 0 { "▌ " } else { "  " };
                let spans = inline_spans_from_node(chunk.to_string());
                let styled: Vec<Span> = spans
                    .into_iter()
                    .map(|s| {
                        if s.style == S_NORMAL {
                            Span::styled(s.content, S_BLOCKQUOTE)
                        } else {
                            s
                        }
                    })
                    .collect();
                let mut result = vec![Span::styled(bar.to_string(), S_LIST_BULLET)];
                result.extend(styled);
                out.push(Line::from(result));
            }
        }
        
        Node::List(list) => {
            for (i, item) in list.children.iter().enumerate() {
                if let Node::ListItem(li) = item {
                    let num = if list.ordered { list.start.unwrap_or(1) as usize + i } else { 0 };
                    walk_list_item(li, list.ordered, num, width, out);
                }
            }
        }
        
        Node::Paragraph(_) => {
            let text = get_node_text(node);
            let inner_w = width.saturating_sub(2);
            for chunk in word_wrap(&text, inner_w) {
                out.push(Line::from(inline_spans_from_node(chunk.to_string())));
            }
        }
        
        Node::Text(t) => {
            if !t.value.is_empty() {
                out.push(Line::from(inline_spans_from_node(t.value.clone())));
            }
        }
        
        Node::Break(_) => {
            out.push(Line::default());
        }
        
        Node::Table(_) | Node::TableRow(_) | Node::TableCell(_) => {
            // Tables not supported - skip
        }
        
        Node::FootnoteDefinition(_) | Node::FootnoteReference(_) => {
            // Footnotes not supported - skip
        }
        
        Node::Html(_) => {
            // Skip HTML
        }
        
        Node::Definition(_) => {
            // Skip definitions
        }
        
        Node::Yaml(_) | Node::Toml(_) => {
            // Skip frontmatter
        }
        
        _ => {
            // Try to get any text content
            let text = get_node_text(node);
            if !text.is_empty() {
                for chunk in word_wrap(&text, width.saturating_sub(2)) {
                    out.push(Line::from(inline_spans_from_node(chunk.to_string())));
                }
            }
        }
    }
}

fn walk_list_item(item: &ListItem, ordered: bool, index: usize, width: usize, out: &mut Vec<Line>) {
    let text = get_item_text(item);
    let inner_w = width.saturating_sub(5);
    
    for (i, chunk) in word_wrap(&text, inner_w).into_iter().enumerate() {
        let prefix = if i == 0 {
            if ordered {
                format!("{}. ", index)
            } else {
                "  • ".to_string()
            }
        } else {
            "    ".to_string()
        };
        let mut spans = vec![Span::styled(prefix, S_LIST_BULLET)];
        spans.extend(inline_spans_from_node(chunk.to_string()));
        out.push(Line::from(spans));
    }
}

// ============================================================================
// Text extraction
// ============================================================================

fn get_node_text(node: &Node) -> String {
    match node {
        Node::Text(t) => t.value.clone(),
        Node::Heading(h) => h.children.iter().map(|c| get_node_text(c)).collect(),
        Node::Paragraph(p) => p.children.iter().map(|c| get_node_text(c)).collect(),
        Node::Blockquote(bq) => bq.children.iter().map(|c| get_node_text(c)).collect(),
        Node::Code(c) => c.value.clone(),
        Node::List(l) => l.children.iter().filter_map(|i| {
            if let Node::ListItem(li) = i { Some(get_item_text(li)) } else { None }
        }).collect::<Vec<_>>().join(" "),
        Node::Emphasis(e) => e.children.iter().map(|c| get_node_text(c)).collect(),
        Node::Strong(s) => s.children.iter().map(|c| get_node_text(c)).collect(),
        Node::Delete(d) => d.children.iter().map(|c| get_node_text(c)).collect(),
        Node::InlineCode(ic) => ic.value.clone(),
        Node::Link(l) => l.children.iter().map(|c| get_node_text(c)).collect(),
        Node::Image(i) => i.alt.clone(),
        _ => String::new(),
    }
}

fn get_item_text(item: &ListItem) -> String {
    item.children.iter().map(|c| get_node_text(c)).collect::<Vec<_>>().join(" ")
}

// ============================================================================
// Inline rendering
// ============================================================================

fn inline_spans_from_node(text: impl Into<String>) -> Vec<Span<'static>> {
    let text = text.into();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut out = Vec::new();
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
        // Inline code: `…`
        if chars[i] == '`' {
            if let Some((code, skip)) = try_parse_delimited(&chars[i..], '`', '`') {
                flush!();
                out.push(Span::styled(code, S_CODE));
                i += skip;
                continue;
            }
        }
        
        // Link: [text](url)
        if chars[i] == '[' {
            if let Some((text, skip)) = try_parse_link(&chars[i..]) {
                flush!();
                out.push(Span::styled(text, S_LINK_TEXT));
                i += skip;
                continue;
            }
        }
        
        // Bold-italic: ***…***
        if i + 2 < len && chars[i] == '*' && chars[i+1] == '*' && chars[i+2] == '*' {
            if let Some((text, skip)) = try_parse_multi(&chars[i..], "***") {
                flush!();
                out.push(Span::styled(text, S_BOLD_ITAL));
                i += skip;
                continue;
            }
        }
        
        // Bold: **…** or __…__
        if i + 1 < len && ((chars[i] == '*' && chars[i+1] == '*') || (chars[i] == '_' && chars[i+1] == '_')) {
            let marker = if chars[i] == '*' { "**" } else { "__" };
            if let Some((text, skip)) = try_parse_multi(&chars[i..], marker) {
                flush!();
                out.push(Span::styled(text, S_BOLD));
                i += skip;
                continue;
            }
        }
        
        // Italic: *…* or _…_
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

// ============================================================================
// Inline parsers
// ============================================================================

fn try_parse_link(chars: &[char]) -> Option<(String, usize)> {
    if chars.is_empty() || chars[0] != '[' { return None; }
    let close_bracket = chars[1..].iter().position(|&c| c == ']')? + 1;
    let text: String = chars[1..close_bracket].iter().collect();
    if close_bracket + 1 >= chars.len() || chars[close_bracket + 1] != '(' { return None; }
    let open_paren = close_bracket + 1;
    let close_paren = chars[open_paren + 1..].iter().position(|&c| c == ')')? + open_paren + 1;
    Some((text, close_paren + 1))
}

fn try_parse_delimited(chars: &[char], open: char, close: char) -> Option<(String, usize)> {
    if chars.is_empty() || chars[0] != open { return None; }
    let end = chars[1..].iter().position(|&c| c == close)? + 1;
    let text: String = chars[1..end].iter().collect();
    Some((text, end + 1))
}

fn try_parse_multi(chars: &[char], marker: &str) -> Option<(String, usize)> {
    let m: Vec<char> = marker.chars().collect();
    let ml = m.len();
    if chars.len() < ml * 2 { return None; }
    if &chars[..ml] != m.as_slice() { return None; }
    let mut j = ml;
    while j + ml <= chars.len() {
        if &chars[j..j + ml] == m.as_slice() && j > 0 {
            let text: String = chars[ml..j].iter().collect();
            return Some((text, j + ml));
        }
        j += 1;
    }
    None
}