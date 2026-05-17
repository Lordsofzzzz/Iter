//! RPC wire protocol types for CLI ↔ Agent communication.
//!
//! The protocol uses JSONL (one JSON object per line) over stdout/stdin.
//! Two message directions exist:
//!   - **Push**: Agent → CLI (unprompted events like text deltas)
//!   - **Pull**: CLI → Agent → CLI (request/response pattern)

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

// ============================================================================
// Commands: CLI -> Agent
// ============================================================================

/// Typed commands sent from the CLI to the agent.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandPayload {
    GetState { id: String },
    GetSessionStats { id: String },
    SetProvider { id: String, provider: String },
    SetModel { id: String, model: String },
    Prompt { id: String, content: String },
    Abort { id: String },
    Clear { id: String },
}

// ============================================================================
// Push Events: Agent → CLI (unprompted)
// ============================================================================

/// Unprompted events from the agent to the CLI.
/// Identified by absence of the `kind` field.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PushEvent {
    AgentStart,
    TurnStart {
        #[serde(default)]
        id: Option<String>,
    },
    TurnEnd {
        #[serde(default)]
        id: Option<String>,
    },
    AgentEnd {
        #[serde(default)]
        id: Option<String>,
        #[serde(default = "default_success")]
        success: bool,
        #[serde(default)]
        error: Option<String>,
    },
    TextDelta { delta: String },
    ThinkingDelta { delta: String },
    Error {
        #[serde(default)]
        id: Option<String>,
        message: String,
    },
    Cooldown { wait_ms: u64, retries_left: u32 },
    RetryResult { success: bool, attempt: u32 },
    AutoRetryStart {
        attempt: u32,
        #[serde(rename = "maxAttempts")]
        max_attempts: u32,
        #[serde(rename = "delayMs")]
        delay_ms: u64,
        #[serde(rename = "errorMessage")]
        error_message: String,
    },
    AutoRetryEnd {
        success: bool,
        attempt: u32,
        #[serde(rename = "finalError")]
        final_error: Option<String>,
    },
    ToolCall { name: String, input: String },   // ← new
    ToolResult { name: String, output: String }, // ← new
    ToolUpdate { tool_call_id: String, delta: String }, // ← live streaming delta
    ModelList { models: Vec<ModelEntry> },
    ProviderChanged {
        provider_id: String,
        provider_name: String,
    },
}

fn default_success() -> bool {
    true
}

/// A single model entry from the dynamic model list.
#[derive(Debug, Deserialize, Clone)]
pub struct ModelEntry {
    pub id:   String,
    pub name: String,
}

// ============================================================================
// Pull Responses: CLI → Agent → CLI
// ============================================================================

/// Response from the agent to a CLI command.
/// Always has `kind: "response"` which distinguishes it from push events.
#[derive(Debug, Deserialize)]
pub struct PullResponse {
    pub command: String,
    #[allow(dead_code)]
    pub id:      Option<String>,
    pub success: bool,
    pub error:   Option<String>,
    pub data:    Option<serde_json::Value>,
}

// ============================================================================
// Parsed Data Shapes (for pull response `data` field)
// ============================================================================

/// Data returned in a successful `set_model` response.
#[derive(Debug, Deserialize)]
pub struct SetModelData {
    pub model_name:  String,
    pub model_limit: u32,
}

/// Data returned in a successful `set_provider` response.
#[derive(Debug, Deserialize)]
pub struct SetProviderData {
    pub provider: String,
    pub model:    Option<String>,
}

/// Data for the `get_state` command response.
#[derive(Debug, Deserialize)]
pub struct StateData {
    pub model_name:   String,
    pub model_limit:  u32,
    pub temp:         f32,
    #[allow(dead_code)]
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

/// Full session statistics for the CLI.
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
        Err(_) => {
            return AgentMessage::Unknown { raw: line.to_string() };
        }
    };

    match envelope.kind.as_deref() {
        // "kind":"response" indicates a pull response.
        Some("response") => {
            match serde_json::from_str::<PullResponse>(line) {
                Ok(r)  => AgentMessage::Pull(r),
                Err(_) => {
                    AgentMessage::Unknown {
                        raw: format!("pull-parse-err: {line}"),
                    }
                }
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

/// Internal CLI channel events.
#[derive(Debug)]
pub enum UiEvent {
    Agent(AgentMessage),
    SpawnError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_prompt_command_with_request_id() {
        let command = CommandPayload::Prompt {
            id: "prompt-7".into(),
            content: "hello".into(),
        };

        let json = serde_json::to_value(command).unwrap();

        assert_eq!(json["type"], "prompt");
        assert_eq!(json["id"], "prompt-7");
        assert_eq!(json["content"], "hello");
    }

    #[test]
    fn parses_terminal_agent_end_contract() {
        let msg = parse_line(r#"{"type":"agent_end","id":"prompt-7","success":false,"error":"Agent busy"}"#);

        match msg {
            AgentMessage::Push(PushEvent::AgentEnd { id, success, error }) => {
                assert_eq!(id.as_deref(), Some("prompt-7"));
                assert!(!success);
                assert_eq!(error.as_deref(), Some("Agent busy"));
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn parses_provider_changed_event() {
        let msg = parse_line(r#"{"type":"provider_changed","provider_id":"openrouter","provider_name":"OpenRouter"}"#);

        match msg {
            AgentMessage::Push(PushEvent::ProviderChanged { provider_id, provider_name }) => {
                assert_eq!(provider_id, "openrouter");
                assert_eq!(provider_name, "OpenRouter");
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }
}
