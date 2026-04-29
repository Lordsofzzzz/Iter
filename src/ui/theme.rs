use ratatui::style::{Color, Modifier, Style};

// ═══════════════════════════════════════════════════════════════════════════════
// THEMES — centralized style constants to avoid Style::default().fg(Color::X) repetition
// ═══════════════════════════════════════════════════════════════════════════════

// Border and label styles
pub const BORDER: Style = Style::new().fg(Color::DarkGray);
pub const LABEL: Style = Style::new().fg(Color::DarkGray);

// Role-based styles for chat messages
pub const USER: Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
pub const ASSISTANT: Style = Style::new().fg(Color::Gray);
pub const SYSTEM: Style = Style::new().fg(Color::DarkGray);

// Accent colors for UI elements
pub const ACCENT: Style = Style::new().fg(Color::Cyan);
pub const SUCCESS: Style = Style::new().fg(Color::Green);
pub const WARNING: Style = Style::new().fg(Color::Yellow);
pub const ERROR: Style = Style::new().fg(Color::Red);

// Dimmed text
pub const DIM: Style = Style::new().fg(Color::DarkGray);

// Tool-specific styles
pub const TOOL_CALL: Style = Style::new().fg(Color::Yellow);
pub const TOOL_RESULT: Style = Style::new().fg(Color::Green);

// Status indicators
pub const STATUS_LIVE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const STATUS_IDLE: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);

// Token breakdown colors
pub const TOK_INPUT: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const TOK_OUTPUT: Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
pub const TOK_CACHE_WRITE: Style = Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);
pub const TOK_CACHE_READ: Style = Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD);
pub const TOK_TOTAL: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);

// Session stats colors
pub const STAT_TURNS: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const STAT_TOOLS: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
pub const STAT_COST: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
pub const STAT_SUMMARIZED: Style = Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);

// Model info colors
pub const MODEL_NAME: Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
pub const MODEL_LIMIT: Style = Style::new().fg(Color::Gray).add_modifier(Modifier::BOLD);
pub const MODEL_TEMP: Style = Style::new().fg(Color::Gray).add_modifier(Modifier::BOLD);

// Status colors for model state
pub const MODEL_READY: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
pub const MODEL_THINKING: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const MODEL_ERROR: Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
pub const MODEL_COOLDOWN: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);

// Context gauge colors based on percentage
pub fn context_gauge_color(pct: f32) -> Color {
    if pct >= 85.0 {
        Color::Red
    } else if pct >= 60.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

// Status color based on context percentage
pub fn context_status_color(pct: f32) -> Color {
    if pct > 80.0 {
        Color::Red
    } else if pct > 60.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}