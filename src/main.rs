//! Iter inline interactive coding agent.
//!
//! Output streams to stdout normally and scrolls in terminal history. The input
//! row renders inline with crossterm and does not use an alternate screen.

mod agent;
mod cli;
mod input;
mod rpc;
mod state;

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::style::{self, Color, Stylize};

use cli::{Cli, Command};
use input::{InputBox, InputResult};
use rpc::{AgentMessage, PushEvent, UiEvent};
use state::State;

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    if let Some(workdir) = &cli.global.workdir {
        std::env::set_current_dir(workdir)?;
    }

    let mut state = State::new();
    state.show_thinking = cli.global.show_thinking;

    let (tx, rx) = mpsc::channel::<UiEvent>();
    let config = agent::AgentConfig {
        entry_path: cli.global.agent_entry.clone(),
        log_dir: cli.global.log_dir.clone(),
    };
    let mut agent_stdin = agent::spawn_agent(tx, &config);

    if agent_stdin.is_none() {
        return Err(io::Error::new(io::ErrorKind::Other, "failed to spawn agent"));
    }

    agent::send_cmd(
        &mut agent_stdin,
        serde_json::json!({
            "id": "startup",
            "type": "get_state",
        }),
    );
    drain_startup(&rx, &mut state, &mut agent_stdin);

    if let Some(model) = &cli.global.model {
        agent::send_cmd(
            &mut agent_stdin,
            serde_json::json!({
                "id": "set-model",
                "type": "set_model",
                "model": model,
            }),
        );
        drain_startup(&rx, &mut state, &mut agent_stdin);
    }

    match cli.command {
        Command::Ask { prompt } if !prompt.is_empty() => {
            let prompt = prompt.join(" ");
            print_agent_header(&state);
            send_prompt(&mut agent_stdin, "prompt-1", prompt);
            stream_response(&rx, &mut state, &mut agent_stdin)?;
        }
        Command::Ask { .. } => {
            interactive_loop(&rx, &mut state, &mut agent_stdin)?;
        }
    }

    Ok(())
}

fn interactive_loop(
    rx: &Receiver<UiEvent>,
    state: &mut State,
    agent_stdin: &mut Option<std::process::ChildStdin>,
) -> io::Result<()> {
    print_welcome(state);

    let mut input = InputBox::new();
    let mut turn_id = 1usize;

    loop {
        let hint = "describe your task...  (ctrl-k abort, ctrl-c quit)";
        match input.read(hint, state)? {
            InputResult::Quit => {
                eprintln!();
                break;
            }
            InputResult::Abort => {
                agent::send_cmd(
                    agent_stdin,
                    serde_json::json!({
                        "id": format!("abort-{turn_id}"),
                        "type": "abort",
                    }),
                );
                print_status("aborted", Color::DarkYellow);
            }
            InputResult::Submit(prompt) => {
                let _ = io::stdout().flush();
                send_prompt(agent_stdin, &format!("prompt-{turn_id}"), prompt);
                stream_response(rx, state, agent_stdin)?;
                refresh_stats(rx, state, agent_stdin, turn_id);
                turn_id += 1;
            }
        }
    }

    Ok(())
}

fn send_prompt(
    agent_stdin: &mut Option<std::process::ChildStdin>,
    id: &str,
    prompt: String,
) {
    agent::send_cmd(
        agent_stdin,
        serde_json::json!({
            "id": id,
            "type": "prompt",
            "content": prompt,
        }),
    );
}

