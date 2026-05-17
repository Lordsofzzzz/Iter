//! Agent process management: spawns bun and handles JSONL messages.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use serde::Serialize;

use crate::rpc::{self, AgentMessage, PushEvent, UiEvent};
use crate::state::State;

const LOG_FILE: &str = "agent.log";

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub entry_path:  PathBuf,
    pub log_dir:     PathBuf,
    pub api_key:     Option<String>,
    pub provider_id: Option<String>,
}

pub fn spawn_agent(tx: Sender<UiEvent>, config: &AgentConfig) -> Option<std::process::ChildStdin> {
    let _        = fs::create_dir_all(&config.log_dir);
    let log_file = fs::OpenOptions::new()
        .create(true).append(true)
        .open(config.log_dir.join(LOG_FILE))
        .map(Stdio::from)
        .ok()?;

    let provider_env_map: &[(&str, &str)] = &[
        ("anthropic",  "ANTHROPIC_API_KEY"),
        ("openai",     "OPENAI_API_KEY"),
        ("google",     "GOOGLE_API_KEY"),
        ("deepseek",   "DEEPSEEK_API_KEY"),
        ("groq",       "GROQ_API_KEY"),
        ("mistral",    "MISTRAL_API_KEY"),
        ("openrouter", "OPENROUTER_API_KEY"),
    ];

    let mut cmd = Command::new("bun");
    cmd.arg("run").arg(&config.entry_path);

    for (provider, env_var) in provider_env_map {
        let key = if config.provider_id.as_deref() == Some(provider) {
            if let Some(ref k) = config.api_key {
                k.clone()
            } else {
                std::env::var(env_var).unwrap_or_default()
            }
        } else {
            std::env::var(env_var).unwrap_or_default()
        };
        cmd.env(env_var, key);
    }

    let mut child = cmd
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

pub fn send_cmd<T: Serialize>(stdin: &mut Option<std::process::ChildStdin>, payload: T) {
    if let Some(ref mut s) = stdin {
        if let Ok(line) = serde_json::to_string(&payload) {
            let _ = writeln!(s, "{line}");
        }
    }
}

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
            PushEvent::ProviderChanged { provider_id, provider_name } => {
                state.provider_name = if provider_name.is_empty() {
                    provider_id
                } else {
                    provider_name
                };
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
                    state.context_pct         = s.context_usage.percent;
                    state.context_tokens     = s.context_usage.tokens;
                    state.cost                = s.cost;
                    state.turns               = s.turns;
                    state.tokens_input        = s.tokens.input;
                    state.tokens_output       = s.tokens.output;
                    state.tokens_cache_read   = s.tokens.cache_read;
                    state.tokens_cache_write  = s.tokens.cache_write;
                    state.tokens_total        = s.tokens.total;
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
        "set_provider" => {
            if let Some(data) = resp.data {
                if let Ok(s) = serde_json::from_value::<rpc::SetProviderData>(data) {
                    state.provider_name = s.provider.clone();
                    state.model_name = s.model.clone().unwrap_or_default();
                }
            }
        }
        _ => {}
    }
}
