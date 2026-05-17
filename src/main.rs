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
use rpc::{AgentMessage, CommandPayload, PushEvent, UiEvent};
use state::State;

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    if let Some(workdir) = &cli.global.workdir {
        std::env::set_current_dir(workdir)?;
    }

    let mut state = State::new();
    state.show_thinking = cli.global.show_thinking;

    // Resolve provider + model + API key BEFORE spawning agent so we can inject env vars.
    let (resolved_provider, resolved_model, resolved_api_key) =
        match (&cli.global.provider, &cli.global.model) {
            (Some(p), Some(m)) => {
                let key = prompt_api_key_if_needed(p);
                (Some(p.clone()), Some(m.clone()), key)
            }
            (Some(p), None) => {
                let key = prompt_api_key_if_needed(p);
                (Some(p.clone()), None, key)
            }
            (None, Some(m)) => {
                let provider = infer_provider_from_model(m);
                let key = provider.as_deref().and_then(|p| prompt_api_key_if_needed(p));
                (provider, Some(m.clone()), key)
            }
            (None, None) => pick_provider_model_interactive()?,
        };

    let (tx, rx) = mpsc::channel::<UiEvent>();
    let config = agent::AgentConfig {
        entry_path:  cli.global.agent_entry.clone(),
        log_dir:     cli.global.log_dir.clone(),
        api_key:     resolved_api_key,
        provider_id: resolved_provider.clone(),
    };
    let mut agent_stdin = agent::spawn_agent(tx, &config);

    if agent_stdin.is_none() {
        return Err(io::Error::new(io::ErrorKind::Other, "failed to spawn agent"));
    }

    agent::send_cmd(
        &mut agent_stdin,
        CommandPayload::GetState { id: "startup".into() },
    );
    drain_startup(&rx, &mut state, &mut agent_stdin);

    if let Some(provider) = &resolved_provider {
        agent::send_cmd(
            &mut agent_stdin,
            CommandPayload::SetProvider {
                id: "set-provider".into(),
                provider: provider.clone(),
            },
        );
        drain_startup(&rx, &mut state, &mut agent_stdin);
    }

    if let Some(model) = &resolved_model {
        agent::send_cmd(
            &mut agent_stdin,
            CommandPayload::SetModel {
                id: "set-model".into(),
                model: model.clone(),
            },
        );
        drain_startup(&rx, &mut state, &mut agent_stdin);
    }

    match cli.command {
        Command::Ask { prompt } if !prompt.is_empty() => {
            let prompt = prompt.join(" ");
            print_agent_header(&state);
            let prompt_id = "prompt-1";
            send_prompt(&mut agent_stdin, prompt_id, prompt);
            stream_response(&rx, &mut state, prompt_id)?;
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
                    CommandPayload::Abort {
                        id: format!("abort-{turn_id}"),
                    },
                );
                print_status("aborted", Color::DarkYellow);
            }
            InputResult::Submit(prompt) => {
                let _ = io::stdout().flush();
                let prompt_id = format!("prompt-{turn_id}");
                send_prompt(agent_stdin, &prompt_id, prompt);
                stream_response(rx, state, &prompt_id)?;
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
        CommandPayload::Prompt {
            id: id.to_string(),
            content: prompt,
        },
    );
}

/// What `handle_push_event` wants the event loop to do next.
enum LoopAction {
    /// Keep receiving events.
    Continue,
    /// Agent turn finished — render the buffered response and return.
    Done,
}

