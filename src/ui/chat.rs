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

pub struct ChatPanel<'a> {
    pub app: &'a App,
}

impl<'a> Widget for ChatPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" CHAT ")
            .borders(Borders::ALL)
            .border_style(theme::BORDER);

        let inner = block.inner(area);
        block.render(area, buf);

        let width = inner.width.max(1) as usize;

        let mut lines: Vec<Line> = Vec::new();

        for msg in &self.app.messages {
            match msg.kind {
                MsgKind::User => {
                    lines.push(Line::default());
                    let prefix = "  ❯ ";
                    let content_width = width.saturating_sub(prefix.chars().count());
                    for (i, chunk) in wrap_text(&msg.content, content_width).into_iter().enumerate() {
                        let styled = if i == 0 { theme::USER } else { theme::USER };
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled(prefix, theme::ACCENT.add_modifier(Modifier::BOLD)),
                                Span::styled(chunk, styled),
                            ]));
                        } else {
                            lines.push(Line::from(vec![
                                Span::raw("    "),
                                Span::styled(chunk, styled),
                            ]));
                        }
                    }
                }
                MsgKind::Assistant => {
                    let content_width = width.saturating_sub(4);
                    for chunk in wrap_text(&msg.content, content_width) {
                        lines.push(Line::from(vec![
                            Span::raw("    "),
                            Span::styled(chunk, theme::ASSISTANT),
                        ]));
                    }
                }
                MsgKind::ToolCall => {
                    lines.push(Line::from(vec![
                        Span::styled("  🛠 ", theme::TOOL_CALL),
                        Span::styled(msg.content.as_str(), theme::TOOL_CALL),
                    ]));
                }
                MsgKind::ToolResult => {
                    lines.push(Line::from(vec![
                        Span::styled("  ✔ ", theme::TOOL_RESULT),
                        Span::styled(msg.content.as_str(), theme::TOOL_RESULT),
                    ]));
                }
                MsgKind::System => {
                    let content_width = width.saturating_sub(6);
                    for (i, chunk) in wrap_text(&msg.content, content_width).into_iter().enumerate() {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled("  ⚑ ", theme::ERROR),
                                Span::styled(chunk, theme::SYSTEM),
                            ]));
                        } else {
                            lines.push(Line::from(vec![
                                Span::raw("      "),
                                Span::styled(chunk, theme::SYSTEM),
                            ]));
                        }
                    }
                }
                MsgKind::RateLimit => {
                    let now = Instant::now();

                    let (remaining_ms, retries_left) = if let Some(dl) = self.app.cooldown_deadline {
                        let rem = dl.saturating_duration_since(now).as_millis() as u64;
                        (rem, self.app.cooldown_retries_left)
                    } else {
                        (0, 0)
                    };
                    let secs = (remaining_ms + 999) / 1000; // ceil

                    // Countdown: [3] [2] [1] [ ] cycling at 1s intervals.
                    let label = if secs > 0 {
                        format!(
                            "  rate limited [{}]  ({} left)",
                            secs,
                            retries_left,
                        )
                    } else {
                        format!("  rate limited [·]  ({} left)", retries_left)
                    };

                    lines.push(Line::from(vec![
                        Span::styled(label, theme::WARNING),
                    ]));
                }
            }
        }

        if self.app.streaming {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("▋", theme::ACCENT),
            ]));
        }

        let total_lines = lines.len() as u16;
        let visible = inner.height;
        let max_scroll = total_lines.saturating_sub(visible);
        let scroll = (self.app.scroll as u16).min(max_scroll);

        Paragraph::new(lines).scroll((scroll, 0)).render(inner, buf);
    }
}

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