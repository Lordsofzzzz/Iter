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

const TOOL_CALL_PREFIX:   &str = "  🛠  ";
const TOOL_RESULT_PREFIX: &str = "  ✔  ";
const SYSTEM_PREFIX:      &str = "  ⚑  ";
const SYSTEM_INDENT:      &str = "       ";

// ============================================================================
// Widget Definition
// ============================================================================

pub struct ChatPanel<'a> {
    pub app: &'a App,
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

        let total_lines = lines.len() as u16;
        let visible     = inner.height;
        let max_scroll  = total_lines.saturating_sub(visible);
        let scroll      = (self.app.scroll as u16).min(max_scroll);

        Paragraph::new(lines).scroll((scroll, 0)).render(inner, buf);
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
        let thinking_style = Style::new()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC);
        for raw_line in msg.thinking.lines() {
            let t = raw_line.trim_end();
            if !t.is_empty() {
                out.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(t.to_string(), thinking_style),
                ]));
            }
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
    out.push(Line::from(vec![
        Span::styled(TOOL_CALL_PREFIX, theme::TOOL_CALL),
        Span::styled(content.to_string(), theme::TOOL_CALL),
    ]));
}

fn render_tool_result(content: &str, out: &mut Vec<Line>) {
    out.push(Line::from(vec![
        Span::styled(TOOL_RESULT_PREFIX, theme::TOOL_RESULT),
        Span::styled(content.to_string(), theme::TOOL_RESULT),
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
