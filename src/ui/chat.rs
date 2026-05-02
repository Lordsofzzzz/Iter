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

        // Use line_count() which accounts for text wrapping.
        let para = Paragraph::new(lines.clone());
        let total_lines = para.line_count(inner.width) as usize;
        let visible     = inner.height as usize;
        let max_scroll  = total_lines.saturating_sub(visible);

        // Write back so scroll_up/down can clamp correctly.
        self.app.scroll_max = max_scroll;
        let scroll = (self.app.scroll.min(max_scroll)) as u16;

        para.scroll((scroll, 0)).render(inner, buf);
    }
}

// ============================================================================
// Message dispatch
// ============================================================================

fn render_message(msg: &crate::state::ChatMessage, width: usize, out: &mut Vec<Line>) {
    match msg.kind {
        MsgKind::User      => render_user_bubble(&msg.content, width, out),
        MsgKind::Assistant => render_assistant_bubble(msg, width, out),
        MsgKind::ToolCall  => render_tool_call(&msg.content, out),
        MsgKind::ToolResult=> render_tool_result(&msg.content, out),
        MsgKind::System    => render_system_message(&msg.content, width, out),
        MsgKind::RateLimit => {},
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

    // Render thinking block first if present — no green bar, italic gray.
    if !msg.thinking.trim().is_empty() {
        out.push(Line::from(vec![
            Span::styled("  ~ thinking", Style::new().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
        ]));
        let max_lines = 5;
        let inner_w = inner_w.saturating_sub(2);
        for raw_line in msg.thinking.lines().take(max_lines) {
            let trimmed = raw_line.trim_end();
            if !trimmed.is_empty() {
                let display = if trimmed.len() > inner_w { format!("{}…", &trimmed[..inner_w]) } else { trimmed.to_string() };
                out.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(display, Style::new().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
                ]));
            }
        }
        if msg.thinking.lines().count() > max_lines {
            out.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("…", Style::new().fg(Color::DarkGray)),
            ]));
        }
        out.push(Line::default()); // blank separator before response
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

fn render_tool_call(content: &str, out: &mut Vec<Line>) {
    let display = if content.len() > 200 { format!("{}...", &content[..200]) } else { content.to_string() };
    out.push(Line::from(vec![
        Span::styled(TOOL_CALL_PREFIX, theme::TOOL_CALL),
        Span::styled(display, theme::TOOL_CALL),
    ]));
}

fn render_tool_result(content: &str, out: &mut Vec<Line>) {
    let max_lines = 8;
    let max_line_len = 160;
    let lines: Vec<&str> = content.lines().take(max_lines).collect();
    for (i, line) in lines.iter().enumerate() {
        let display = if line.len() > max_line_len {
            format!("{}…", &line[..max_line_len])
        } else {
            line.to_string()
        };
        if i == 0 {
            out.push(Line::from(vec![
                Span::styled(TOOL_RESULT_PREFIX, theme::TOOL_RESULT),
                Span::styled(display, theme::TOOL_RESULT),
            ]));
        } else {
            out.push(Line::from(vec![
                Span::raw("       "),
                Span::styled(display, theme::TOOL_RESULT),
            ]));
        }
    }
    if content.lines().count() > max_lines {
        out.push(Line::from(vec![
            Span::raw("       "),
            Span::styled("… (truncated)", theme::DIM),
        ]));
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