mod agent;
mod agent_event;
mod cli;
mod config;
mod context;
mod hooks;
mod input;
mod state;
mod tools;

use std::io::{self, Write};
use std::time::Instant;

use clap::Parser;
use crossterm::style::{self, Color, Stylize};
use crossterm::terminal;
use tokio::sync::mpsc;

use agent_event::{AgentEvent, TuiCommand};
use cli::Cli;
use input::{format_tokens, InputBox, InputResult};
use state::State;

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {{
        if std::env::var("ITER_PROFILE").is_ok() {
            let _ = writeln!(std::io::stderr(), "[profile] {}", format_args!($($arg)*));
        }
    }};
}

use termimad::MadSkin;

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    if let Some(workdir) = &cli.global.workdir {
        std::env::set_current_dir(workdir)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    }

    let model = cli.global.model.unwrap_or_else(|| "deepseek/deepseek-v4-flash:free".into());
    let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
    let api_key = if api_key.is_empty() {
        eprint!("OPENROUTER_API_KEY not set. Enter key: ");
        let mut key = String::new();
        io::stdin().read_line(&mut key).ok();
        key.trim().to_string()
    } else {
        api_key
    };

    let mut state = State::new();
    state.model_name = model.clone();
    state.provider_name = "openrouter".into();
    state.show_thinking = cli.global.show_thinking;

    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(256);
    let (cmd_tx, cmd_rx) = mpsc::channel::<TuiCommand>(16);

    let agent_config = agent::AgentConfig {
        model: model.clone(),
        api_key,
        system_prompt: format!(
            "You are a helpful coding assistant. Current directory: {}. OS: {}.",
            cwd,
            std::env::consts::OS,
        ),
    };

    let hooks: Vec<Box<dyn hooks::AgentHook>> = vec![
        Box::new(hooks::CompactContextHook::new(128_000)),
    ];

    rt.spawn(async move {
        agent::run_agent_loop(agent_config, event_tx, cmd_rx, hooks).await;
    });

    let initial_prompt = match &cli.command {
        cli::Command::Ask { prompt } if !prompt.is_empty() => Some(prompt.join(" ")),
        _ => None,
    };

    interactive_loop(&mut state, &rt, &mut event_rx, cmd_tx, initial_prompt)
}

