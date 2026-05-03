//! Main layout orchestration for the TUI.
//!
//! Arranges the header, chat panel, context panel, hints bar, and input
//! into a cohesive terminal interface.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use std::time::Instant;

use crate::state::App;
use crate::ui::theme;

// ============================================================================
// Layout Constants
// ============================================================================

/// Height of the header bar.


/// Height of the animation area.
const ANIMATION_HEIGHT: u16 = 1;

/// Height of the input area.
const INPUT_HEIGHT: u16 = 3;

/// Chat panel width as percentage of main content area.
const CHAT_PANEL_WIDTH_PCT: u16 = 75;

/// Context panel width as percentage of main content area.
const CONTEXT_PANEL_WIDTH_PCT: u16 = 25;

// ============================================================================
// Public API
// ============================================================================

/// Renders the complete UI layout.
pub fn ui(f: &mut Frame, app: &mut App) {
    let size = f.area();

    // Paint entire terminal with dark background.
    f.render_widget(Block::default().style(Style::new().bg(theme::BG)), size);

    // Vertical layout: main content, animation, input.
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(ANIMATION_HEIGHT),
            Constraint::Length(INPUT_HEIGHT),
        ])
        .split(size);

    // ── MAIN CONTENT: Chat + Context panels ────────────────────────────────
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(CHAT_PANEL_WIDTH_PCT),
            Constraint::Percentage(CONTEXT_PANEL_WIDTH_PCT),
        ])
        .split(root[0]);

    use crate::ui::{chat::ChatPanel, context::ContextPanel, model_picker::ModelPicker};
    f.render_widget(ChatPanel { app }, cols[0]);
    f.render_widget(ContextPanel { app }, cols[1]);

    // ── MODEL PICKER OVERLAY ───────────────────────────────────────────────
    if app.model_picker_open {
        f.render_widget(ModelPicker { app }, size);
    }

    // ── RATE LIMIT / THINKING ANIMATION ───────────────────────────────────────
    if let Some(deadline) = app.cooldown_deadline {
        let remaining_ms = deadline.saturating_duration_since(Instant::now()).as_millis() as u64;
        let secs = (remaining_ms + 999) / 1000;
        let elapsed_ms = app.streaming_elapsed_ms().unwrap_or(0);
        let frame_char = get_rate_limit_frame(elapsed_ms);
        let rate_label = format!("{} rate limited: {}s ({} left)", frame_char, secs, app.cooldown_retries_left);
        let rate = Paragraph::new(Line::from(vec![
            Span::styled(rate_label, theme::WARNING),
        ]));
        f.render_widget(rate, root[1]);
    } else if app.streaming {
        if let Some(elapsed_ms) = app.streaming_elapsed_ms() {
            let frame = get_breathing_frame(elapsed_ms);
            let (char1, status) = frame;
            let anim = Paragraph::new(Line::from(vec![
                Span::styled(format!(" {} {}...", char1, status), theme::ACCENT),
            ]));
            f.render_widget(anim, root[1]);
        }
    }

    // ── INPUT ──────────────────────────────────────────────────────────────
    app.textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::ACCENT)
            .style(Style::new().bg(theme::BG))
            
    );
    f.render_widget(&app.textarea, root[2]);
}

// ============================================================================
// Private Helpers
// ============================================================================

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

fn get_rate_limit_frame(elapsed_ms: u64) -> char {
    const FRAMES: [char; 4] = ['⏳', '⏱', '⚡', '⚠'];
    let idx = (elapsed_ms / 500) as usize % FRAMES.len();
    FRAMES[idx]
}
