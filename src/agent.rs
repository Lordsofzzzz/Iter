//! Agent process management and message handling.
//!
//! Spawns the TypeScript agent as a child process and handles bidirectional
//! JSONL communication over stdin/stdout.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use crate::rpc::{self, AgentMessage, PushEvent, StateData, UiEvent};
use crate::state::ModelStatus;

// ============================================================================
// Constants
// ============================================================================

/// Relative path to the TypeScript agent entry point.
const AGENT_ENTRY_PATH: &str = "agent/src/index.ts";

/// Directory for agent logs (stderr redirection).
const LOG_DIR: &str = "agent/logs";

/// Log file name within the logs directory.
const LOG_FILE: &str = "tui.log";

// ============================================================================
// Public API
// ============================================================================

/// Spawns the TypeScript agent as a child process.
///
/// Redirects stderr to a log file to prevent corrupting the TUI's alternate
/// screen. Returns the child stdin handle for sending commands.
pub fn spawn_agent(tx: Sender<UiEvent>) -> Option<std::process::ChildStdin> {
    let log_path = setup_logging()?;

    let mut child = match Command::new("bun")
        .arg("run")
        .arg(AGENT_ENTRY_PATH)
        .env("OPENROUTER_API_KEY", std::env::var("OPENROUTER_API_KEY").unwrap_or_default())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log_path)
        .spawn()
    {
        Ok(c)  => c,
        Err(e) => {
            let _ = tx.send(UiEvent::SpawnError(e.to_string()));
            return None;
        }
    };

    let stdout = child.stdout.take().expect("child stdout");
    let stdin  = child.stdin.take().expect("child stdin");

    // Spawn thread to read agent's stdout (push events).
    let tx_clone = tx.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().flatten() {
            // eprintln!("[AGENT→TUI] {}", line);  // Debug logging - disabled to prevent TUI corruption
            let msg = rpc::parse_line(&line);
            let _ = tx_clone.send(UiEvent::Agent(msg));
        }
    });

    Some(stdin)
}

/// Sends a JSON command to the agent over stdin.
pub fn send_cmd(agent_stdin: &mut Option<std::process::ChildStdin>, payload: serde_json::Value) {
    // eprintln!("[TUI→AGENT] {}", payload);  // Debug logging - disabled to prevent TUI corruption
    if let Some(ref mut stdin) = agent_stdin {
        let _ = writeln!(stdin, "{}", payload);
    }
}

/// Sends abort command to stop the current streaming operation.
pub fn send_abort(agent_stdin: &mut Option<std::process::ChildStdin>) {
    send_cmd(agent_stdin, serde_json::json!({ "type": "abort" }));
}

/// Handles incoming agent messages, updating app state accordingly.
pub fn handle_agent_msg(
    app:         &mut crate::state::App,
    agent_stdin: &mut Option<std::process::ChildStdin>,
    msg:         AgentMessage,
) {
    match msg {
        AgentMessage::Push(ev) => handle_push_event(app, agent_stdin, ev),
        AgentMessage::Pull(resp) => handle_pull_response(app, agent_stdin, resp),
        AgentMessage::Unknown { raw } => {
            app.push_system(format!("[rpc] unparsed: {raw}"));
        }
    }
}

// ============================================================================
// Private Helpers
// ============================================================================

/// Sets up logging directory and returns stderr file handle.
fn setup_logging() -> Option<Stdio> {
    let log_dir = std::path::Path::new(LOG_DIR);
    let _ = fs::create_dir_all(log_dir);

    let log_path = log_dir.join(LOG_FILE);
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map(Stdio::from)
        .ok()
}

/// Handles push events from the agent (unprompted).
fn handle_push_event(
    app:         &mut crate::state::App,
    agent_stdin: &mut Option<std::process::ChildStdin>,
    ev:          PushEvent,
) {
    match ev {
        PushEvent::AgentStart => {}

        PushEvent::TurnStart => {
            app.start_streaming();
        }

        PushEvent::TextDelta { delta } => {
            app.push_assistant_delta(delta);
            app.scroll_to_bottom();
        }

        PushEvent::ThinkingDelta { delta } => {
            app.push_thinking_delta(delta);
            app.scroll_to_bottom();
        }

        PushEvent::TurnEnd => {}

        PushEvent::Cooldown { wait_ms, retries_left } => {
            app.upsert_rate_limit(wait_ms, retries_left);
            app.model_status = ModelStatus::Cooldown;
        }

        PushEvent::RetryResult { success, attempt } => {
            app.clear_rate_limit();
            if !success {
                app.push_system(format!(
                    "rate limit: failed after {} attempt{}",
                    attempt,
                    if attempt == 1 { "" } else { "s" }
                ));
            }
        }

        PushEvent::AgentEnd => {
            app.clear_rate_limit();
            app.end_streaming();
            app.turns += 1;
            // Request updated session stats after turn ends.
            send_cmd(agent_stdin, serde_json::json!({
                "id":   format!("stats-{}", app.turns),
                "type": "get_session_stats",
            }));
        }

        PushEvent::Error { message } => {
            app.clear_rate_limit();
            app.push_system(format!("error: {message}"));
            app.set_error();
        }

        PushEvent::ToolCall { name, input } => {
            app.push_system(format!("▶ {name}({input})"));
            app.tool_calls += 1;
        }

        PushEvent::ToolResult { name, output } => {
            app.push_system(format!("◀ {name}: {output}"));
        }
    }
}

/// Handles pull responses from the agent (replies to TUI commands).
fn handle_pull_response(
    app:         &mut crate::state::App,
    _agent_stdin: &mut Option<std::process::ChildStdin>,
    resp:        rpc::PullResponse,
) {
    if !resp.success {
        let err = resp.error.unwrap_or_else(|| "unknown error".into());
        app.push_system(format!("[{}] failed: {}", resp.command, err));
        return;
    }

    match resp.command.as_str() {
        "get_state" => {
            if let Some(data) = resp.data {
                if let Ok(s) = serde_json::from_value::<StateData>(data) {
                    app.update_model_info(s.model_name, s.model_limit, s.temp);
                }
            }
        }
        "get_session_stats" => {
            if let Some(data) = resp.data {
                if let Ok(s) = serde_json::from_value::<rpc::SessionStatsData>(data) {
                    app.update_tokens(
                        s.tokens.input,
                        s.tokens.output,
                        s.tokens.cache_read,
                        s.tokens.cache_write,
                        s.tokens.total,
                        s.context_usage.limit,
                    );
                    app.context_pct = s.context_usage.percent;
                    app.cost        = s.cost;
                    app.turns       = s.turns;
                }
            }
        }
        "prompt" | "abort" | "clear" => {}
        other => {
            app.push_system(format!("[rpc] unknown response: {other}"));
        }
    }
}