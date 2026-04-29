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
use crate::ui::theme;

// ============================================================================
// Constants
// ============================================================================

/// Prefix for user messages.
const USER_PREFIX: &str = "    ";

/// Prefix for assistant messages (indentation).
const ASSISTANT_INDENT: &str = "    ";

/// Prefix for tool call messages.
const TOOL_CALL_PREFIX: &str = "  🛠 ";

/// Prefix for tool result messages.
const TOOL_RESULT_PREFIX: &str = "  ✔ ";

/// Prefix for system messages.
const SYSTEM_PREFIX: &str = "  ⚑ ";

/// Indentation for wrapped system message lines.
const SYSTEM_INDENT: &str = "      ";

// ============================================================================
// Widget Definition
// ============================================================================

/// Chat panel widget that renders the message history.
pub struct ChatPanel<'a> {
    pub app: &'a App,
}

impl<'a> Widget for ChatPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Render panel border and title.
        let block = Block::default()
            .title(" CHAT ")
            .borders(Borders::ALL)
            .border_style(theme::BORDER);

        let inner = block.inner(area);
        block.render(area, buf);

        // Calculate available width for text wrapping.
        let width = inner.width.max(1) as usize;
        let mut lines: Vec<Line> = Vec::new();

        // Render each message based on its kind.
        for msg in &self.app.messages {
            render_message(msg, self.app, width, &mut lines);
        }

        // Apply scrolling offset.
        let total_lines = lines.len() as u16;
        let visible = inner.height;
        let max_scroll = total_lines.saturating_sub(visible);
        let scroll = (self.app.scroll as u16).min(max_scroll);

        Paragraph::new(lines).scroll((scroll, 0)).render(inner, buf);
    }
}

// ============================================================================
// Private Helpers
// ============================================================================

/// Renders a single message based on its kind.
fn render_message<'a>(msg: &'a crate::state::ChatMessage, _app: &'a crate::state::App, width: usize, out: &mut Vec<Line<'a>>) {
    match msg.kind {
        MsgKind::User => render_user_message(&msg.content, width, out),
        MsgKind::Assistant => render_assistant_message(&msg.content, width, out),
        MsgKind::ToolCall => render_tool_call(&msg.content, out),
        MsgKind::ToolResult => render_tool_result(&msg.content, out),
        MsgKind::System => render_system_message(&msg.content, width, out),
        MsgKind::RateLimit => {}, // handled in animation area
    }
}

/// Renders a user message with bubble styling.
fn render_user_message(content: &str, width: usize, out: &mut Vec<Line>) {
    out.push(Line::default());

    let inner_content_width = width.saturating_sub(USER_PREFIX.chars().count() + 4);
    let wrapped = wrap_text(content, inner_content_width);

    let border_color = theme::BUBBLE_USER_BORDER.fg.unwrap_or(Color::Cyan);

    out.push(Line::from(vec![
        Span::styled("┌─", Style::new().fg(border_color)),
        Span::styled(" ".repeat(width.saturating_sub(2)), theme::DIM),
    ]));

    for (i, chunk) in wrapped.into_iter().enumerate() {
        let prefix = if i == 0 { USER_PREFIX } else { "│ " };
        out.push(Line::from(vec![
            Span::styled("│ ", Style::new().fg(border_color)),
            Span::styled(prefix, theme::ACCENT.add_modifier(Modifier::BOLD)),
            Span::styled(chunk, theme::USER),
        ]));
    }

    out.push(Line::from(vec![
        Span::styled("└─", Style::new().fg(border_color)),
        Span::styled(" ".repeat(width.saturating_sub(2)), theme::DIM),
    ]));
}

/// Renders assistant message with bubble styling.
fn render_assistant_message(content: &str, width: usize, out: &mut Vec<Line>) {
    out.push(Line::default());

    let inner_content_width = width.saturating_sub(ASSISTANT_INDENT.chars().count() + 4);
    let wrapped = wrap_text(content, inner_content_width);

    let border_color = theme::BUBBLE_ASSISTANT_BORDER.fg.unwrap_or(Color::Green);

    out.push(Line::from(vec![
        Span::styled("┌─", Style::new().fg(border_color)),
        Span::styled(" ".repeat(width.saturating_sub(2)), theme::DIM),
    ]));

    for (i, chunk) in wrapped.into_iter().enumerate() {
        let prefix = if i == 0 { ASSISTANT_INDENT } else { "│   " };
        out.push(Line::from(vec![
            Span::styled("│ ", Style::new().fg(border_color)),
            Span::styled(prefix, theme::DIM),
            Span::styled(chunk, theme::ASSISTANT),
        ]));
    }

    out.push(Line::from(vec![
        Span::styled("└─", Style::new().fg(border_color)),
        Span::styled(" ".repeat(width.saturating_sub(2)), theme::DIM),
    ]));
}

/// Renders a tool call message.
fn render_tool_call<'a>(content: &'a str, out: &mut Vec<Line<'a>>) {
    out.push(Line::from(vec![
        Span::styled(TOOL_CALL_PREFIX, theme::TOOL_CALL),
        Span::styled(content, theme::TOOL_CALL),
    ]));
}

/// Renders a tool result message.
fn render_tool_result<'a>(content: &'a str, out: &mut Vec<Line<'a>>) {
    out.push(Line::from(vec![
        Span::styled(TOOL_RESULT_PREFIX, theme::TOOL_RESULT),
        Span::styled(content, theme::TOOL_RESULT),
    ]));
}

/// Renders a system message (errors, warnings, etc.).
fn render_system_message(content: &str, width: usize, out: &mut Vec<Line>) {
    let content_width = width.saturating_sub(SYSTEM_PREFIX.chars().count());
    for (i, chunk) in wrap_text(content, content_width).into_iter().enumerate() {
        if i == 0 {
            out.push(Line::from(vec![
                Span::styled(SYSTEM_PREFIX, theme::ERROR),
                Span::styled(chunk, theme::SYSTEM),
            ]));
        } else {
            out.push(Line::from(vec![
                Span::raw(SYSTEM_INDENT),
                Span::styled(chunk, theme::SYSTEM),
            ]));
        }
    }
}

/// Word-wrap text to fit within the given width.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }

    let mut out = Vec::new();

    for raw_line in text.split('\n') {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }

        let mut current = String::new();

        for word in raw_line.split_whitespace() {
            let word_len = word.chars().count();

            // Handle words longer than width by breaking character-by-character.
            if word_len > width {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }

                let mut chunk = String::new();
                for ch in word.chars() {
                    if chunk.chars().count() >= width {
                        out.push(std::mem::take(&mut chunk));
                    }
                    chunk.push(ch);
                }
                if !chunk.is_empty() {
                    current = chunk;
                }
                continue;
            }

            // Fit word into current line or start new one.
            if current.is_empty() {
                current.push_str(word);
            } else if current.chars().count() + 1 + word_len <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                out.push(std::mem::take(&mut current));
                current.push_str(word);
            }
        }

        if !current.is_empty() {
            out.push(current);
        }
    }

    if out.is_empty() {
        out.push(String::new());
    }

    out
}