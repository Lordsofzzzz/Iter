//! RPC wire protocol types for TUI ↔ Agent communication.
//!
//! The protocol uses JSONL (one JSON object per line) over stdout/stdin.
//! Two message directions exist:
//!   - **Push**: Agent → TUI (unprompted events like text deltas)
//!   - **Pull**: TUI → Agent → TUI (request/response pattern)

use serde::Deserialize;

// ============================================================================
// Chat Message Types (used by UI for rendering)
// ============================================================================

/// Kind of chat message, determines styling in the UI.
#[derive(Clone, PartialEq)]
pub enum MsgKind {
    User,
    Assistant,
    ToolCall,
    ToolResult,
    System,
    RateLimit,
}

/// A single message in the chat history.
#[derive(Clone)]
pub struct ChatMessage {
    pub kind: MsgKind,
    pub content: String,
}

/// Current status of the LLM model (affects UI indicators).
#[derive(Clone, PartialEq)]
pub enum ModelStatus {
    Ready,
    Thinking,
    Error,
    Cooldown,
}

impl ModelStatus {
    /// Human-readable label for UI display.
    pub fn label(&self) -> &'static str {
        match self {
            ModelStatus::Ready    => "✔ ready",
            ModelStatus::Thinking => "⟳ thinking",
            ModelStatus::Error    => "✗ error",
            ModelStatus::Cooldown => "⏸ cooldown",
        }
    }
}

// ============================================================================
// Push Events: Agent → TUI (unprompted)
// ============================================================================

/// Unprompted events from the agent to the TUI.
/// Identified by absence of the `kind` field.
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

// ============================================================================
// Pull Responses: TUI → Agent → TUI
// ============================================================================

/// Response from the agent to a TUI command.
/// Always has `kind: "response"` which distinguishes it from push events.
#[derive(Debug, Deserialize)]
pub struct PullResponse {
    pub command: String,
    pub id:      Option<String>,   // correlation ID echoed back
    pub success: bool,
    pub error:   Option<String>,
    pub data:    Option<serde_json::Value>,
}

// ============================================================================
// Parsed Data Shapes (for pull response `data` field)
// ============================================================================

/// Data for the `get_state` command response.
#[derive(Debug, Deserialize)]
pub struct StateData {
    pub model_name:   String,
    pub model_limit:  u32,
    pub temp:         f32,
    pub is_streaming: bool,
}

/// Token usage breakdown.
#[derive(Debug, Deserialize)]
pub struct TokenBreakdown {
    pub input:       u32,
    pub output:      u32,
    pub cache_read:  u32,
    pub cache_write: u32,
    pub total:       u32,
}

/// Context window usage information.
#[derive(Debug, Deserialize)]
pub struct ContextUsage {
    pub tokens:  u32,
    pub limit:   u32,
    pub percent: f32,
}

/// Full session statistics for the UI.
#[derive(Debug, Deserialize)]
pub struct SessionStatsData {
    pub tokens:        TokenBreakdown,
    pub context_usage: ContextUsage,
    pub cost:          f64,
    pub turns:         u32,
}

// ============================================================================
// Internal Channel Types
// ============================================================================

/// Wire format envelope — first step in parsing to determine message direction.
#[derive(Debug, Deserialize)]
struct RawEnvelope {
    pub kind: Option<String>,
}

/// Top-level parsed message from the agent.
#[derive(Debug)]
pub enum AgentMessage {
    Push(PushEvent),
    Pull(PullResponse),
    /// Parse failure — logged but never silently dropped.
    Unknown { raw: String },
}

/// Parse a raw JSONL line into an `AgentMessage`.
///
/// Never returns `Err` silently — all parse failures become `Unknown`.
pub fn parse_line(line: &str) -> AgentMessage {
    // Peek at envelope to determine message direction.
    let envelope: RawEnvelope = match serde_json::from_str(line) {
        Ok(e)  => e,
        Err(_) => return AgentMessage::Unknown { raw: line.to_string() },
    };

    match envelope.kind.as_deref() {
        // "kind":"response" indicates a pull response.
        Some("response") => {
            match serde_json::from_str::<PullResponse>(line) {
                Ok(r)  => AgentMessage::Pull(r),
                Err(e) => AgentMessage::Unknown {
                    raw: format!("pull-parse-err: {e} | {line}"),
                },
            }
        }
        // No `kind` field means push event.
        None => {
            match serde_json::from_str::<PushEvent>(line) {
                Ok(ev) => AgentMessage::Push(ev),
                Err(e) => AgentMessage::Unknown {
                    raw: format!("push-parse-err: {e} | {line}"),
                },
            }
        }
        Some(other) => AgentMessage::Unknown {
            raw: format!("unknown-kind: {other} | {line}"),
        },
    }
}

/// Internal TUI channel events.
#[derive(Debug)]
pub enum UiEvent {
    Agent(AgentMessage),
    SpawnError(String),
}