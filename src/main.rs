//! Iter Coding Agent - Terminal UI
//!
//! A Rust TUI application that provides a terminal interface for an LLM agent.
//! Communicates with a TypeScript agent process via JSONL over stdin/stdout.

mod agent;
mod rpc;
mod state;
mod ui;

use std::{io, time::Duration};
use std::sync::mpsc::{self, Receiver};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};

use state::{App, ChatMessage, MsgKind};
use ui::layout::ui as ui_layout;

// ============================================================================
// Constants
// ============================================================================

/// Polling interval for keyboard events (in milliseconds).
const POLL_INTERVAL_MS: u64 = 100;

/// Initial request ID for startup state request.
const STARTUP_REQUEST_ID: &str = "startup-state";

/// Request type for getting initial agent state.
const GET_STATE_TYPE: &str = "get_state";

// ============================================================================
// Main Entry Point
// ============================================================================

fn main() -> io::Result<()> {
    // Initialize terminal for alternate screen mode.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend  = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;

    // Initialize application state and spawn agent process.
    let mut app = App::new();
    let (tx, rx) = mpsc::channel::<rpc::UiEvent>();
    let mut agent_stdin = agent::spawn_agent(tx);

    // Request initial state from agent.
    agent::send_cmd(&mut agent_stdin, serde_json::json!({
        "id": STARTUP_REQUEST_ID,
        "type": GET_STATE_TYPE,
    }));

    // Run the main event loop.
    let result = run(&mut term, &mut app, rx, &mut agent_stdin);

    // Restore terminal to normal mode before exit.
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    term.show_cursor()?;

    if let Err(e) = result {
        eprintln!("{e:?}");
    }
    Ok(())
}

// ============================================================================
// Event Loop
// ============================================================================

/// Main application event loop.
///
/// Handles both terminal input events and agent messages from the channel.
fn run(
    term:        &mut Terminal<CrosstermBackend<io::Stdout>>,
    app:         &mut App,
    rx:          Receiver<rpc::UiEvent>,
    agent_stdin: &mut Option<std::process::ChildStdin>,
) -> io::Result<()> {
    loop {
        // Render UI with current state.
        term.draw(|f| ui_layout(f, app))?;

        // Process any pending agent messages.
        while let Ok(event) = rx.try_recv() {
            match event {
                rpc::UiEvent::Agent(msg) => agent::handle_agent_msg(app, agent_stdin, msg),
                rpc::UiEvent::SpawnError(err) => app.push_system(format!("spawn error: {err}")),
            }
        }

        // Poll for keyboard input with timeout.
        if event::poll(Duration::from_millis(POLL_INTERVAL_MS))? {
            if let Event::Key(key) = event::read()? {
                handle_key_input(key, app, agent_stdin);
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// Handles keyboard input events.
fn handle_key_input(
    key:         crossterm::event::KeyEvent,
    app:         &mut App,
    agent_stdin: &mut Option<std::process::ChildStdin>,
) {
use KeyCode::*;

    // ── Model picker intercepts all keys when open ──────────────────────
    if app.model_picker_open {
        handle_picker_key(key, app, agent_stdin);
        return;
    }

    match (key.modifiers, key.code) {
        // Quit application.
        (KeyModifiers::CONTROL, Char('c')) => app.should_quit = true,

        // Open model picker.
        (KeyModifiers::CONTROL, Char('p')) => {
            app.model_picker_open     = true;
            app.model_picker_query    = String::new();
            app.model_picker_selected = 0;
        }

        // Clear input buffer.
        (KeyModifiers::CONTROL, Char('u')) => app.input.clear(),

        // Clear chat history.
        (KeyModifiers::CONTROL, Char('l')) => {
            app.messages.clear();
            app.scroll = 0;
        }

        // Stop/cancel current streaming operation.
        (_, Esc) => {
            if app.streaming {
                app.end_streaming();
                agent::send_abort(agent_stdin);
            }
        }

        // Insert newline in input (Alt+Enter).
        (KeyModifiers::ALT, Enter) => {
            app.input.push('\n');
        }

        // Send user message to agent.
        (_, Enter) => {
            let text = app.input.trim().to_string();
            if !text.is_empty() && !app.streaming {
                let id = format!("prompt-{}", app.turns + 1);
                agent::send_cmd(agent_stdin, serde_json::json!({
                    "id": id,
                    "type": "prompt",
                    "content": text,
                }));
                app.messages.push(ChatMessage {
                    kind: MsgKind::User,
                    content: text,
                });
                app.input.clear();
                app.scroll_to_bottom();
            }
        }

        // Scroll navigation.
        (_, PageUp)   => app.scroll_up(),
        (_, PageDown) => app.scroll_down(),
        (_, Up)       => app.scroll_up(),
        (_, Down)     => app.scroll_down(),

        // Text input.
        (_, Backspace) => { app.input.pop(); }
        (_, Char(c))   => { app.input.push(c); }

        // Ignore other keys (e.g., Home, End, etc.).
        _ => {}
    }
}

/// Handles keys while the model picker is open.
fn handle_picker_key(
    key:         crossterm::event::KeyEvent,
    app:         &mut App,
    agent_stdin: &mut Option<std::process::ChildStdin>,
) {
    use KeyCode::*;
    use ui::model_picker::filtered_models;

    match (key.modifiers, key.code) {
        // Close picker.
        (KeyModifiers::CONTROL, Char('p')) | (_, Esc) => {
            app.model_picker_open = false;
        }

        // Navigate down.
        (_, Down) | (KeyModifiers::CONTROL, Char('n')) => {
            let count = filtered_models(&app.model_picker_query).len();
            if count > 0 {
                app.model_picker_selected = (app.model_picker_selected + 1).min(count - 1);
            }
        }

        // Navigate up.
        (_, Up) => {
            app.model_picker_selected = app.model_picker_selected.saturating_sub(1);
        }

        // Also handle plain Ctrl+K for up (common in pickers).
        (KeyModifiers::CONTROL, Char('k')) => {
            app.model_picker_selected = app.model_picker_selected.saturating_sub(1);
        }
        (KeyModifiers::CONTROL, Char('j')) => {
            let count = filtered_models(&app.model_picker_query).len();
            if count > 0 {
                app.model_picker_selected = (app.model_picker_selected + 1).min(count - 1);
            }
        }

        // Select model.
        (_, Enter) => {
            let matches = filtered_models(&app.model_picker_query);
            if let Some(&model_idx) = matches.get(app.model_picker_selected) {
                let (model_id, _) = ui::model_picker::MODELS[model_idx];
                agent::send_cmd(agent_stdin, serde_json::json!({
                    "id": "set-model",
                    "type": "set_model",
                    "model": model_id,
                }));
                app.model_name = model_id.to_string();
            }
            app.model_picker_open = false;
        }

        // Backspace in search.
        (_, Backspace) => {
            app.model_picker_query.pop();
            app.model_picker_selected = 0;
        }

        // Type to filter.
        (KeyModifiers::NONE, Char(c)) => {
            app.model_picker_query.push(c);
            app.model_picker_selected = 0;
        }

        _ => {}
    }
}