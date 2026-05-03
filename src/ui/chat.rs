//! Chat panel widget - renders message history.
//!
//! Displays user messages, assistant responses, tool calls/results,
//! and system errors with appropriate styling.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::state::{App, MsgKind};
use crate::ui::utils::word_wrap;
use crate::ui::{markdown::render_markdown, theme};

// ============================================================================
// Constants
// ============================================================================

const TOOL_ARROW:    &str = "→ ";
const TOOL_RESULT_PREFIX: &str = "  └─ ";
const SYSTEM_PREFIX: &str = "  !! ";
const SYSTEM_INDENT: &str = "      ";

// ============================================================================
// Widget Definition
// ============================================================================

pub struct ChatPanel<'a> {
    pub app: &'a mut App,
}

impl<'a> Widget for ChatPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" CHAT ")
            .borders(Borders::ALL)
            .border_style(theme::BORDER)
            .style(Style::new().bg(theme::BG));

        let inner = block.inner(area);
        block.render(area, buf);

        let width = inner.width.max(1) as usize;
        let mut lines: Vec<Line> = Vec::new();

        for msg in &self.app.messages {
            render_message(msg, width, &mut lines);
        }

        let total_lines = lines.len();
        let visible     = inner.height as usize;
        let max_scroll  = total_lines.saturating_sub(visible);

        // Write back so scroll_up/down can clamp correctly.
        self.app.scroll_max = max_scroll;
        let scroll = (self.app.scroll.min(max_scroll)) as u16;

        Paragraph::new(lines).scroll((scroll, 0)).render(inner, buf);
    }
}

// ============================================================================
// Message dispatch
// ============================================================================

fn render_message(msg: &crate::state::ChatMessage, width: usize, out: &mut Vec<Line>) {
    match msg.kind {
        MsgKind::User       => render_user_bubble(&msg.content, width, out),
        MsgKind::Assistant  => render_assistant_bubble(msg, width, out),
        MsgKind::ToolCall   => {}  // merged into ToolResult line
        MsgKind::ToolResult => render_tool_result(msg, width, out),
        MsgKind::System     => render_system_message(&msg.content, width, out),
        MsgKind::RateLimit  => {},
    }
}

// ============================================================================
// User bubble  — cyan left bar, white bold text
// ============================================================================

