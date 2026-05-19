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
}

#[derive(Debug, Clone)]
pub enum TuiCommand {
    Prompt(String),
    Abort,
    SetModel(String),
    SetProvider(String),
    Clear,
}
