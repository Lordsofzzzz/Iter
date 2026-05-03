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

const TOOL_CALL_PREFIX:   &str = "  >> ";
const TOOL_RESULT_PREFIX: &str = "  ok ";
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
        MsgKind::ToolCall   => render_tool_call(msg, out),
        MsgKind::ToolResult => render_tool_result(msg, out),
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
    if !msg.thinking.trim().is_empty() {
        let think_style = Style::new().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
        let think_w     = inner_w.saturating_sub(4);

        if msg.done {
            // Collapsed: single preview line — pi style.
            let preview = msg.thinking
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            let truncated = if preview.len() > think_w {
                format!("{}…", &preview[..think_w])
            } else {
                preview.to_string()
            };
            out.push(Line::from(vec![
                Span::styled("  ~ ", think_style),
                Span::styled(truncated, think_style),
            ]));
        } else {
            // Streaming: show last 3 wrapped lines so it scrolls as it grows.
            let wrapped: Vec<String> = word_wrap(msg.thinking.trim(), think_w);
            let start = wrapped.len().saturating_sub(3);
            for (i, line) in wrapped[start..].iter().enumerate() {
                if i == 0 {
                    out.push(Line::from(vec![
                        Span::styled("  ~ ", think_style),
                        Span::styled(line.clone(), think_style),
                    ]));
                } else {
                    out.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(line.clone(), think_style),
                    ]));
                }
            }
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

fn render_tool_call(msg: &crate::state::ChatMessage, out: &mut Vec<Line>) {
    let content = &msg.content;
    let display = if content.len() > 200 { format!("{}...", &content[..200]) } else { content.clone() };
    let suffix  = if msg.done { "" } else { " ⋯" };
    out.push(Line::from(vec![
        Span::styled(TOOL_CALL_PREFIX, theme::TOOL_CALL),
        Span::styled(format!("{display}{suffix}"), theme::TOOL_CALL),
    ]));
}

fn render_tool_result(msg: &crate::state::ChatMessage, out: &mut Vec<Line>) {
    // msg.thinking holds the tool name (set in agent.rs).
    // msg.content holds the raw output.
    let tool_name = &msg.thinking;
    let content   = &msg.content;

    // Collapsed: single line with tool name + first line of output.
    let first_line = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("(no output)");
    let max_preview = 120;
    let preview = if first_line.len() > max_preview {
        format!("{}…", &first_line[..max_preview])
    } else {
        first_line.to_string()
    };
    let line_count = content.lines().count();
    let suffix = if line_count > 1 { format!("  (+{} lines)", line_count - 1) } else { String::new() };

    out.push(Line::from(vec![
        Span::styled(TOOL_RESULT_PREFIX, theme::TOOL_RESULT),
        Span::styled(format!("{tool_name}: {preview}"), theme::TOOL_RESULT),
        Span::styled(suffix, theme::DIM),
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