/// Handle a single push event, updating `state` and `response_buf` as needed.
///
/// All rendering decisions live here; `stream_response` is just a loop.
fn handle_push_event(
    event: PushEvent,
    state: &mut State,
    response_buf: &mut String,
    stdout: &mut impl Write,
    expected_id: &str,
) -> io::Result<LoopAction> {
    match event {
        PushEvent::TextDelta { delta } => {
            response_buf.push_str(&delta);
            // Stream live only if no markdown detected yet.
            // Once we see markers, stop streaming and let AgentEnd render.
            let has_markdown = response_buf.contains("```")
                || response_buf.contains("**")
                || response_buf.contains("# ")
                || response_buf.contains("- ")
                || response_buf.contains("* ");
            if !has_markdown {
                print!("{}", delta);
                let _ = io::stdout().flush();
            }
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
            state.tool_start_time = Some(std::time::Instant::now());
            state.tool_calls += 1;
            print_tool_call(&name, &input);
        }
        PushEvent::ToolResult { name, output } => {
            let elapsed_ms = state.tool_start_time
                .take()
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(0);
            state.pending_tool_call = None;
            print_tool_result(&name, &output, elapsed_ms);
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
        PushEvent::Error { id, message } => {
            if !event_id_matches(&id, expected_id) {
                return Ok(LoopAction::Continue);
            }
            print_status(&format!("error: {message}"), Color::DarkRed);
        }
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
        PushEvent::AgentEnd { id, success, error } => {
            if !event_id_matches(&id, expected_id) {
                return Ok(LoopAction::Continue);
            }
            if !response_buf.is_empty() {
                let has_markdown = response_buf.contains("```")
                    || response_buf.contains("**")
                    || response_buf.contains("# ")
                    || response_buf.contains("- ")
                    || response_buf.contains("* ");
                if has_markdown {
                    // Nothing was streamed — render with full formatting now.
                    print_response(response_buf);
                }
                // else: already streamed live, nothing to do.
            }
            if !success {
                let message = error.unwrap_or_else(|| "agent turn failed".into());
                print_status(&message, Color::DarkRed);
            }
            println!();
            return Ok(LoopAction::Done);
        }
        PushEvent::AgentStart => {}
        PushEvent::TurnStart { id } => {
            if !event_id_matches(&id, expected_id) {
                return Ok(LoopAction::Continue);
            }
            state.thinking_token_count = 0;
        }
        PushEvent::TurnEnd { id } => {
            if !event_id_matches(&id, expected_id) {
                return Ok(LoopAction::Continue);
            }
            state.thinking_token_count = 0;
            state.thinking_buf.clear();
        }
    }
    Ok(LoopAction::Continue)
}

fn event_id_matches(id: &Option<String>, expected_id: &str) -> bool {
    id.as_deref().map_or(true, |id| id == expected_id)
}

fn stream_response(
    rx: &Receiver<UiEvent>,
    state: &mut State,
    expected_id: &str,
) -> io::Result<()> {
    let mut stdout = io::stdout();
    let mut response_buf = String::new();

    loop {
        match rx.recv() {
            Ok(UiEvent::Agent(AgentMessage::Push(event))) => {
                match handle_push_event(event, state, &mut response_buf, &mut stdout, expected_id)? {
                    LoopAction::Continue => {}
                    LoopAction::Done => return Ok(()),
                }
            }
            Ok(UiEvent::Agent(AgentMessage::Pull(response))) => {
                let failed_active_prompt = response.command == "prompt"
                    && response.id.as_deref() == Some(expected_id)
                    && !response.success;
                let error = response.error.clone();
                agent::apply_pull_response(state, response);
                if failed_active_prompt {
                    let message = error.unwrap_or_else(|| "prompt rejected".into());
                    print_status(&message, Color::DarkRed);
                    return Ok(());
                }
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
        CommandPayload::GetSessionStats {
            id: format!("stats-{turn_id}"),
        },
    );
    drain_startup(rx, state, agent_stdin);
}

fn print_welcome(state: &State) {
    let model = if state.model_name.is_empty() {
        "unknown"
    } else {
        &state.model_name
    };

    let version = env!("CARGO_PKG_VERSION");

    println!(
        "{}  {}  {}",
        "iter".with(Color::DarkGreen).bold(),
        format!("v{version}").with(Color::DarkGrey),
        model.with(Color::DarkGrey),
    );
    println!(
        "{}",
        "─────────────────────────────────────────".with(Color::DarkGrey)
    );
    println!(
        "  {}",
        "↵ submit   ^k abort   ^u clear   ^c quit".with(Color::DarkGrey),
    );
    println!();
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

fn print_tool_result(name: &str, output: &str, elapsed_ms: u64) {
    let elapsed = if elapsed_ms >= 1000 {
        format!("{:.1}s", elapsed_ms as f64 / 1000.0)
    } else {
        format!("{}ms", elapsed_ms)
    };

    println!(
        "  {} {} · {}",
        "✓".with(Color::DarkGreen),
        name.with(Color::DarkGrey),
        elapsed.with(Color::DarkGrey),
    );

    let lines: Vec<&str> = output.lines().take(8).collect();
    let preview = format_tool_output(&lines.join("\n"));
    for line in preview.lines() {
        println!("    {}", line);
    }
    if output.lines().count() > 8 {
        println!("    {}", format!("… +{} lines", output.lines().count() - 8).with(Color::DarkGrey));
    }
    println!();
}

fn format_tool_output(raw: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
        serde_json::to_string_pretty(&val).unwrap_or_else(|_| raw.to_string())
    } else {
        raw.to_string()
    }
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

/// Interactive startup picker: select vendor, model, and API key.
/// Returns (provider, model, api_key).
fn pick_provider_model_interactive() -> io::Result<(Option<String>, Option<String>, Option<String>)> {
    let providers: &[(&str, &str)] = &[
        ("anthropic",  "Anthropic"),
        ("openai",     "OpenAI"),
        ("google",     "Google"),
        ("deepseek",   "DeepSeek"),
        ("groq",       "Groq"),
        ("mistral",    "Mistral"),
        ("openrouter", "OpenRouter"),
        ("ollama",     "Ollama"),
    ];

    println!("\n{}", "Select vendor:".with(Color::DarkCyan).bold());
    for (i, (id, name)) in providers.iter().enumerate() {
        println!("  {}  {} {}", format!("[{i}]").with(Color::DarkGrey), name.with(Color::White), id.with(Color::DarkGrey));
    }
    println!("  {}  {}", "[\u{21b5}]".with(Color::DarkGrey), "skip (use env defaults)".with(Color::DarkGrey));
    print!("\n{} ", "vendor >".with(Color::DarkGreen));
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        return Ok((None, None, None));
    }

    let provider_entry = input.parse::<usize>()
        .ok()
        .and_then(|i| providers.get(i))
        .or_else(|| providers.iter().find(|(id, name)| id.eq_ignore_ascii_case(input) || name.eq_ignore_ascii_case(input)));

    let Some((provider_id, _provider_name)) = provider_entry else {
        println!("  {} unknown vendor '{}', using env defaults", "!".with(Color::DarkYellow), input);
        return Ok((None, None, None));
    };

    let model = if provider_id == &"ollama" {
        println!();
        "".to_string()
    } else {
        print!("\n{} {}: ", "Model".with(Color::DarkCyan), "(type model name)".with(Color::DarkGrey));
        io::stdout().flush()?;
        let mut model_input = String::new();
        io::stdin().read_line(&mut model_input)?;
        let model_input = model_input.trim().to_string();
        println!("  {} {}  {}\n",
            "using".with(Color::DarkGrey),
            provider_id.with(Color::DarkCyan),
            model_input.as_str().with(Color::White).bold(),
        );
        model_input
    };

    let api_key = prompt_api_key_if_needed(provider_id);

    Ok((Some(provider_id.to_string()), Some(model), api_key))
}

/// Check if the provider already has a key in env; if not, prompt for it (masked).
fn prompt_api_key_if_needed(provider_id: &str) -> Option<String> {
    if provider_id == "ollama" {
        return None;
    }

    let env_var = match provider_id {
        "anthropic"  => "ANTHROPIC_API_KEY",
        "openai"     => "OPENAI_API_KEY",
        "google"     => "GOOGLE_API_KEY",
        "deepseek"   => "DEEPSEEK_API_KEY",
        "groq"       => "GROQ_API_KEY",
        "mistral"    => "MISTRAL_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        _            => return None,
    };

    if let Ok(v) = std::env::var(env_var) {
        if !v.is_empty() {
            return None;
        }
    }

    print!("  {} {} {}: ",
        "API key".with(Color::DarkCyan),
        format!("({env_var})").with(Color::DarkGrey),
        "[Enter to skip]".with(Color::DarkGrey),
    );
    let _ = io::stdout().flush();

    let key = read_masked_line().unwrap_or_default();
    println!();

    if key.is_empty() {
        println!("  {} no key entered — make sure {} is set\n", "!".with(Color::DarkYellow), env_var);
        None
    } else {
        std::env::set_var(env_var, &key);
        Some(key)
    }
}

/// Infer provider ID from model name prefix (mirrors provider.ts inferProvider).
fn infer_provider_from_model(model: &str) -> Option<String> {
    if model.starts_with("claude-") { return Some("anthropic".into()); }
    if model.starts_with("gpt-") || model.starts_with("o1") || model.starts_with("o3") || model.starts_with("o4") {
        return Some("openai".into());
    }
    if model.starts_with("gemini-") { return Some("google".into()); }
    if model.starts_with("deepseek-") { return Some("deepseek".into()); }
    if model.starts_with("llama") || model.starts_with("mixtral") { return Some("groq".into()); }
    if model.starts_with("mistral-") { return Some("mistral".into()); }
    if model.contains('/') { return Some("openrouter".into()); }
    None
}

/// Read a line from stdin without echoing characters (masked password input).
fn read_masked_line() -> io::Result<String> {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
    use crossterm::terminal;
    use crossterm::execute;

    terminal::enable_raw_mode()?;
    execute!(io::stdout(), event::EnableBracketedPaste)?;

    let mut buf = String::new();
    let mut stdout = io::stdout();

    loop {
        match event::read()? {
            Event::Key(KeyEvent { code: KeyCode::Enter, .. }) => break,
            Event::Key(KeyEvent { code: KeyCode::Char('c'), modifiers: KeyModifiers::CONTROL, .. }) => {
                execute!(stdout, event::DisableBracketedPaste)?;
                terminal::disable_raw_mode()?;
                return Err(io::Error::new(io::ErrorKind::Interrupted, "ctrl-c"));
            }
            Event::Key(KeyEvent { code: KeyCode::Backspace, .. }) => {
                if buf.pop().is_some() {
                    print!("\x08 \x08");
                    let _ = stdout.flush();
                }
            }
            Event::Key(KeyEvent { code: KeyCode::Char(c), modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT, .. }) => {
                buf.push(c);
                print!("*");
                let _ = stdout.flush();
            }
            Event::Paste(text) => {
                let stars: String = "*".repeat(text.chars().count());
                buf.push_str(&text);
                print!("{stars}");
                let _ = stdout.flush();
            }
            _ => {}
        }
    }

    execute!(stdout, event::DisableBracketedPaste)?;
    terminal::disable_raw_mode()?;
    Ok(buf)
}