fn stream_response(
    rx: &Receiver<UiEvent>,
    state: &mut State,
    _agent_stdin: &mut Option<std::process::ChildStdin>,
) -> io::Result<()> {
    let mut stdout = io::stdout();

    loop {
        match rx.recv() {
            Ok(UiEvent::Agent(AgentMessage::Push(event))) => match event {
                PushEvent::TextDelta { delta } => {
                    print!("{delta}");
                    stdout.flush()?;
                }
                PushEvent::ThinkingDelta { delta } => {
                    if state.show_thinking {
                        crossterm::queue!(
                            stdout,
                            style::PrintStyledContent(delta.with(Color::DarkGrey))
                        )?;
                        stdout.flush()?;
                    }
                }
                PushEvent::ToolCall { name, input } => {
                    state.pending_tool_call = Some((name.clone(), input.clone()));
                    state.tool_calls += 1;
                    print_tool_call(&name, &input);
                }
                PushEvent::ToolResult { name, output } => {
                    state.pending_tool_call = None;
                    print_tool_result(&name, &output);
                }
                PushEvent::ToolUpdate { .. } => {}
                PushEvent::Cooldown { wait_ms, retries_left } => {
                    let secs = (wait_ms + 999) / 1000;
                    print_status(
                        &format!("rate limited: waiting {secs}s ({retries_left} retries left)"),
                        Color::DarkYellow,
                    );
                }
                PushEvent::RetryResult { success, attempt } => {
                    if !success {
                        print_status(&format!("retry attempt {attempt} failed"), Color::DarkRed);
                    }
                }
                PushEvent::AutoRetryStart {
                    attempt,
                    max_attempts,
                    delay_ms,
                    ..
                } => {
                    print_status(
                        &format!("retry {attempt}/{max_attempts} in {delay_ms}ms"),
                        Color::DarkYellow,
                    );
                }
                PushEvent::AutoRetryEnd {
                    success,
                    attempt,
                    final_error,
                } => {
                    if !success {
                        print_status(
                            &format!(
                                "failed after {attempt} attempts: {}",
                                final_error.unwrap_or_else(|| "unknown".into())
                            ),
                            Color::DarkRed,
                        );
                    }
                }
                PushEvent::Error { message } => {
                    print_status(&format!("error: {message}"), Color::DarkRed);
                    return Ok(());
                }
                PushEvent::ModelList { models } => {
                    state.models = models.into_iter().map(|m| (m.id, m.name)).collect();
                }
                PushEvent::AgentEnd => {
                    println!();
                    return Ok(());
                }
                PushEvent::AgentStart | PushEvent::TurnStart | PushEvent::TurnEnd => {}
            },
            Ok(UiEvent::Agent(AgentMessage::Pull(response))) => {
                agent::apply_pull_response(state, response);
            }
            Ok(UiEvent::Agent(AgentMessage::Unknown { raw })) => {
                eprintln!("\n[rpc] {raw}");
            }
            Ok(UiEvent::SpawnError(message)) => {
                return Err(io::Error::new(io::ErrorKind::Other, message));
            }
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "agent disconnected",
                ));
            }
        }
    }
}

fn drain_startup(
    rx: &Receiver<UiEvent>,
    state: &mut State,
    agent_stdin: &mut Option<std::process::ChildStdin>,
) {
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(UiEvent::Agent(msg)) => agent::handle_agent_msg_state(state, agent_stdin, msg),
            Ok(UiEvent::SpawnError(e)) => eprintln!("spawn: {e}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn refresh_stats(
    rx: &Receiver<UiEvent>,
    state: &mut State,
    agent_stdin: &mut Option<std::process::ChildStdin>,
    turn_id: usize,
) {
    agent::send_cmd(
        agent_stdin,
        serde_json::json!({
            "id": format!("stats-{turn_id}"),
            "type": "get_session_stats",
        }),
    );
    drain_startup(rx, state, agent_stdin);
}

fn print_welcome(state: &State) {
    let model = if state.model_name.is_empty() {
        "unknown"
    } else {
        &state.model_name
    };

    println!(
        "{}  {}",
        "iter".with(Color::DarkGreen).bold(),
        model.with(Color::DarkGrey),
    );
    println!(
        "{}",
        "-----------------------------------------".with(Color::DarkGrey)
    );
}

fn print_agent_header(state: &State) {
    println!(
        "\n{} {}",
        ">".with(Color::DarkGreen),
        state.model_name.as_str().with(Color::DarkGrey),
    );
}

fn print_tool_call(name: &str, input: &str) {
    let preview = truncate_chars(input, 120);
    println!(
        "\n  {} {} {}",
        "tool".with(Color::DarkCyan),
        name.with(Color::Cyan),
        preview.with(Color::DarkGrey),
    );
}

fn print_tool_result(name: &str, output: &str) {
    let lines: Vec<&str> = output.lines().take(6).collect();
    let preview = lines.join("\n    ");
    let suffix = if output.lines().count() > 6 {
        "\n    ..."
    } else {
        ""
    };

    println!(
        "  {} {}\n    {}{}",
        "done".with(Color::DarkGreen),
        name.with(Color::DarkGrey),
        preview,
        suffix,
    );
}

fn print_status(msg: &str, color: Color) {
    println!("\n  {}", msg.with(color));
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}