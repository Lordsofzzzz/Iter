mod agent;
mod rpc;
mod state;
mod ui;

use state::{App, ChatMessage, MsgKind};
use ui::layout::ui as ui_layout;

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

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend  = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;

    let mut app = App::new();
    let (tx, rx) = mpsc::channel::<rpc::UiEvent>();
    let mut agent_stdin = agent::spawn_agent(tx);

    agent::send_cmd(&mut agent_stdin, serde_json::json!({
        "id": "startup-state", "type": "get_state"
    }));

    let res = run(&mut term, &mut app, rx, &mut agent_stdin);

    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    term.show_cursor()?;
    if let Err(e) = res { eprintln!("{e:?}"); }
    Ok(())
}

fn run(
    term:        &mut Terminal<CrosstermBackend<io::Stdout>>,
    app:         &mut App,
    rx:          Receiver<rpc::UiEvent>,
    agent_stdin: &mut Option<std::process::ChildStdin>,
) -> io::Result<()> {
    loop {
        term.draw(|f| ui_layout(f, app))?;

        while let Ok(event) = rx.try_recv() {
            match event {
                rpc::UiEvent::Agent(msg) => agent::handle_agent_msg(app, agent_stdin, msg),
                rpc::UiEvent::SpawnError(err) => app.push_system(format!("spawn error: {err}")),
            }
        }

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match (key.modifiers, key.code) {
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) => app.should_quit = true,
                    (KeyModifiers::CONTROL, KeyCode::Char('u')) => app.input.clear(),
                    (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
                        app.messages.clear();
                        app.scroll = 0;
                    }
                    (_, KeyCode::Enter) => {
                        let text = app.input.trim().to_string();
                        if !text.is_empty() && !app.streaming {
                            let id = format!("prompt-{}", app.turns + 1);
                            agent::send_cmd(agent_stdin, serde_json::json!({
                                "id": id, "type": "prompt", "content": text,
                            }));
                            app.messages.push(ChatMessage {
                                kind: MsgKind::User, content: text,
                            });
                            app.input.clear();
                            app.scroll_to_bottom();
                        }
                    }
                    (_, KeyCode::PageUp)    => app.scroll_up(),
                    (_, KeyCode::PageDown)  => app.scroll_down(),
                    (_, KeyCode::Up)        => app.scroll_up(),
                    (_, KeyCode::Down)      => app.scroll_down(),
                    (_, KeyCode::Backspace) => { app.input.pop(); }
                    (_, KeyCode::Char(c))   => { app.input.push(c); }
                    _ => {}
                }
            }
        }

        if app.should_quit { break; }
    }
    Ok(())
}