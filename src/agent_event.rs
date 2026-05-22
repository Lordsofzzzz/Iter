#[derive(Debug, Clone)]
pub enum AgentEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCall { name: String, input: String },
    ToolOutput { delta: String },
    ToolResult { name: String, output: String, elapsed_ms: u64 },
    Error { message: String },
    AgentEnd { success: bool, error: Option<String> },
    TurnStart,
    TurnEnd,
    AgentStart,
    ProviderChanged { provider_id: String, provider_name: String },
    ModelList { models: Vec<(String, String)> },
    TokenUsage {
        input: u32,
        output: u32,
        total: u32,
        cache_read: u32,
        cache_write: u32,
        context_pct: f32,
    },
    /// Emitted once per retry tick so the TUI can display a live countdown.
    /// `attempt`  — which retry this is (1-based).
    /// `total`    — max retries allowed.
    /// `wait_ms`  — total backoff duration for this attempt.
    /// `elapsed_ms` — how many ms have passed so far in this wait.
    Retrying {
        attempt: u8,
        total: u8,
        wait_ms: u64,
        elapsed_ms: u64,
    },
}

#[derive(Debug, Clone)]
pub enum TuiCommand {
    Prompt(String),
    Abort,
    SetModel(String),
    SetProvider(String),
    SetProviderWithKey { provider: String, key: String },
    Clear,
}
