//! Minimal runtime state for CLI output.

/// Runtime state for a single CLI session.
pub struct State {
    pub model_name:        String,
    pub model_limit:       u32,
    pub model_temp:        f32,
    pub context_pct:       f32,
    pub cost:              f64,
    pub turns:             u32,
    pub tool_calls:        u32,
    pub pending_tool_call: Option<(String, String)>,
    pub models:            Vec<(String, String)>,
    pub show_thinking:     bool,
}

impl State {
    pub fn new() -> Self {
        Self {
            model_name:        String::new(),
            model_limit:       128_000,
            model_temp:        0.7,
            context_pct:       0.0,
            cost:              0.0,
            turns:             0,
            tool_calls:        0,
            pending_tool_call: None,
            models:            Vec::new(),
            show_thinking:     false,
        }
    }
}
