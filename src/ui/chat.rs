//! Chat panel widget - renders message history.
//!
//! Displays user messages, assistant responses, tool calls/results,
//! system errors, and rate limit status with appropriate styling.

use std::time::Instant;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::state::{App, MsgKind};
use crate::ui::theme;

// ============================================================================
// Constants
// ============================================================================

/// Prefix for user messages.
const USER_PREFIX: &str = "  ❯ ";

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

/// Cursor character shown during streaming.
const STREAMING_CURSOR: &str = "▋";

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

        // Show streaming cursor or breathing animation if agent is responding.
        if self.app.streaming {
            let has_assistant_msg = self.app.messages.iter().any(|m| matches!(m.kind, MsgKind::Assistant));

            if has_assistant_msg {
                // Show cursor at end of existing text
                lines.push(Line::from(vec![
                    Span::raw(ASSISTANT_INDENT),
                    Span::styled(STREAMING_CURSOR, theme::ACCENT),
                ]));
            } else {
                // Show breathing skeleton animation (no content yet)
                if let Some(elapsed_ms) = self.app.streaming_elapsed_ms() {
                    // Show 3-line wave effect
                    let frame = get_breathing_frame(elapsed_ms);
                    let (char1, status1) = frame;
                    let (_, status2) = get_breathing_frame(elapsed_ms.saturating_sub(150));
                    let (_, status3) = get_breathing_frame(elapsed_ms.saturating_sub(300));

                    lines.push(Line::from(vec![
                        Span::raw(ASSISTANT_INDENT),
                        Span::styled(format!("{} {}...", char1, status1), theme::ACCENT),
                    ]));
                    lines.push(Line::from(vec![
                        Span::raw(ASSISTANT_INDENT),
                        Span::styled(format!("  {}...", status2), theme::DIM),
                    ]));
                    lines.push(Line::from(vec![
                        Span::raw(ASSISTANT_INDENT),
                        Span::styled(format!("  {}...", status3), theme::DIM),
                    ]));
                }
            }
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
fn render_message<'a>(msg: &'a crate::state::ChatMessage, app: &'a crate::state::App, width: usize, out: &mut Vec<Line<'a>>) {
    match msg.kind {
        MsgKind::User => render_user_message(&msg.content, width, out),
        MsgKind::Assistant => render_assistant_message(&msg.content, width, out),
        MsgKind::ToolCall => render_tool_call(&msg.content, out),
        MsgKind::ToolResult => render_tool_result(&msg.content, out),
        MsgKind::System => render_system_message(&msg.content, width, out),
        MsgKind::RateLimit => render_rate_limit(app, out),
    }
}

/// Renders a user message with input prefix.
fn render_user_message(content: &str, width: usize, out: &mut Vec<Line>) {
    out.push(Line::default()); // Empty line before user message.

    let content_width = width.saturating_sub(USER_PREFIX.chars().count());
    for (i, chunk) in wrap_text(content, content_width).into_iter().enumerate() {
        if i == 0 {
            out.push(Line::from(vec![
                Span::styled(USER_PREFIX, theme::ACCENT.add_modifier(Modifier::BOLD)),
                Span::styled(chunk, theme::USER),
            ]));
        } else {
            out.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(chunk, theme::USER),
            ]));
        }
    }
}

/// Renders assistant message with indentation.
fn render_assistant_message(content: &str, width: usize, out: &mut Vec<Line>) {
    let content_width = width.saturating_sub(ASSISTANT_INDENT.chars().count());
    for chunk in wrap_text(content, content_width) {
        out.push(Line::from(vec![
            Span::raw(ASSISTANT_INDENT),
            Span::styled(chunk, theme::ASSISTANT),
        ]));
    }
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

/// Renders rate limit message with countdown.
fn render_rate_limit(app: &crate::state::App, out: &mut Vec<Line>) {
    let now = Instant::now();

    let (remaining_ms, retries_left) = if let Some(dl) = app.cooldown_deadline {
        let rem = dl.saturating_duration_since(now).as_millis() as u64;
        (rem, app.cooldown_retries_left)
    } else {
        (0, 0)
    };

    let secs = (remaining_ms + 999) / 1000; // ceil

    let label = if secs > 0 {
        format!(
            "  rate limited [{}]  ({} left)",
            secs,
            retries_left,
        )
    } else {
        format!("  rate limited [·]  ({} left)", retries_left)
    };

    out.push(Line::from(vec![
        Span::styled(label, theme::WARNING),
    ]));
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

/// Returns a breathing animation frame based on elapsed time.
/// Cycles through box-drawing characters with status text.
fn get_breathing_frame(elapsed_ms: u64) -> (char, &'static str) {
    const FRAMES: [(char, &str); 12] = [
        ('▖', "thinking"),
        ('▗', "thinking"),
        ('▘', "processing"),
        ('▙', "processing"),
        ('▚', "generating"),
        ('▛', "generating"),
        ('▜', "computing"),
        ('▝', "computing"),
        ('▞', "creating"),
        ('▟', "creating"),
        ('▧', "finalizing"),
        ('▦', "finalizing"),
    ];
    let idx = (elapsed_ms / 150) as usize % FRAMES.len();
    FRAMES[idx]
}