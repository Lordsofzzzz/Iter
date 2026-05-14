//! Minimal runtime state for CLI output.

/// Runtime state for a single CLI session.
pub struct State {
    pub provider_name:       String,
    pub model_name:          String,
    pub model_limit:         u32,
    pub model_temp:          f32,
    pub context_pct:         f32,
    pub context_tokens:      u32,
    pub cost:                f64,
    pub turns:               u32,
    pub tool_calls:          u32,
    pub tokens_input:        u32,
    pub tokens_output:       u32,
    pub tokens_cache_read:   u32,
    pub tokens_cache_write:  u32,
    pub tokens_total:        u32,
    pub pending_tool_call:   Option<(String, String)>,
    pub models:              Vec<(String, String)>,
    pub show_thinking:       bool,
}

impl State {
    pub fn new() -> Self {
        Self {
            provider_name:      String::new(),
            model_name:         String::new(),
            model_limit:        128_000,
            model_temp:         0.7,
            context_pct:        0.0,
            context_tokens:     0,
            cost:               0.0,
            turns:              0,
            tool_calls:         0,
            tokens_input:       0,
            tokens_output:      0,
            tokens_cache_read:  0,
            tokens_cache_write: 0,
            tokens_total:       0,
            pending_tool_call: None,
            models:             Vec::new(),
            show_thinking:      false,
        }
    }
}
