//! Agent process management: spawns bun and handles JSONL messages.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use crate::rpc::{self, AgentMessage, PushEvent, UiEvent};
use crate::state::State;

const LOG_FILE: &str = "agent.log";

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub entry_path: PathBuf,
    pub log_dir:    PathBuf,
}

// ── Spawn ─────────────────────────────────────────────────────────────────────

pub fn spawn_agent(tx: Sender<UiEvent>, config: &AgentConfig) -> Option<std::process::ChildStdin> {
    let _        = fs::create_dir_all(&config.log_dir);
    let log_file = fs::OpenOptions::new()
        .create(true).append(true)
        .open(config.log_dir.join(LOG_FILE))
        .map(Stdio::from)
        .ok()?;

    let mut child = Command::new("bun")
        .arg("run")
        .arg(&config.entry_path)
        .env("OPENROUTER_API_KEY", std::env::var("OPENROUTER_API_KEY").unwrap_or_default())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log_file)
        .spawn()
        .map_err(|e| { let _ = tx.send(UiEvent::SpawnError(e.to_string())); })
        .ok()?;

    let stdout = child.stdout.take().expect("child stdout");
    let stdin  = child.stdin.take().expect("child stdin");

    let tx2 = tx.clone();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().flatten() {
            let _ = tx2.send(UiEvent::Agent(rpc::parse_line(&line)));
        }
    });

    Some(stdin)
}

// ── Commands ──────────────────────────────────────────────────────────────────

pub fn send_cmd(stdin: &mut Option<std::process::ChildStdin>, payload: serde_json::Value) {
    if let Some(ref mut s) = stdin {
        let _ = writeln!(s, "{payload}");
    }
}

// ── Message handler (used outside streaming loop) ─────────────────────────────

pub fn handle_agent_msg_state(
    state:       &mut State,
    _agent_stdin: &mut Option<std::process::ChildStdin>,
    msg:         AgentMessage,
) {
    match msg {
        AgentMessage::Push(ev) => match ev {
            PushEvent::ModelList { models } => {
                state.models = models.into_iter().map(|m| (m.id, m.name)).collect();
            }
            _ => {}
        },
        AgentMessage::Pull(resp) => {
            apply_pull_response(state, resp);
        }
        AgentMessage::Unknown { raw } => eprintln!("[rpc] {raw}"),
    }
}

pub fn apply_pull_response(state: &mut State, resp: rpc::PullResponse) {
    if !resp.success { return; }
    match resp.command.as_str() {
        "get_state" => {
            if let Some(data) = resp.data {
                if let Ok(s) = serde_json::from_value::<rpc::StateData>(data) {
                    state.model_name  = s.model_name;
                    state.model_limit = s.model_limit;
                    state.model_temp  = s.temp;
                }
            }
        }
        "get_session_stats" => {
            if let Some(data) = resp.data {
                if let Ok(s) = serde_json::from_value::<rpc::SessionStatsData>(data) {
                    state.context_pct = s.context_usage.percent;
                    state.cost        = s.cost;
                    state.turns       = s.turns;
                }
            }
        }
        "set_model" => {
            if let Some(data) = resp.data {
                if let Ok(s) = serde_json::from_value::<rpc::SetModelData>(data) {
                    state.model_name  = s.model_name;
                    state.model_limit = s.model_limit;
                }
            }
        }
        _ => {}
    }
}
