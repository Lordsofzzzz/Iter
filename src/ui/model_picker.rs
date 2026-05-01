//! Model picker overlay — triggered by Ctrl+P.
//!
//! Renders a centered popup with a fuzzy-search input and scrollable
//! model list, similar to opencode's command palette.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, StatefulWidget, Widget},
};

use crate::state::App;
use crate::ui::theme;

// ============================================================================
// Known Models
// ============================================================================

pub const MODELS: &[(&str, &str)] = &[
    ("anthropic/claude-sonnet-4-5:thinking",  "Claude Sonnet 4.5 Thinking"),
    ("nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free", "Nemotron Nano Reasoning (free)"),
    ("anthropic/claude-sonnet-4-5",           "Claude Sonnet 4.5"),
    ("anthropic/claude-opus-4",               "Claude Opus 4"),
    ("anthropic/claude-haiku-3-5",            "Claude Haiku 3.5"),
    ("openai/gpt-4o",                         "GPT-4o"),
    ("openai/gpt-4o-mini",                    "GPT-4o Mini"),
    ("openai/o3",                             "OpenAI o3"),
    ("openai/o4-mini",                        "OpenAI o4-mini"),
    ("google/gemini-2.5-pro",                 "Gemini 2.5 Pro"),
    ("google/gemini-2.5-flash",               "Gemini 2.5 Flash"),
    ("meta-llama/llama-4-maverick",           "Llama 4 Maverick"),
    ("meta-llama/llama-4-scout",              "Llama 4 Scout"),
    ("deepseek/deepseek-r2",                  "DeepSeek R2"),
    ("deepseek/deepseek-chat-v3-0324",        "DeepSeek V3"),
    ("minimax/minimax-m2.5:free",             "MiniMax M2.5 (free)"),
    ("mistralai/mistral-large-2411",          "Mistral Large"),
    ("qwen/qwq-32b",                          "Qwen QwQ 32B (thinking)"),
    ("x-ai/grok-3-mini",                      "Grok 3 Mini"),
];

// ============================================================================
// Filtering
// ============================================================================

/// Returns indices into MODELS that match the query (case-insensitive substring).
pub fn filtered_models(query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    MODELS
        .iter()
        .enumerate()
        .filter(|(_, (id, name))| {
            q.is_empty() || id.contains(&*q) || name.to_lowercase().contains(&*q)
        })
        .map(|(i, _)| i)
        .collect()
}

// ============================================================================
// Widget
// ============================================================================

pub struct ModelPicker<'a> {
    pub app: &'a App,
}

impl<'a> Widget for ModelPicker<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // ── Center a 60×18 popup ──────────────────────────────────────────
        let popup = centered_rect(60, 18, area);

        // Clear the background behind the popup.
        Clear.render(popup, buf);

        // Outer block.
        let block = Block::default()
            .title(Line::from(vec![
                Span::styled(" 🔍 ", theme::ACCENT),
                Span::styled("Model Picker", theme::ACCENT.add_modifier(Modifier::BOLD)),
                Span::styled("  Ctrl+P to close  ", theme::DIM),
            ]))
            .borders(Borders::ALL)
            .border_style(theme::ACCENT);

        let inner = block.inner(popup);
        block.render(popup, buf);

        // ── Split inner: search bar (3) + list (rest) ────────────────────
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(inner);

        // ── Search input ─────────────────────────────────────────────────
        let input = Paragraph::new(format!(" ❯ {}_", self.app.model_picker_query))
            .style(Style::default().fg(Color::White))
            .block(Block::default().borders(Borders::ALL).border_style(theme::DIM));
        input.render(layout[0], buf);

        // ── Model list ───────────────────────────────────────────────────
        let matches = filtered_models(&self.app.model_picker_query);
        let selected_idx = self.app.model_picker_selected.min(matches.len().saturating_sub(1));

        let items: Vec<ListItem> = matches
            .iter()
            .enumerate()
            .map(|(pos, &model_idx)| {
                let (id, name) = MODELS[model_idx];
                let is_current = id == self.app.model_name.as_str();
                let is_selected = pos == selected_idx;

                let style = if is_selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::from_u32(0x00_7C_A5_FF)) // accent blue
                        .add_modifier(Modifier::BOLD)
                } else if is_current {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::White)
                };

                let marker = if is_current { "✓ " } else { "  " };
                ListItem::new(Line::from(vec![
                    Span::styled(marker, style),
                    Span::styled(name, style),
                    Span::styled(format!("  ({})", id), if is_selected {
                        style
                    } else {
                        Style::default().fg(Color::DarkGray)
                    }),
                ]))
            })
            .collect();

        if items.is_empty() {
            let no_match = Paragraph::new(" No models match your query")
                .style(Style::default().fg(Color::DarkGray));
            no_match.render(layout[1], buf);
        } else {
            let list = List::new(items)
                .block(Block::default().borders(Borders::NONE))
                .highlight_symbol("");
            let mut state = ListState::default();
            state.select(Some(selected_idx));
            StatefulWidget::render(list, layout[1], buf, &mut state);
        }

        // ── Hint bar at bottom of outer popup ────────────────────────────
        let hint_y = popup.y + popup.height.saturating_sub(1);
        if hint_y < area.y + area.height {
            let hint = Paragraph::new(Line::from(vec![
                Span::styled(" ↑↓ navigate  ", theme::DIM),
                Span::styled("Enter", theme::ACCENT),
                Span::styled(" select  ", theme::DIM),
                Span::styled("Esc/Ctrl+P", theme::ACCENT),
                Span::styled(" close ", theme::DIM),
            ]));
            let hint_area = Rect { x: popup.x, y: hint_y, width: popup.width, height: 1 };
            hint.render(hint_area, buf);
        }
    }
}

// ============================================================================
// Helper
// ============================================================================

/// Returns a centered `Rect` with the given width/height inside `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width:  width.min(area.width),
        height: height.min(area.height),
    }
}