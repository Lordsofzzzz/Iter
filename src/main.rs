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
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use tui_textarea::Input;

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

        // Poll for keyboard/mouse input with timeout.
        if event::poll(Duration::from_millis(POLL_INTERVAL_MS))? {
            match event::read()? {
                Event::Key(key) => handle_key_input(key, app, agent_stdin),
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp   => app.scroll_up(),
                    MouseEventKind::ScrollDown => app.scroll_down(),
                    _ => {}
                },
                _ => {}
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
        (KeyModifiers::CONTROL, Char('c')) => {
            app.should_quit = true;
            return;
        }

        // Open model picker.
        (KeyModifiers::CONTROL, Char('p')) => {
            app.model_picker_open     = true;
            app.model_picker_query    = String::new();
            app.model_picker_selected = 0;
            return;
        }

        // Clear chat history.
        (KeyModifiers::CONTROL, Char('l')) => {
            app.messages.clear();
            app.scroll = 0;
            return;
        }

        // Stop/cancel current streaming operation.
        (_, Esc) => {
            if app.streaming {
                app.end_streaming();
                agent::send_abort(agent_stdin);
            }
            return;
        }

        // Scroll navigation.
        (_, PageUp) | (KeyModifiers::ALT, Up)   => { app.scroll_up();   return; }
        (_, PageDown) | (KeyModifiers::ALT, Down) => { app.scroll_down();  return; }

        // Send message on plain Enter.
        (KeyModifiers::NONE, Enter) | (KeyModifiers::NONE, Char('\n')) => {
            let text = app.textarea.lines().join("\n");
            let text = text.trim().to_string();
            if !text.is_empty() && !app.streaming {
                let id = format!("prompt-{}", app.turns + 1);
                agent::send_cmd(agent_stdin, serde_json::json!({
                    "id": id,
                    "type": "prompt",
                    "content": text,
                }));
                app.messages.push(ChatMessage {
                    kind:     MsgKind::User,
                    content:  text,
                    thinking: String::new(),
                    done:     true,
                });
                // Reset textarea to empty.
                app.textarea = {
                    let mut ta = tui_textarea::TextArea::default();
                    ta.set_cursor_line_style(ratatui::style::Style::default());
                    ta.set_cursor_style(ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED));
                    ta
                };
                app.scroll_to_bottom();
            }
            return;
        }

        _ => {}
    }

    // All other keys go to textarea (arrows, Home/End, Ctrl+W, etc.)
    app.textarea.input(Input::from(key));
}

/// Handles keys while the model picker is open.
fn handle_picker_key(
    key:         crossterm::event::KeyEvent,
    app:         &mut App,
    agent_stdin: &mut Option<std::process::ChildStdin>,
) {
    use KeyCode::*;
    use ui::model_picker::{filtered_models_dynamic, MODELS};

    // Use dynamic model list if available, else static fallback.
    let static_models: Vec<(String, String)>;
    let model_slice: &[(String, String)] = if !app.models.is_empty() {
        &app.models
    } else {
        static_models = MODELS.iter().map(|(id, name)| (id.to_string(), name.to_string())).collect();
        &static_models
    };

    match (key.modifiers, key.code) {
        // Close picker.
        (KeyModifiers::CONTROL, Char('p')) | (_, Esc) => {
            app.model_picker_open = false;
        }

        // Navigate down.
        (_, Down) | (KeyModifiers::CONTROL, Char('n')) => {
            let count = filtered_models_dynamic(&app.model_picker_query, model_slice).len();
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
            let count = filtered_models_dynamic(&app.model_picker_query, model_slice).len();
            if count > 0 {
                app.model_picker_selected = (app.model_picker_selected + 1).min(count - 1);
            }
        }

        // Select model.
        (_, Enter) => {
            let matches = filtered_models_dynamic(&app.model_picker_query, model_slice);
            if let Some(&model_idx) = matches.get(app.model_picker_selected) {
                let (model_id, _) = &model_slice[model_idx];
                agent::send_cmd(agent_stdin, serde_json::json!({
                    "id": "set-model",
                    "type": "set_model",
                    "model": model_id,
                }));
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