use serde::Deserialize;

// ── Push events ──────────────────────────────────────────────────────────
// Identified by absence of "kind" field.
// Deserialized via tag = "type".
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PushEvent {
    AgentStart,
    TurnStart,
    TurnEnd,
    AgentEnd,
    TextDelta { delta: String },
    Error { message: String },
    Cooldown { wait_ms: u64, retries_left: u32 },
    RetryResult { success: bool, attempt: u32 },
}

// ── Pull responses ───────────────────────────────────────────────────────
// Identified by kind = "response".
// Always has command + success. id is echoed back if sent.
#[derive(Debug, Deserialize)]
pub struct PullResponse {
    pub command: String,
    pub id:      Option<String>,   // F3: echoed correlation id
    pub success: bool,
    pub error:   Option<String>,   // present when success = false
    pub data:    Option<serde_json::Value>, // parsed per-command in handler
}

// ── Parsed data shapes for pull responses ───────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StateData {
    pub model_name:   String,
    pub model_limit:  u32,
    pub temp:         f32,
    pub is_streaming: bool,
}

#[derive(Debug, Deserialize)]
pub struct TokenBreakdown {
    pub input:       u32,
    pub output:      u32,
    pub cache_read:  u32,
    pub cache_write: u32,
    pub total:       u32,
}

#[derive(Debug, Deserialize)]
pub struct ContextUsage {
    pub tokens:  u32,
    pub limit:   u32,
    pub percent: f32,
}

#[derive(Debug, Deserialize)]
pub struct SessionStatsData {
    pub tokens:        TokenBreakdown,
    pub context_usage: ContextUsage,
    pub cost:          f64,
    pub turns:         u32,
}

// ── Raw wire envelope ────────────────────────────────────────────────────
// First step: determine if line is a push event or pull response.
// Check for "kind":"response" field.
#[derive(Debug, Deserialize)]
struct RawEnvelope {
    pub kind: Option<String>,
}

// ── Top-level discriminated type ─────────────────────────────────────────
#[derive(Debug)]
pub enum AgentMessage {
    Push(PushEvent),
    Pull(PullResponse),
    // F4: unknown event — logged, never silently dropped
    Unknown { raw: String },
}

/// Parse a raw JSONL line into an AgentMessage.
/// F4: never returns Err silently — all parse failures become Unknown { raw }.
pub fn parse_line(line: &str) -> AgentMessage {
    // peek at envelope to route
    let envelope: RawEnvelope = match serde_json::from_str(line) {
        Ok(e)  => e,
        Err(_) => return AgentMessage::Unknown { raw: line.to_string() },
    };

    match envelope.kind.as_deref() {
        // F2: "kind":"response" → pull response
        Some("response") => {
            match serde_json::from_str::<PullResponse>(line) {
                Ok(r)  => AgentMessage::Pull(r),
                // F4: parse failure logged as Unknown, never swallowed
                Err(e) => AgentMessage::Unknown {
                    raw: format!("pull-parse-err: {e} | {line}"),
                },
            }
        }
        // no kind → push event
        None => {
            match serde_json::from_str::<PushEvent>(line) {
                Ok(ev) => AgentMessage::Push(ev),
                // F4: unknown event type — stored as Unknown for logging
                Err(e) => AgentMessage::Unknown {
                    raw: format!("push-parse-err: {e} | {line}"),
                },
            }
        }
        // unexpected kind value
        Some(other) => AgentMessage::Unknown {
            raw: format!("unknown-kind: {other} | {line}"),
        },
    }
}

// ── Internal channel event ────────────────────────────────────────────────
#[derive(Debug)]
pub enum UiEvent {
    Agent(AgentMessage),
    SpawnError(String),
}