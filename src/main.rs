//! Iter inline interactive coding agent.
//!
//! Output streams to stdout normally and scrolls in terminal history. The input
//! row renders inline with crossterm and does not use an alternate screen.

mod agent;
mod cli;
mod highlight;
mod input;
mod rpc;
mod state;

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::style::{self, Attribute, Color, Stylize};

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

    // Resolve provider + model: flags > interactive picker > env defaults.
    let (resolved_provider, resolved_model) = match (&cli.global.provider, &cli.global.model) {
        (Some(p), Some(m)) => (Some(p.clone()), Some(m.clone())),
        (Some(p), None) => (Some(p.clone()), None),
        (None, Some(m)) => (None, Some(m.clone())),
        (None, None) => pick_provider_model_interactive()?,
    };

    if let Some(provider) = &resolved_provider {
        agent::send_cmd(
            &mut agent_stdin,
            serde_json::json!({
                "id": "set-provider",
                "type": "set_provider",
                "provider": provider,
            }),
        );
        drain_startup(&rx, &mut state, &mut agent_stdin);
    }

    if let Some(model) = &resolved_model {
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

/// What `handle_push_event` wants the event loop to do next.
enum LoopAction {
    /// Keep receiving events.
    Continue,
    /// Agent turn finished — render the buffered response and return.
    Done,
    /// Unrecoverable error — propagate immediately.
    Error(io::Error),
}

/// Handle a single push event, updating `state` and `response_buf` as needed.
///
/// All rendering decisions live here; `stream_response` is just a loop.
fn handle_push_event(
    event: PushEvent,
    state: &mut State,
    response_buf: &mut String,
    stdout: &mut impl Write,
) -> io::Result<LoopAction> {
    match event {
        PushEvent::TextDelta { delta } => {
            response_buf.push_str(&delta);
        }
        PushEvent::ThinkingDelta { delta } => {
            if state.show_thinking {
                crossterm::queue!(
                    stdout,
                    style::PrintStyledContent(
                        delta.with(Color::DarkGrey).attribute(Attribute::Italic)
                    )
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
        PushEvent::AutoRetryStart { attempt, max_attempts, delay_ms, .. } => {
            print_status(
                &format!("retry {attempt}/{max_attempts} in {delay_ms}ms"),
                Color::DarkYellow,
            );
        }
        PushEvent::AutoRetryEnd { success, attempt, final_error } => {
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
            return Ok(LoopAction::Error(io::Error::new(
                io::ErrorKind::Other,
                message,
            )));
        }
        PushEvent::ModelList { models } => {
            state.models = models.into_iter().map(|m| (m.id, m.name)).collect();
        }
        PushEvent::AgentEnd => {
            if !response_buf.is_empty() {
                print_response(response_buf);
            }
            println!();
            return Ok(LoopAction::Done);
        }
        PushEvent::AgentStart | PushEvent::TurnStart | PushEvent::TurnEnd => {}
    }
    Ok(LoopAction::Continue)
}

fn stream_response(
    rx: &Receiver<UiEvent>,
    state: &mut State,
    _agent_stdin: &mut Option<std::process::ChildStdin>,
) -> io::Result<()> {
    let mut stdout = io::stdout();
    let mut response_buf = String::new();

    loop {
        match rx.recv() {
            Ok(UiEvent::Agent(AgentMessage::Push(event))) => {
                match handle_push_event(event, state, &mut response_buf, &mut stdout)? {
                    LoopAction::Continue => {}
                    LoopAction::Done => return Ok(()),
                    LoopAction::Error(e) => return Err(e),
                }
            }
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

/// Render a complete LLM response.
///
/// Splits on fenced code blocks:
/// - Markdown segments  → termimad (headings, bold, lists, etc.)
/// - Code block content → syntect  (syntax-highlighted, printed directly)
fn print_response(text: &str) {
    let mut md_buf = String::new();
    let mut lines = text.split('\n').peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") {
            // Flush accumulated markdown first.
            if !md_buf.is_empty() {
                termimad::print_text(md_buf.trim_end_matches('\n'));
                md_buf.clear();
            }

            let lang = trimmed.trim_start_matches('`').trim();

            // Print the opening fence in dim style.
            println!("\x1b[2m{line}\x1b[0m");

            // Collect and highlight code body.
            let mut body = String::new();
            let mut closed = false;
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    // Print highlighted body.
                    let highlighted = highlight::highlight_code(lang, &body);
                    print!("{highlighted}");
                    // Print closing fence in dim style.
                    println!("\x1b[2m{inner}\x1b[0m");
                    closed = true;
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }

            if !closed {
                // Unclosed fence — print body plain.
                print!("{body}");
            }
        } else {
            md_buf.push_str(line);
            md_buf.push('\n');
        }
    }

    // Flush any remaining markdown.
    if !md_buf.is_empty() {
        termimad::print_text(md_buf.trim_end_matches('\n'));
    }
}

/// Interactive startup picker: select vendor then model.
/// Returns (provider, model) — both optional (Enter skips to env defaults).
fn pick_provider_model_interactive() -> io::Result<(Option<String>, Option<String>)> {
    let providers: &[(&str, &str, &[&str])] = &[
        ("anthropic",  "Anthropic",  &["claude-sonnet-4-20250514", "claude-opus-4-5-20250514", "claude-haiku-3-5-20250514"]),
        ("openai",     "OpenAI",     &["gpt-4o", "gpt-4o-mini", "o3", "o4-mini"]),
        ("google",     "Google",     &["gemini-2.0-flash", "gemini-2.0-flash-lite", "gemini-1.5-pro"]),
        ("deepseek",   "DeepSeek",   &["deepseek-chat", "deepseek-coder"]),
        ("groq",       "Groq",       &["llama-3.3-70b-versatile", "mixtral-8x7b-32768"]),
        ("mistral",    "Mistral",    &["mistral-small-latest", "mistral-large-latest"]),
        ("openrouter", "OpenRouter", &["anthropic/claude-3.5-sonnet", "google/gemma-3-27b-it", "deepseek/deepseek-chat"]),
        ("ollama",     "Ollama",     &["llama3", "mistral", "codellama"]),
    ];

    // --- Vendor picker ---
    println!("\n{}", "Select vendor:".with(Color::DarkCyan).bold());
    for (i, (id, name, _)) in providers.iter().enumerate() {
        println!("  {}  {} {}", format!("[{i}]").with(Color::DarkGrey), name.with(Color::White), id.with(Color::DarkGrey));
    }
    println!("  {}  {}", "[\u{21b5}]".with(Color::DarkGrey), "skip (use env defaults)".with(Color::DarkGrey));
    print!("\n{} ", "vendor >".with(Color::DarkGreen));
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        return Ok((None, None));
    }

    let provider_entry = input.parse::<usize>()
        .ok()
        .and_then(|i| providers.get(i))
        .or_else(|| providers.iter().find(|(id, name, _)| id.eq_ignore_ascii_case(input) || name.eq_ignore_ascii_case(input)));

    let Some((provider_id, provider_name, models)) = provider_entry else {
        println!("  {} unknown vendor '{}', using env defaults", "!".with(Color::DarkYellow), input);
        return Ok((None, None));
    };

    // --- Model picker ---
    println!("\n{} {}:", "Select model for".with(Color::DarkCyan).bold(), provider_name.with(Color::White));
    for (i, m) in models.iter().enumerate() {
        println!("  {}  {}", format!("[{i}]").with(Color::DarkGrey), m.with(Color::White));
    }
    println!("  {}  {}", "[\u{21b5}]".with(Color::DarkGrey), format!("default ({})", models[0]).with(Color::DarkGrey));
    println!("  {}  {}", "[text]".with(Color::DarkGrey), "type any model name".with(Color::DarkGrey));
    print!("\n{} ", "model  >".with(Color::DarkGreen));
    io::stdout().flush()?;

    let mut model_input = String::new();
    io::stdin().read_line(&mut model_input)?;
    let model_input = model_input.trim();

    let model = if model_input.is_empty() {
        models[0].to_string()
    } else if let Ok(i) = model_input.parse::<usize>() {
        models.get(i).copied().unwrap_or(models[0]).to_string()
    } else {
        model_input.to_string()
    };

    println!("\n  {} {}  {}\n",
        "using".with(Color::DarkGrey),
        provider_id.with(Color::DarkCyan),
        model.as_str().with(Color::White).bold(),
    );

    Ok((Some(provider_id.to_string()), Some(model)))
}