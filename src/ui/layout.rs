//! Main layout orchestration for the TUI.
//!
//! Arranges the header, chat panel, context panel, hints bar, and input
//! into a cohesive terminal interface.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::App;
use crate::ui::theme;

// ============================================================================
// Layout Constants
// ============================================================================

/// Height of the header bar.
const HEADER_HEIGHT: u16 = 1;

/// Height of the hints bar.
const HINTS_HEIGHT: u16 = 2;

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
pub fn ui(f: &mut Frame, app: &App) {
    let size = f.area();

    // Vertical layout: header, main content, hints, animation, input.
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Min(0),
            Constraint::Length(HINTS_HEIGHT),
            Constraint::Length(ANIMATION_HEIGHT),
            Constraint::Length(INPUT_HEIGHT),
        ])
        .split(size);

    // ── HEADER ─────────────────────────────────────────────────────────────
    let status_style = if app.streaming { theme::STATUS_LIVE } else { theme::STATUS_IDLE };
    let status_label = if app.streaming { "LIVE" } else { "IDLE" };
    let ctx_color = theme::context_status_color(app.context_pct);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(" 🤖 Agent   ", theme::USER),
        Span::styled("model: ", theme::DIM),
        Span::styled(app.model_name.clone(), theme::ACCENT),
        Span::styled("   ctx: ", theme::DIM),
        Span::styled(format!("{:.1}%", app.context_pct), ctx_color),
        Span::styled("   cost: ", theme::DIM),
        Span::styled(format!("${:.4}", app.cost), theme::SUCCESS),
        Span::styled("   ", theme::DIM),
        Span::styled(format!("[{status_label}]"), status_style),
    ]))
    .style(ratatui::style::Style::new().bg(ratatui::style::Color::Black));
    f.render_widget(header, root[0]);

    // ── MAIN CONTENT: Chat + Context panels ────────────────────────────────
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(CHAT_PANEL_WIDTH_PCT),
            Constraint::Percentage(CONTEXT_PANEL_WIDTH_PCT),
        ])
        .split(root[1]);

    use crate::ui::{chat::ChatPanel, context::ContextPanel};
    f.render_widget(ChatPanel { app }, cols[0]);
    f.render_widget(ContextPanel { app }, cols[1]);

    // ── HINTS BAR ──────────────────────────────────────────────────────────
    let hints = Paragraph::new(Line::from(vec![
        Span::styled(" /clear  ", theme::DIM),
        Span::styled("PgUp/PgDn  ", theme::DIM),
        Span::styled("Ctrl+U clear input  ", theme::DIM),
        Span::styled("Ctrl+C exit", theme::DIM),
    ]))
    .block(Block::default().borders(Borders::TOP).border_style(theme::BORDER));
    f.render_widget(hints, root[2]);

    // ── THINKING ANIMATION ────────────────────────────────────────────────
    if app.streaming {
        if let Some(elapsed_ms) = app.streaming_elapsed_ms() {
            let frame = get_breathing_frame(elapsed_ms);
            let (char1, status) = frame;
            let anim = Paragraph::new(Line::from(vec![
                Span::styled(format!("{} {}...", char1, status), theme::ACCENT),
            ]));
            f.render_widget(anim, root[3]);
        }
    }

    // ── INPUT ──────────────────────────────────────────────────────────────
    let input_widget = Paragraph::new(format!(" ❯ {}_", app.input))
        .style(ratatui::style::Style::new().fg(ratatui::style::Color::White))
        .block(Block::default().borders(Borders::ALL).border_style(theme::ACCENT));
    f.render_widget(input_widget, root[4]);
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