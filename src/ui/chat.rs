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

const TOOL_PREFIX: &str = ">> ";
const TOOL_RESULT_PREFIX: &str = "ok ";
const SYSTEM_PREFIX:      &str = "  !! ";
const SYSTEM_INDENT:      &str = "      ";

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
            // Collapsed: single preview line with ellipsis if needed.
            let wrapped  = word_wrap(thinking_trimmed, think_w);
            let preview  = wrapped.iter().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
            let total_chunks = wrapped.iter().filter(|l| !l.trim().is_empty()).count();
            let ellipsis = if total_chunks > 1 { "…" } else { "" };
            out.push(Line::from(vec![
                Span::styled("  ~ ", think_style),
                Span::styled(format!("{preview}{ellipsis}"), think_style),
            ]));
        } else {
            // Streaming: single truncated line — no word-wrap while text is arriving
            // word by word, which would split each word onto its own line.
            let display: String = thinking_trimmed.chars().take(think_w).collect();
            let ellipsis = if thinking_trimmed.chars().count() > think_w { "…" } else { "" };
            out.push(Line::from(vec![
                Span::styled("  ~ ", think_style),
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
    // msg.thinking = "tool_name(args)", msg.content = raw output.
    let call_sig   = &msg.thinking;  // e.g. "run_command({"cmd":"git status"})"
    let content    = &msg.content;
    let prefix_len = TOOL_PREFIX.chars().count();

    // First non-empty output line as result preview.
    let first_line = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("(no output)");
    let line_count = content.lines().filter(|l| !l.trim().is_empty()).count();
    let count_suf  = if line_count > 1 { format!("  (+{} lines)", line_count - 1) } else { String::new() };

    // Line 1: >> tool_name(args)
    let call_avail = width.saturating_sub(prefix_len);
    let call_wrapped = word_wrap(call_sig, call_avail);
    for (i, chunk) in call_wrapped.iter().enumerate() {
        if i == 0 {
            out.push(Line::from(vec![
                Span::styled(TOOL_PREFIX, theme::TOOL_CALL),
                Span::styled(chunk.clone(), theme::TOOL_CALL),
            ]));
        } else {
            out.push(Line::from(vec![
                Span::raw(" ".repeat(prefix_len)),
                Span::styled(chunk.clone(), theme::TOOL_CALL),
            ]));
        }
    }

    // Line 2: └─ first_line  (+N lines)
    let prefix     = "└─ ";
    let prefix_len = prefix.chars().count();
    let result_avail = width.saturating_sub(prefix_len + count_suf.chars().count());
    let preview: String = first_line.chars().take(result_avail).collect();

    out.push(Line::from(vec![
        Span::styled(prefix, theme::DIM),
        Span::styled(preview, theme::TOOL_RESULT),
        Span::styled(count_suf, theme::DIM),
    ]));
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