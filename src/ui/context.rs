//! Context panel widget - displays token usage and session stats.
//!
//! Shows context window gauge, token breakdown, session statistics,
//! and model configuration info in a side panel.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Widget},
};

use crate::state::{App, ModelStatus};
use crate::ui::theme;

// ============================================================================
// Layout Constants
// ============================================================================

/// Height of the context gauge section (in lines).
const CONTEXT_GAUGE_HEIGHT: u16 = 3;

/// Height of the token breakdown section.
const TOKEN_BREAKDOWN_HEIGHT: u16 = 8;

/// Height of the session stats section.
const SESSION_STATS_HEIGHT: u16 = 7;



// ============================================================================
// Widget Definition
// ============================================================================

/// Context panel widget displaying token usage and model info.
pub struct ContextPanel<'a> {
    pub app: &'a App,
}

impl<'a> Widget for ContextPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" CONTEXT ")
            .borders(Borders::ALL)
            .border_style(theme::BORDER);
        let inner = block.inner(area);
        block.render(area, buf);

        // Split into vertical sections.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(CONTEXT_GAUGE_HEIGHT),
                Constraint::Length(TOKEN_BREAKDOWN_HEIGHT),
                Constraint::Length(SESSION_STATS_HEIGHT),
            ])
            .split(inner);

        self.render_context_gauge(chunks[0], buf);
        self.render_token_breakdown(chunks[1], buf);
        self.render_session_stats(chunks[2], buf);
    }
}

impl<'a> ContextPanel<'a> {
    /// Renders the context window usage gauge.
    fn render_context_gauge(&self, area: Rect, buf: &mut Buffer) {
        let ctx_pct = self.app.context_pct.min(100.0) as u16;
        let ctx_col = theme::context_gauge_color(self.app.context_pct);
        let ctx_label = format!(
            "{} / {}k tok  ({:.1}%)",
            self.app.tok_input,
            self.app.tok_limit / 1000,
            self.app.context_pct,
        );

        let gauge = Gauge::default()
            .block(Block::default()
                .title(" Context Window ")
                .borders(Borders::ALL)
                .border_style(theme::BORDER))
            .gauge_style(Style::new().fg(ctx_col).bg(ratatui::style::Color::Black))
            .percent(ctx_pct)
            .label(ctx_label);
        gauge.render(area, buf);
    }

    /// Renders token breakdown (input, output, cache).
    fn render_token_breakdown(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" Tokens ")
            .borders(Borders::ALL)
            .border_style(theme::BORDER);
        let inner = block.inner(area);
        block.render(area, buf);

        let tok_lines = vec![
            token_row("input   ", self.app.tok_input, theme::TOK_INPUT),
            token_row("cache-in", self.app.tok_cache_read, theme::TOK_CACHE_READ),
            token_row("cache-out", self.app.tok_cache_write, theme::TOK_CACHE_WRITE),
            token_row("output  ", self.app.tok_output, theme::TOK_OUTPUT),
            Line::from(vec![
                Span::styled(" ─────────────────", theme::DIM),
            ]),
            token_row("total   ", self.app.tok_total, theme::TOK_TOTAL),
        ];
        Paragraph::new(tok_lines).render(inner, buf);
    }

    /// Renders session statistics (turns, tools, cost).
    fn render_session_stats(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" Session ")
            .borders(Borders::ALL)
            .border_style(theme::BORDER);
        let inner = block.inner(area);
        block.render(area, buf);

        // Show more decimal places for free/cheap models.
        let cost_str = if self.app.cost < 0.001 {
            format!("${:.6}", self.app.cost)
        } else {
            format!("${:.4}", self.app.cost)
        };

        let sess_lines = vec![
            stat_row("turns  ", self.app.turns.to_string(), theme::STAT_TURNS),
            stat_row("tools  ", self.app.tool_calls.to_string(), theme::STAT_TOOLS),
            stat_row("cost   ", cost_str, theme::STAT_COST),
            stat_row("summ'd ", format!("{}x", self.app.summarized), theme::STAT_SUMMARIZED),
        ];
        Paragraph::new(sess_lines).render(inner, buf);
    }

    }

// ============================================================================
// Private Helpers
// ============================================================================

/// Creates a token row with label and value.
fn token_row(label: &str, tok: u32, val_style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(" ", Style::new()),
        Span::styled(format!("{label} "), theme::LABEL),
        Span::styled(format!("{tok:>7}"), val_style),
    ])
}

/// Creates a stats row with label and value.
fn stat_row<'a>(label: &'a str, value: String, val_style: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled(" ", Style::new()),
        Span::styled(label, theme::LABEL),
        Span::styled(format!(" {value}"), val_style),
    ])
}