fn interactive_loop(
    state: &mut State,
    rt: &tokio::runtime::Runtime,
    event_rx: &mut mpsc::Receiver<AgentEvent>,
    cmd_tx: mpsc::Sender<TuiCommand>,
    initial_prompt: Option<String>,
) -> io::Result<()> {
    if initial_prompt.is_none() {
        println!(
            "{}  {}  {}",
            "iter".with(Color::DarkGreen).bold(),
            "v0.2.0".with(Color::DarkGrey),
            state.model_name.as_str().with(Color::DarkGrey),
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

    if let Some(prompt) = initial_prompt {
        let _ = io::stdout().flush();
        rt.block_on(handle_turn(event_rx, &cmd_tx, prompt, state));
        println!();
        print_status_bar(state);
        return Ok(());
    }

    let mut input = InputBox::new();

    loop {
        match input.read("describe your task...", state)? {
            InputResult::Quit => {
                eprintln!();
                break;
            }
            InputResult::Abort => {
                let _ = cmd_tx.try_send(TuiCommand::Abort);
            }
            InputResult::Submit(prompt) => {
                let _ = io::stdout().flush();
                if !handle_slash_command(&prompt, &cmd_tx, state) {
                    rt.block_on(handle_turn(event_rx, &cmd_tx, prompt, state));
                    print_status_bar(state);
                }
            }
        }
    }

    Ok(())
}

async fn handle_turn(
    event_rx: &mut mpsc::Receiver<AgentEvent>,
    cmd_tx: &mpsc::Sender<TuiCommand>,
    prompt: String,
    state: &mut State,
) {
    let t0 = Instant::now();
    let _ = cmd_tx.send(TuiCommand::Prompt(prompt)).await;
    let mut first_event = true;
    let mut first_token_time: Option<u64> = None;

    let mut current_text_block = String::new();
    let mut last_rendered_lines: Vec<String> = Vec::new();
    let mut last_char = '\n';
    let skin = MadSkin::default();

    while let Some(event) = event_rx.recv().await {
        let elapsed = t0.elapsed().as_millis() as u64;
        if first_event {
            trace!("first event received at {}ms", elapsed);
            first_event = false;
        }

        match event {
            AgentEvent::TextDelta(delta) => {
                if first_token_time.is_none() {
                    first_token_time = Some(elapsed);
                    trace!("first text delta at {}ms", elapsed);
                }
                
                if current_text_block.is_empty() && last_char != '\n' {
                    println!();
                    last_char = '\n';
                }

                current_text_block.push_str(&delta);

                let term_width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80) as usize;
                let t_mad = Instant::now();
                let fmt_text = skin.text(&current_text_block, Some(term_width));
                let rendered = format!("{}", fmt_text);
                let mad_us = t_mad.elapsed().as_micros();

                let new_lines: Vec<String> = if rendered.is_empty() {
                    Vec::new()
                } else {
                    rendered
                        .strip_suffix('\n')
                        .unwrap_or(&rendered)
                        .split('\n')
                        .map(|s| s.to_string())
                        .collect()
                };

                let mut common_len = 0;
                for (old, new) in last_rendered_lines.iter().zip(new_lines.iter()) {
                    if old == new {
                        common_len += 1;
                    } else {
                        break;
                    }
                }

                let go_up = last_rendered_lines.len().saturating_sub(common_len);
                if go_up > 0 {
                    print!("\x1b[{}A", go_up);
                }
                
                if go_up > 0 || common_len < new_lines.len() {
                    print!("\r\x1b[0J");
                    for line in new_lines.iter().skip(common_len) {
                        println!("{}", line);
                    }
                    io::stdout().flush().ok();
                }
                let line_count = new_lines.len();
                last_rendered_lines = new_lines;
                trace!("render: {}µs ({} chars, {} lines)", mad_us, current_text_block.len(), line_count);
            }
            AgentEvent::ToolCall { name, input } => {
                current_text_block.clear();
                last_rendered_lines.clear();
                last_char = '\n';
                
                if first_token_time.is_none() {
                    first_token_time = Some(elapsed);
                    trace!("first tool call at {}ms", elapsed);
                }
                state.pending_tool_call = Some((name.clone(), input.clone()));
                state.tool_start_time = Some(Instant::now());
                state.tool_calls += 1;
                print_tool_call(&name, &input);
            }
            AgentEvent::ToolOutput { delta } => {
                let cols = crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
                let inner = cols.saturating_sub(2);
                let bar_l = "│".with(Color::DarkGrey);
                let bar_r = "│".with(Color::DarkGrey);
                let truncated = truncate_chars(&delta, inner.saturating_sub(2));
                let pad = " ".repeat(inner.saturating_sub(1 + truncated.chars().count()));
                println!("{} {}{}{}", bar_l, truncated.with(Color::Grey), pad, bar_r);
                io::stdout().flush().ok();
            }
            AgentEvent::ToolResult { name, output, elapsed_ms } => {
                last_char = '\n';
                state.pending_tool_call = None;
                print_tool_result(&name, &output, elapsed_ms);
            }
            AgentEvent::TokenUsage { input, output, total, cache_read, cache_write, context_pct } => {
                state.tokens_input = input;
                state.tokens_output = output;
                state.tokens_total = total;
                state.tokens_cache_read = cache_read;
                state.tokens_cache_write = cache_write;
                state.context_pct = context_pct;
            }
            AgentEvent::TurnEnd => {
                state.turns += 1;
            }
            AgentEvent::Error { message } => {
                print_status(&message, Color::DarkRed);
            }
            AgentEvent::AgentEnd { .. } => {
                println!();
                trace!("turn complete: {}ms total, first_token={}ms",
                    t0.elapsed().as_millis(),
                    first_token_time.unwrap_or(0));
                break;
            }
            AgentEvent::ThinkingDelta(delta) => {
                if state.show_thinking && !delta.is_empty() {
                    last_char = delta.chars().last().unwrap_or(last_char);
                    crossterm::queue!(
                        io::stdout(),
                        style::PrintStyledContent(
                            delta.with(Color::DarkGrey).attribute(crossterm::style::Attribute::Italic)
                        )
                    ).ok();
                    io::stdout().flush().ok();
                }
            }
            AgentEvent::TurnStart => {}
            _ => {}
        }
    }
}

fn handle_slash_command(
    prompt: &str,
    cmd_tx: &mpsc::Sender<TuiCommand>,
    state: &mut State,
) -> bool {
    let trimmed = prompt.trim();
    if !trimmed.starts_with('/') {
        return false;
    }

    let mut parts = trimmed.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();

    match cmd {
        "/clear" => {
            let _ = cmd_tx.try_send(TuiCommand::Clear);
            println!("  {}", "conversation history cleared".with(Color::DarkGrey));
            true
        }
        "/abort" => {
            let _ = cmd_tx.try_send(TuiCommand::Abort);
            true
        }
        "/model" => {
            if arg.is_empty() {
                println!("  {}", "usage: /model <model-id>".with(Color::DarkYellow));
            } else {
                state.model_name = arg.to_string();
                let _ = cmd_tx.try_send(TuiCommand::SetModel(arg.to_string()));
                println!("  {} {}", "model set to".with(Color::DarkGrey), arg.with(Color::White));
            }
            true
        }
        "/provider" => {
            // Format from UI: /provider <id> <api_key>
            let mut parts = arg.splitn(2, ' ');
            let provider_id  = parts.next().unwrap_or("").trim();
            let api_key      = parts.next().unwrap_or("").trim();

            if provider_id.is_empty() {
                println!("  {}", "usage: /provider <id> [api_key]".with(Color::DarkYellow));
            } else {
                // Store key in env so agent can pick it up via make_client().
                if !api_key.is_empty() {
                    let env_var = format!("{}_API_KEY", provider_id.to_uppercase().replace('-', "_"));
                    std::env::set_var(&env_var, api_key);
                }
                state.provider_name = provider_id.to_string();
                let _ = cmd_tx.try_send(TuiCommand::SetProvider(provider_id.to_string()));
                println!(
                    "  {} {}{}",
                    "provider set to".with(Color::DarkGrey),
                    provider_id.with(Color::White),
                    if api_key.is_empty() { "" } else { " (key saved)" },
                );
            }
            true
        }
        "/help" => {
            println!("  {}", "commands:".with(Color::DarkGrey));
            println!("  {}  {}", "/model <id>   ".with(Color::White), "switch model".with(Color::DarkGrey));
            println!("  {}  {}", "/provider <id>".with(Color::White), "switch provider".with(Color::DarkGrey));
            println!("  {}  {}", "/clear        ".with(Color::White), "clear conversation history".with(Color::DarkGrey));
            println!("  {}  {}", "/abort        ".with(Color::White), "abort current request".with(Color::DarkGrey));
            println!("  {}  {}", "/help         ".with(Color::White), "show this help".with(Color::DarkGrey));
            true
        }
        _ => false,
    }
}

fn print_tool_call(name: &str, input: &str) {
    let label = tool_human_label(name, input);
    let cols = terminal::size().unwrap_or((80, 24)).0 as usize;
    let tag = format!(" {} ", name);
    let inner = cols.saturating_sub(2);
    let label_len = label.chars().count();
    let tag_len = tag.chars().count();
    let gap = inner.saturating_sub(1 + label_len + tag_len);

    println!();
    println!(
        "{}{}{}{}{}",
        "┌".with(Color::DarkGrey),
        format!(" {}", label).with(Color::White).bold(),
        " ".repeat(gap).with(Color::DarkGrey),
        tag.with(Color::DarkCyan),
        "┐".with(Color::DarkGrey),
    );
}

fn print_tool_result(_name: &str, output: &str, elapsed_ms: u64) {
    let elapsed = if elapsed_ms >= 1000 {
        format!("{:.1}s", elapsed_ms as f64 / 1000.0)
    } else {
        format!("{}ms", elapsed_ms)
    };

    let cols = terminal::size().unwrap_or((80, 24)).0 as usize;
    let inner = cols.saturating_sub(2);
    let all_lines: Vec<&str> = output.lines().collect();
    let total = all_lines.len();

    let bar_l = "│".with(Color::DarkGrey);
    let bar_r = "│".with(Color::DarkGrey);

    if total > 2000 {
        let msg = format!("[Truncated: showing {} of {} lines]", 12, total);
        let pad = " ".repeat(inner.saturating_sub(1 + msg.chars().count()));
        println!("{} {}{}{}", bar_l, msg.with(Color::DarkYellow), pad, bar_r);
    }

    if total > 12 {
        let msg = format!("... ({} earlier lines)", total - 12);
        let pad = " ".repeat(inner.saturating_sub(1 + msg.chars().count()));
        println!("{} {}{}{}", bar_l, msg.with(Color::DarkGrey), pad, bar_r);
    }

    let tail = &all_lines[total.saturating_sub(12)..];
    for line in tail {
        let truncated = truncate_chars(line, inner.saturating_sub(2));
        let pad = " ".repeat(inner.saturating_sub(1 + truncated.chars().count()));
        println!("{} {}{}{}", bar_l, truncated.with(Color::Grey), pad, bar_r);
    }

    let took = format!(" Took {} ", elapsed);
    let border_fill = inner.saturating_sub(took.chars().count());
    println!(
        "{}{}{}{}",
        "└".with(Color::DarkGrey),
        took.with(Color::DarkGrey),
        "─".repeat(border_fill).with(Color::DarkGrey),
        "┘".with(Color::DarkGrey),
    );
    println!();
}

fn print_status_bar(state: &State) {
    let ctx_color = if state.context_pct > 80.0 {
        Color::DarkRed
    } else if state.context_pct > 50.0 {
        Color::DarkYellow
    } else {
        Color::DarkGreen
    };
    let bar_width = 10usize;
    let filled = ((state.context_pct / 100.0) * bar_width as f32).round() as usize;
    let filled = filled.min(bar_width);
    let bar: String = format!("[{}{}]",
        "█".repeat(filled),
        "─".repeat(bar_width - filled),
    );
    writeln!(
        io::stdout(),
        "  {} {} {} {}  {} {} {} {}  {} {} {:.0}%  {} ${:.4}  {} {}",
        "in".with(Color::DarkGrey),
        format_tokens(state.tokens_input).with(Color::White),
        "out".with(Color::DarkGrey),
        format_tokens(state.tokens_output).with(Color::White),
        "↑".with(Color::DarkGrey),
        format_tokens(state.tokens_cache_write).with(Color::DarkCyan),
        "↓".with(Color::DarkGrey),
        format_tokens(state.tokens_cache_read).with(Color::Cyan),
        "ctx".with(Color::DarkGrey),
        bar.with(ctx_color),
        state.context_pct,
        "cost".with(Color::DarkGrey),
        state.cost,
        "turn".with(Color::DarkGrey),
        state.turns,
    )
    .ok();
}

fn print_status(msg: &str, color: Color) {
    println!("\n  {}", msg.with(color));
}

fn tool_human_label(name: &str, input: &str) -> String {
    let val: serde_json::Value = serde_json::from_str(input).unwrap_or(serde_json::Value::Null);
    match name {
        "bash" | "shell" | "run_command" | "execute" | "run_bash" => {
            let cmd = val.get("command")
                .or_else(|| val.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or(input);
            let first = cmd.lines().next().unwrap_or(cmd);
            format!("$ {}", truncate_chars(first, 80))
        }
        "read_file" | "read" | "view_file" | "view" => {
            let path = val.get("path").and_then(|v| v.as_str()).unwrap_or(input);
            format!("read {}", path)
        }
        "write_file" | "write" | "create_file" => {
            let path = val.get("path").and_then(|v| v.as_str()).unwrap_or(input);
            format!("write {}", path)
        }
        "list_files" | "ls" | "list_directory" => {
            let path = val.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            format!("$ ls {}", path)
        }
        "search" | "grep" | "find" | "search_files" => {
            let pattern = val.get("pattern")
                .or_else(|| val.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or(input);
            format!("$ grep \"{}\"", truncate_chars(pattern, 60))
        }
        _ => {
            format!("{} {}", name, truncate_chars(input, 80))
        }
    }
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
