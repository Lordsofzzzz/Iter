//! Terminal UI theme and styling constants.
//!
//! Centralized style definitions to avoid repetitive `Style::default().fg(Color::X)`
//! throughout the UI components.

use ratatui::style::{Color, Modifier, Style};

// ============================================================================
// Border & Labels
// ============================================================================

pub const BORDER: Style = Style::new().fg(Color::DarkGray);
pub const LABEL: Style = Style::new().fg(Color::DarkGray);

// ============================================================================
// Chat Message Roles
// ============================================================================

/// User message style (white, bold).
pub const USER: Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);

/// Assistant/AI response style (gray).
pub const ASSISTANT: Style = Style::new().fg(Color::Gray);

/// System message style (dark gray).
pub const SYSTEM: Style = Style::new().fg(Color::DarkGray);

// ============================================================================
// Accent Colors
// ============================================================================

pub const ACCENT: Style = Style::new().fg(Color::Cyan);
pub const SUCCESS: Style = Style::new().fg(Color::Green);
pub const WARNING: Style = Style::new().fg(Color::Yellow);
pub const ERROR: Style = Style::new().fg(Color::Red);

/// Dimmed/secondary text.
pub const DIM: Style = Style::new().fg(Color::DarkGray);

// ============================================================================
// Tool Messages
// ============================================================================

pub const TOOL_CALL: Style = Style::new().fg(Color::Yellow);
pub const TOOL_RESULT: Style = Style::new().fg(Color::Green);

// ============================================================================
// Status Indicators
// ============================================================================

/// Live/streaming status (cyan, bold).
pub const STATUS_LIVE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

/// Idle/ready status (green, bold).
pub const STATUS_IDLE: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);

// ============================================================================
// Token Breakdown Colors
// ============================================================================

pub const TOK_INPUT:        Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const TOK_OUTPUT:      Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
pub const TOK_CACHE_WRITE: Style = Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);
pub const TOK_CACHE_READ:  Style = Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD);
pub const TOK_TOTAL:       Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);

// ============================================================================
// Session Stats Colors
// ============================================================================

pub const STAT_TURNS:       Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const STAT_TOOLS:       Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
pub const STAT_COST:        Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
pub const STAT_SUMMARIZED:  Style = Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);

// ============================================================================
// Model Info Colors
// ============================================================================

pub const MODEL_NAME:   Style = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
pub const MODEL_LIMIT: Style = Style::new().fg(Color::Gray).add_modifier(Modifier::BOLD);
pub const MODEL_TEMP:  Style = Style::new().fg(Color::Gray).add_modifier(Modifier::BOLD);

// ============================================================================
// Model Status Colors
// ============================================================================

pub const MODEL_READY:    Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
pub const MODEL_THINKING: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const MODEL_ERROR:    Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
pub const MODEL_COOLDOWN: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);

// ============================================================================
// Dynamic Color Functions
// ============================================================================

/// Returns gauge color based on context usage percentage.
pub fn context_gauge_color(pct: f32) -> Color {
    if pct >= 85.0 {
        Color::Red
    } else if pct >= 60.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

/// Returns status bar color based on context usage percentage.
pub fn context_status_color(pct: f32) -> Color {
    if pct > 80.0 {
        Color::Red
    } else if pct > 60.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}