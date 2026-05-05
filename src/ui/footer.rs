//! Footer status line — Codex-style single row.
//!
//! Left:  [anim] model · ctx% · cost
//! Right: turns · tools · key hints
//! Width-responsive: drops hints → cost → tools as terminal narrows.

use std::time::Instant;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::state::{App, ModelStatus};
use crate::ui::theme;

// ── Animation frames ────────────────────────────────────────────────────────

const THINK_FRAMES: [char; 8] = ['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];
const COOLDOWN_FRAMES: [char; 4] = ['◐', '◓', '◑', '◒'];

fn thinking_frame(elapsed_ms: u64) -> char {
    THINK_FRAMES[(elapsed_ms / 80) as usize % THINK_FRAMES.len()]
}

fn cooldown_frame(elapsed_ms: u64) -> char {
    COOLDOWN_FRAMES[(elapsed_ms / 300) as usize % COOLDOWN_FRAMES.len()]
}

// ── Widget ───────────────────────────────────────────────────────────────────

pub struct Footer<'a> {
    pub app: &'a App,
}

impl<'a> Widget for Footer<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let width = area.width as usize;
        if width < 10 {
            return;
        }

        // ── Left side ────────────────────────────────────────────────────────
        let elapsed_ms = self.app.streaming_elapsed_ms().unwrap_or_else(|| {
            self.app
                .cooldown_started
                .map(|s| s.elapsed().as_millis() as u64)
                .unwrap_or(0)
        });

        // Status indicator char + style
        let (anim_char, anim_style) = match self.app.model_status {
            ModelStatus::Thinking => (
                thinking_frame(elapsed_ms),
                theme::ACCENT.add_modifier(Modifier::BOLD),
            ),
            ModelStatus::Cooldown => (
                cooldown_frame(elapsed_ms),
                theme::WARNING,
            ),
            ModelStatus::Error => ('✗', theme::ERROR),
            ModelStatus::Ready => ('·', theme::DIM),
        };

        // Model name — truncate to 24 chars
        let model_short = truncate_model_name(&self.app.model_name, 24);

        // Context %
        let ctx_color = theme::context_gauge_color(self.app.context_pct);
        let ctx_str = format!("{:.0}%", self.app.context_pct.min(100.0));

        // Cost
        let cost_str = if self.app.cost < 0.001 {
            format!("${:.5}", self.app.cost)
        } else {
            format!("${:.3}", self.app.cost)
        };

        // Cooldown detail
        let cooldown_str = self.app.cooldown_deadline.map(|d| {
            let secs = (d.saturating_duration_since(Instant::now()).as_millis() + 999) / 1000;
            format!(" rate limit {}s", secs)
        });

        // ── Right side ───────────────────────────────────────────────────────
        let hints = " esc·abort  ?·model";
        let turns_str = format!("{}t {}tools", self.app.turns, self.app.tool_calls);

        // ── Assemble left spans ──────────────────────────────────────────────
        let mut left: Vec<Span> = vec![
            Span::styled(format!(" {anim_char} "), anim_style),
            Span::styled(model_short, theme::LABEL.add_modifier(Modifier::BOLD)),
            Span::styled(" · ", theme::DIM),
            Span::styled(ctx_str, Style::new().fg(ctx_color).add_modifier(Modifier::BOLD)),
        ];

        if let Some(cd) = &cooldown_str {
            left.push(Span::styled(cd.clone(), theme::WARNING));
        } else {
            left.push(Span::styled(" · ", theme::DIM));
            left.push(Span::styled(cost_str.clone(), theme::DIM));
        }

        // ── Measure widths ───────────────────────────────────────────────────
        let left_len: usize = left.iter().map(|s| s.content.chars().count()).sum();
        let right_full = format!("  {}  {}", turns_str, hints);
        let right_mid  = format!("  {}", turns_str);
        let right_hint_only = format!("  {}", hints.trim());

        // Choose what fits
        let right_str = if left_len + right_full.len() <= width {
            right_full
        } else if left_len + right_mid.len() <= width {
            right_mid
        } else if left_len + right_hint_only.len() <= width {
            right_hint_only
        } else {
            String::new()
        };

        // Pad to fill width so bg covers the full row
        let used = left_len + right_str.chars().count();
        let pad  = width.saturating_sub(used);

        let mut spans = left;
        if !right_str.is_empty() {
            spans.push(Span::styled(right_str, theme::DIM));
        }
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad)));
        }

        Paragraph::new(Line::from(spans))
            .style(Style::new().bg(theme::FOOTER_BG))
            .render(area, buf);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn truncate_model_name(name: &str, max: usize) -> String {
    // Strip common prefixes: "openai/", "anthropic/", etc.
    let stripped = name
        .find('/')
        .map(|i| &name[i + 1..])
        .unwrap_or(name);

    if stripped.chars().count() <= max {
        stripped.to_string()
    } else {
        let mut s: String = stripped.chars().take(max - 1).collect();
        s.push('…');
        s
    }
}