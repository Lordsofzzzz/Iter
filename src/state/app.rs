//! Application state for the TUI.
//!
//! Holds all mutable state including messages, token counts, model info,
//! and UI-related flags like scrolling position.

use std::time::Instant;

use crate::rpc::{ChatMessage, MsgKind, ModelStatus};

// ============================================================================
// Constants (extracted from magic numbers)
// ============================================================================

/// Default context window size (200k tokens).
const DEFAULT_CONTEXT_LIMIT: u32 = 200_000;

/// Default temperature for LLM requests.
const DEFAULT_TEMPERATURE: f32 = 0.3;

/// Default scroll jump amount (number of lines).
const SCROLL_STEP: usize = 3;

/// Maximum scroll position (used to scroll to bottom).
const MAX_SCROLL: usize = usize::MAX;

// ============================================================================
// Application State
// ============================================================================

/// Main application state, shared across all UI components.
pub struct App {
    // ── Chat Messages ───────────────────────────────────────────────────────
    pub messages: Vec<ChatMessage>,
    pub scroll:   usize,

    // ── Input State ─────────────────────────────────────────────────────────
    pub input:      String,
    pub streaming:  bool,

    // ── Token Tracking ──────────────────────────────────────────────────────
    pub tok_input:       u32,
    pub tok_output:      u32,
    pub tok_cache_read:  u32,
    pub tok_cache_write: u32,
    pub tok_total:       u32,
    pub tok_limit:       u32,

    // ── Context Usage ───────────────────────────────────────────────────────
    pub context_pct: f32,

    // ── Session Statistics ──────────────────────────────────────────────────
    pub turns:      u32,
    pub tool_calls: u32,
    pub cost:       f64,
    pub summarized: u32,

    // ── Model Configuration ─────────────────────────────────────────────────
    pub model_name:   String,
    pub model_limit:  u32,
    pub model_temp:   f32,
    pub model_status: ModelStatus,

    // ── Application Control ──────────────────────────────────────────────────
    pub should_quit: bool,

    // ── Rate Limiting State ─────────────────────────────────────────────────
    pub cooldown_deadline:    Option<Instant>,
    pub cooldown_started:     Option<Instant>,
    pub cooldown_retries_left: u32,

    // ── Animation State ────────────────────────────────────────────────────
    pub streaming_started_at: Option<Instant>,
}

impl App {
    /// Creates a new `App` with default values.
    pub fn new() -> Self {
        App {
            messages:   Vec::new(),
            scroll:     0,
            input:      String::new(),
            streaming:  false,

            tok_input:       0,
            tok_output:      0,
            tok_cache_read:  0,
            tok_cache_write: 0,
            tok_total:       0,
            tok_limit:       DEFAULT_CONTEXT_LIMIT,

            context_pct: 0.0,

            turns:      0,
            tool_calls: 0,
            cost:       0.0,
            summarized: 0,

            model_name:   "—".into(),
            model_limit:  DEFAULT_CONTEXT_LIMIT,
            model_temp:   DEFAULT_TEMPERATURE,
            model_status: ModelStatus::Ready,

            should_quit: false,

            cooldown_deadline:    None,
            cooldown_started:     None,
            cooldown_retries_left: 0,

            streaming_started_at: None,
        }
    }

    /// Returns context percentage clamped to 0-100 for UI display.
    #[allow(dead_code)]
    pub fn context_pct_computed(&self) -> u16 {
        (self.context_pct.min(100.0)) as u16
    }

    /// Returns total token usage as a percentage of limit.
    #[allow(dead_code)]
    pub fn token_pct(&self) -> u16 {
        ((self.tok_total as f64 / self.tok_limit as f64) * 100.0).min(100.0) as u16
    }

    /// Scroll up by the standard step amount.
    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(SCROLL_STEP);
    }

    /// Scroll down by the standard step amount.
    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(SCROLL_STEP);
    }

    /// Scroll to the bottom of the message list.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll = MAX_SCROLL;
    }

    /// Mark streaming as started; updates model status.
    pub fn start_streaming(&mut self) {
        self.streaming           = true;
        self.model_status        = ModelStatus::Thinking;
        self.streaming_started_at = Some(Instant::now());
    }

    /// Mark streaming as complete; resets model status to ready.
    pub fn end_streaming(&mut self) {
        self.streaming           = false;
        self.model_status        = ModelStatus::Ready;
        self.streaming_started_at = None;
    }

    /// Returns elapsed milliseconds since streaming started.
    pub fn streaming_elapsed_ms(&self) -> Option<u64> {
        self.streaming_started_at.map(|started| {
            started.elapsed().as_millis() as u64
        })
    }

    /// Set error state; used when LLM returns an error.
    pub fn set_error(&mut self) {
        self.streaming    = false;
        self.model_status = ModelStatus::Error;
    }

    /// Append text delta to the last assistant message, or create a new one.
    pub fn push_assistant_delta(&mut self, delta: String) {
        if let Some(last) = self.messages.last_mut() {
            if matches!(last.kind, MsgKind::Assistant) {
                last.content.push_str(&delta);
                return;
            }
        }
        self.messages.push(ChatMessage { kind: MsgKind::Assistant, content: delta });
    }

    /// Add a system message (error, warning, etc.) to the chat.
    pub fn push_system(&mut self, text: String) {
        self.messages.push(ChatMessage { kind: MsgKind::System, content: text });
    }

    /// Update rate limit state when a 429 is received.
    pub fn upsert_rate_limit(&mut self, wait_ms: u64, retries_left: u32) {
        let now = Instant::now();
        self.cooldown_deadline    = Some(now + std::time::Duration::from_millis(wait_ms));
        self.cooldown_started     = Some(now);
        self.cooldown_retries_left = retries_left;
    }

    /// Clear rate limit state after cooldown expires or retry succeeds.
    pub fn clear_rate_limit(&mut self) {
        self.messages.retain(|m| m.kind != MsgKind::RateLimit);
        self.cooldown_deadline    = None;
        self.cooldown_started     = None;
        self.cooldown_retries_left = 0;
    }

    /// Update token counts from session stats response.
    pub fn update_tokens(&mut self, input: u32, output: u32, cache_read: u32, cache_write: u32, total: u32, limit: u32) {
        self.tok_input       = input;
        self.tok_output      = output;
        self.tok_cache_read  = cache_read;
        self.tok_cache_write = cache_write;
        self.tok_total       = total;
        self.tok_limit       = limit;
    }

    /// Update model configuration from state response.
    pub fn update_model_info(&mut self, name: String, limit: u32, temp: f32) {
        self.model_name  = name;
        self.model_limit = limit;
        self.model_temp  = temp;
    }
}