fn render_user_bubble(content: &str, width: usize, out: &mut Vec<Line>) {
    let inner_w = width.saturating_sub(3); // "│ " = 2 chars + 1 space
    out.push(Line::default());
    for chunk in word_wrap(content, inner_w) {
        out.push(Line::from(vec![
            Span::styled("│ ", Style::new().fg(Color::Cyan)),
            Span::styled(chunk, Style::new().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]));
    }
}

// ============================================================================
// Assistant bubble — green left bar, markdown content
// ============================================================================

fn render_assistant_bubble(msg: &crate::state::ChatMessage, width: usize, out: &mut Vec<Line>) {
    let inner_w = width.saturating_sub(3);
    out.push(Line::default());

    // Render thinking block if present.
    // Skip if too short — avoids blank "~ " lines from tiny partial deltas.
    let thinking_trimmed = msg.thinking.trim();
    if thinking_trimmed.chars().count() >= 10 {
        let think_style = Style::new().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
        // Streaming: show raw thinking as single truncated line (no wrap).
        // Done: word-wrap first so streaming blobs get correctly chunked.
        let think_w = inner_w.saturating_sub(4);

        if msg.done {
            // Collapsed: word-wrap and take first non-empty chunk + ellipsis.
            let wrapped  = word_wrap(thinking_trimmed, think_w);
            let preview  = wrapped.iter().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
            let ellipsis = if wrapped.iter().filter(|l| !l.trim().is_empty()).count() > 1 { "…" } else { "" };
            out.push(Line::from(vec![
                Span::styled("Thinking: ", Style::new().fg(Color::DarkGray).add_modifier(Modifier::BOLD).add_modifier(Modifier::ITALIC)),
                Span::styled(format!("{preview}{ellipsis}"), think_style),
            ]));
        } else {
            // Streaming: single truncated line — no word-wrap while text is arriving
            // word by word, which would split each word onto its own line.
            let display: String = thinking_trimmed.chars().take(think_w).collect();
            let ellipsis = if thinking_trimmed.chars().count() > think_w { "…" } else { "" };
            out.push(Line::from(vec![
                Span::styled("Thinking: ", Style::new().fg(Color::DarkGray).add_modifier(Modifier::BOLD).add_modifier(Modifier::ITALIC)),
                Span::styled(format!("{display}{ellipsis}"), think_style),
            ]));
        }
        out.push(Line::default());
    }

    // Render response content with green bar.
    for line in render_markdown(&msg.content, inner_w) {
        let mut spans: Vec<Span<'static>> = vec![Span::styled("│ ", Style::new().fg(Color::Green))];
        spans.extend(line.spans.into_iter().map(|s| Span::styled(s.content.to_string(), s.style)));
        out.push(Line::from(spans));
    }
}

// ============================================================================
// Tool / system messages (no bubble)
// ============================================================================

fn render_tool_result(msg: &crate::state::ChatMessage, width: usize, out: &mut Vec<Line>) {
    let call_sig = &msg.thinking;
    let content  = &msg.content;

    let human = humanize_tool_call(call_sig);

    let arrow_len  = TOOL_ARROW.chars().count();
    let call_avail = width.saturating_sub(arrow_len);
    let call_disp: String = human.chars().take(call_avail).collect();
    out.push(Line::from(vec![
        Span::styled(TOOL_ARROW, theme::TOOL_CALL),
        Span::styled(call_disp, theme::TOOL_CALL),
    ]));

    let first_line = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("(no output)");
    let line_count = content.lines().filter(|l| !l.trim().is_empty()).count();
    let count_suf  = if line_count > 1 { format!(" (+{} lines)", line_count - 1) } else { String::new() };
    let pfx_len    = TOOL_RESULT_PREFIX.chars().count();
    let avail      = width.saturating_sub(pfx_len + count_suf.chars().count());
    let preview: String = first_line.chars().take(avail).collect();

    out.push(Line::from(vec![
        Span::styled(TOOL_RESULT_PREFIX, theme::DIM),
        Span::styled(preview, theme::TOOL_RESULT),
        Span::styled(count_suf, theme::DIM),
    ]));
}

fn humanize_tool_call(sig: &str) -> String {
    let (tool, args_raw) = match sig.find('(') {
        Some(i) => (&sig[..i], sig[i+1..].trim_end_matches(')')),
        None    => (sig, ""),
    };

    let extract = |key: &str| -> Option<String> {
        let needle = format!("\"{}\":", key);
        let start  = args_raw.find(&needle)? + needle.len();
        let rest   = args_raw[start..].trim_start();
        if rest.starts_with('"') {
            let inner = &rest[1..];
            let end   = inner.find('"')?;
            Some(inner[..end].to_string())
        } else {
            let end = rest.find(|c| c == ',' || c == '}').unwrap_or(rest.len());
            Some(rest[..end].trim().to_string())
        }
    };

    match tool {
        "run_command" => {
            let cmd = extract("cmd").unwrap_or_default();
            let short: String = cmd.chars().take(60).collect();
            let ellipsis = if cmd.chars().count() > 60 { "…" } else { "" };
            format!("Run {short}{ellipsis}")
        }
        "read_file" => {
            let path = extract("path").unwrap_or_default();
            format!("Read {path}")
        }
        "write_file" => {
            let path = extract("path").unwrap_or_default();
            format!("Write {path}")
        }
        "list_files" => {
            let path  = extract("path").unwrap_or(".".to_string());
            let depth = extract("depth").map(|d| format!(" (depth {d})")).unwrap_or_default();
            format!("List {path}{depth}")
        }
        "search_files" => {
            let pattern = extract("pattern").unwrap_or_default();
            let path    = extract("path").map(|p| format!(" in {p}")).unwrap_or_default();
            format!("Search {pattern}{path}")
        }
        _ => {
            let args_short: String = args_raw.chars().take(50).collect();
            format!("{tool} {args_short}")
        }
    }
}

fn render_system_message(content: &str, width: usize, out: &mut Vec<Line>) {
    let content_width = width.saturating_sub(SYSTEM_PREFIX.chars().count());
    for (i, chunk) in word_wrap(content, content_width).into_iter().enumerate() {
        if i == 0 {
            out.push(Line::from(vec![
                Span::styled(SYSTEM_PREFIX, theme::ERROR),
                Span::styled(chunk, theme::SYSTEM),
            ]));
        } else {
            out.push(Line::from(vec![
                Span::raw(SYSTEM_INDENT.to_string()),
                Span::styled(chunk, theme::SYSTEM),
            ]));
        }
    }
}