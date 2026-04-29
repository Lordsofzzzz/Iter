use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Widget},
};

use crate::state::{App, ModelStatus};
use crate::ui::theme;

pub struct ContextPanel<'a> {
    pub app: &'a App,
}

impl<'a> Widget for ContextPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" CONTEXT & TOKENS ")
            .borders(Borders::ALL)
            .border_style(theme::BORDER);
        let inner = block.inner(area);
        block.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // D2: context window gauge
                Constraint::Length(8),  // D4: token breakdown
                Constraint::Length(7),  // session stats
                Constraint::Min(6),     // model info
            ])
            .split(inner);

        self.render_context_gauge(chunks[0], buf);
        self.render_token_breakdown(chunks[1], buf);
        self.render_session_stats(chunks[2], buf);
        self.render_model_info(chunks[3], buf);
    }
}

impl<'a> ContextPanel<'a> {
    /// ── 1. CONTEXT WINDOW GAUGE ─────────────────────────────────────────────
    /// Shows how much of the model context window is used
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

    /// ── 2. TOKEN BREAKDOWN ───────────────────────────────────────────────────
    fn render_token_breakdown(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" Tokens ")
            .borders(Borders::ALL)
            .border_style(theme::BORDER);
        let inner = block.inner(area);
        block.render(area, buf);

        let tok_lines = vec![
            tok_row("input  ", self.app.tok_input, theme::TOK_INPUT),
            tok_row("output ", self.app.tok_output, theme::TOK_OUTPUT),
            tok_row("$cache↑", self.app.tok_cache_write, theme::TOK_CACHE_WRITE),
            tok_row("$cache↓", self.app.tok_cache_read, theme::TOK_CACHE_READ),
            Line::from(vec![
                Span::styled(" ─────────────────", theme::DIM),
            ]),
            tok_row("total  ", self.app.tok_total, theme::TOK_TOTAL),
        ];
        Paragraph::new(tok_lines).render(inner, buf);
    }

    /// ── 3. SESSION STATS ──────────────────────────────────────────────────────
    fn render_session_stats(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" Session ")
            .borders(Borders::ALL)
            .border_style(theme::BORDER);
        let inner = block.inner(area);
        block.render(area, buf);

        // D3: cost — show more decimals for free/cheap models
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

    /// ── 4. MODEL INFO ─────────────────────────────────────────────────────────
    fn render_model_info(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" Model ")
            .borders(Borders::ALL)
            .border_style(theme::BORDER);
        let inner = block.inner(area);
        block.render(area, buf);

        let status_style = match self.app.model_status {
            ModelStatus::Ready    => theme::MODEL_READY,
            ModelStatus::Thinking => theme::MODEL_THINKING,
            ModelStatus::Error    => theme::MODEL_ERROR,
            ModelStatus::Cooldown => theme::MODEL_COOLDOWN,
        };

        let model_lines = vec![
            stat_row("name  ", self.app.model_name.clone(), theme::MODEL_NAME),
            stat_row("limit ", format!("{}k", self.app.model_limit / 1000), theme::MODEL_LIMIT),
            stat_row("temp  ", format!("{:.1}", self.app.model_temp), theme::MODEL_TEMP),
            stat_row("status", self.app.model_status.label().to_string(), status_style),
        ];
        Paragraph::new(model_lines).render(inner, buf);
    }
}

fn tok_row(label: &str, tok: u32, val_style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(" ", Style::new()),
        Span::styled(format!("{label} "), theme::LABEL),
        Span::styled(format!("{tok:>7}"), val_style),
    ])
}

fn stat_row<'a>(label: &'a str, value: String, val_style: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled(" ", Style::new()),
        Span::styled(label, theme::LABEL),
        Span::styled(format!(" {value}"), val_style),
    ])
}