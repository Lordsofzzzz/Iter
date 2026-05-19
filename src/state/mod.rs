//! Minimal runtime state for CLI output.

use std::time::Instant;

/// Runtime state for a single CLI session.
pub struct State {
    pub provider_name:       String,
    pub model_name:          String,
    pub context_pct:         f32,
    pub cost:                f64,
    pub turns:               u32,
    pub tool_calls:          u32,
    pub tokens_input:        u32,
    pub tokens_output:       u32,
    pub tokens_cache_read:   u32,
    pub tokens_cache_write:  u32,
    pub tokens_total:        u32,
    pub pending_tool_call:   Option<(String, String)>,
    pub tool_start_time:     Option<Instant>,
    pub show_thinking:       bool,
    pub cost_input_per_mtok:  f64,
    pub cost_output_per_mtok: f64,
}

impl State {
    pub fn new() -> Self {
        Self {
            provider_name:      String::new(),
            model_name:         String::new(),
            context_pct:        0.0,
            cost:               0.0,
            turns:              0,
            tool_calls:         0,
            tokens_input:       0,
            tokens_output:      0,
            tokens_cache_read:  0,
            tokens_cache_write: 0,
            tokens_total:       0,
            pending_tool_call:  None,
            tool_start_time:    None,
            show_thinking:      false,
            cost_input_per_mtok:  0.0,
            cost_output_per_mtok: 0.0,
        }
    }
}
