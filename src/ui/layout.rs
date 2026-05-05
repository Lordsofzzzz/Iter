//! Main layout orchestration for the TUI.
//!
//! Codex-style: full-width chat transcript, single-row footer status line.
//! No side panel — token stats live in the footer.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style,
    widgets::{Block, Borders},
    Frame,
};

use crate::state::App;
use crate::ui::{chat::ChatPanel, footer::Footer, model_picker::ModelPicker, theme};

// ── Layout constants ─────────────────────────────────────────────────────────

/// Height of the footer status line.
const FOOTER_HEIGHT: u16 = 1;

/// Height of the input composer.
const INPUT_HEIGHT: u16 = 3;

// ── Public API ────────────────────────────────────────────────────────────────

/// Renders the complete UI layout.
pub fn ui(f: &mut Frame, app: &mut App) {
    let size = f.area();

    // Paint entire terminal with dark background.
    f.render_widget(Block::default().style(Style::new().bg(theme::BG)), size);

    // Vertical split: chat (fills remaining) · footer · input
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(FOOTER_HEIGHT),
            Constraint::Length(INPUT_HEIGHT),
        ])
        .split(size);

    // ── Full-width chat transcript ────────────────────────────────────────
    f.render_widget(ChatPanel { app }, root[0]);

    // ── Footer status line ────────────────────────────────────────────────
    f.render_widget(Footer { app }, root[1]);

    // ── Input composer ────────────────────────────────────────────────────
    app.textarea.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::ACCENT)
            .style(Style::new().bg(theme::BG)),
    );
    f.render_widget(&app.textarea, root[2]);

    // ── Model picker overlay (rendered on top) ────────────────────────────
    if app.model_picker_open {
        f.render_widget(ModelPicker { app }, size);
    }
}