use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;
use std::fs;

use crate::rpc::{self, AgentMessage, PushEvent, StateData, UiEvent};
use crate::state::{App, ModelStatus};

pub fn spawn_agent(tx: Sender<UiEvent>) -> Option<std::process::ChildStdin> {
    let agent_path = std::path::Path::new("agent/src/index.ts");

    // Mirror the agent's stderr to a log file so raw Bun/Node error output
    // never reaches the terminal and corrupts the alternate screen.
    let log_dir = std::path::Path::new("agent/logs");
    let _ = fs::create_dir_all(log_dir);
    let stderr_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("tui.log"))
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());

    let mut child = match Command::new("bun")
        .arg("run").arg(agent_path)
        .env("OPENROUTER_API_KEY", std::env::var("OPENROUTER_API_KEY").unwrap_or_default())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr_file)
        .spawn()
    {
        Ok(c)  => c,
        Err(e) => { let _ = tx.send(rpc::UiEvent::SpawnError(e.to_string())); return None; }
    };

    let stdout = child.stdout.take().expect("child stdout");
    let stdin  = child.stdin.take().expect("child stdin");

    let tx_clone = tx.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().flatten() {
            let msg = rpc::parse_line(&line);
            let _ = tx_clone.send(rpc::UiEvent::Agent(msg));
        }
    });
    Some(stdin)
}

pub fn send_cmd(agent_stdin: &mut Option<std::process::ChildStdin>, payload: serde_json::Value) {
    if let Some(ref mut stdin) = agent_stdin {
        let _ = writeln!(stdin, "{}", payload);
    }
}

pub fn handle_agent_msg(
    app:         &mut App,
    agent_stdin: &mut Option<std::process::ChildStdin>,
    msg:         AgentMessage,
) {
    match msg {
        AgentMessage::Push(ev) => match ev {
            PushEvent::AgentStart => {}

            PushEvent::TurnStart => {
                app.start_streaming();
            }

            PushEvent::TextDelta { delta } => {
                app.push_assistant_delta(delta);
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
        },

        AgentMessage::Pull(resp) => {
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

        AgentMessage::Unknown { raw } => {
            app.push_system(format!("[rpc] unparsed: {raw}"));
        }
    }
}