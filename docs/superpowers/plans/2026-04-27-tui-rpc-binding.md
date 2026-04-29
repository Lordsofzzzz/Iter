# TUI RPC Binding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Spawn the Node agent process from the Rust TUI and stream JSONL `text_delta` events into the chat view.

**Architecture:** The TUI spawns `ts-node agent/src/index.ts` as a child process, sends JSONL messages on stdin, and reads stdout JSONL events on a background thread, forwarding them to the UI loop via a channel.

**Tech Stack:** Rust (ratatui, crossterm), std::process, std::sync::mpsc

---

### Task 1: Add RPC Event Types and Process Wrapper

**Files:**
- Create: `src/rpc.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Define RPC events and JSON parsing**

Create `src/rpc.rs`:
```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum RpcEvent {
    #[serde(rename = "agent_start")]
    AgentStart,
    #[serde(rename = "turn_start")]
    TurnStart,
    #[serde(rename = "text_delta")]
    TextDelta { delta: String },
    #[serde(rename = "turn_end")]
    TurnEnd,
    #[serde(rename = "agent_end")]
    AgentEnd,
    #[serde(rename = "error")]
    Error { message: String },
}

#[derive(Debug)]
pub enum UiEvent {
    Rpc(RpcEvent),
    SpawnError(String),
}
```

- [ ] **Step 2: Expose rpc module in main**

Update `src/main.rs` to add:
```rust
mod rpc;
```

- [ ] **Step 3: Commit**

```bash
```

---

### Task 2: Spawn Agent Process + Reader Thread

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Spawn child process and capture pipes**

Add imports:
```rust
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender, Receiver};
use std::thread;
use std::io::{BufRead, BufReader, Write};
```

Add a helper in `main.rs`:
```rust
fn spawn_agent(tx: Sender<rpc::UiEvent>) -> Option<std::process::ChildStdin> {
    let mut child = match Command::new("ts-node")
        .arg("agent/src/index.ts")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            let _ = tx.send(rpc::UiEvent::SpawnError(e.to_string()));
            return None;
        }
    };

    let stdout = child.stdout.take().expect("child stdout");
    let stdin = child.stdin.take().expect("child stdin");

    let tx_clone = tx.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().flatten() {
            match serde_json::from_str::<rpc::RpcEvent>(&line) {
                Ok(ev) => { let _ = tx_clone.send(rpc::UiEvent::Rpc(ev)); }
                Err(_) => { /* ignore parse errors */ }
            }
        }
    });

    Some(stdin)
}
```

- [ ] **Step 2: Commit**

```bash
```

---

### Task 3: Wire UI State Updates from RPC Events

**Files:**
- Modify: `src/state.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add helper methods to App**

In `src/state.rs`, add:
```rust
impl App {
    pub fn start_streaming(&mut self) {
        self.streaming = true;
    }

    pub fn end_streaming(&mut self) {
        self.streaming = false;
    }

    pub fn push_assistant_delta(&mut self, delta: String) {
        if let Some(last) = self.messages.last_mut() {
            if matches!(last.kind, MsgKind::Assistant) {
                last.content.push_str(&delta);
                return;
            }
        }
        self.messages.push(ChatMessage { kind: MsgKind::Assistant, content: delta });
    }

    pub fn push_system(&mut self, text: String) {
        self.messages.push(ChatMessage { kind: MsgKind::System, content: text });
    }
}
```

- [ ] **Step 2: Poll channel in main loop**

In `main.rs`, create channel and store stdin handle:
```rust
let (tx, rx) = mpsc::channel::<rpc::UiEvent>();
let mut agent_stdin = spawn_agent(tx);
```

In the run loop, before rendering, drain events:
```rust
while let Ok(event) = rx.try_recv() {
    match event {
        rpc::UiEvent::Rpc(rpc::RpcEvent::AgentStart) => app.start_streaming(),
        rpc::UiEvent::Rpc(rpc::RpcEvent::TurnStart) => app.start_streaming(),
        rpc::UiEvent::Rpc(rpc::RpcEvent::TextDelta { delta }) => app.push_assistant_delta(delta),
        rpc::UiEvent::Rpc(rpc::RpcEvent::TurnEnd) => app.end_streaming(),
        rpc::UiEvent::Rpc(rpc::RpcEvent::AgentEnd) => app.end_streaming(),
        rpc::UiEvent::Rpc(rpc::RpcEvent::Error { message }) => {
            app.push_system(message);
            app.end_streaming();
        }
        rpc::UiEvent::SpawnError(err) => app.push_system(err),
    }
}
```

- [ ] **Step 3: Commit**

```bash
```

---

### Task 4: Send JSONL Messages to Agent

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Write JSONL on Enter**

In Enter key handling, replace `app.push_input()` with:
```rust
if let Some(stdin) = agent_stdin.as_mut() {
    let text = app.input.trim().to_string();
    if !text.is_empty() && !app.streaming {
        let payload = serde_json::json!({ "type": "message", "content": text });
        let _ = writeln!(stdin, "{}", payload.to_string());
        app.messages.push(state::ChatMessage { kind: state::MsgKind::User, content: text });
        app.input.clear();
        app.scroll = app.messages.len().saturating_sub(1);
    }
}
```

- [ ] **Step 2: Commit**

```bash
```

---

### Task 5: Manual Test

**Files:**
- None

- [ ] **Step 1: Run the TUI**

```bash
cargo run
```

- [ ] **Step 2: Send a message**
Type a prompt and press Enter. Verify deltas stream and input is disabled while streaming.

Expected:
- Assistant response appears incrementally
- Streaming cursor disappears when agent_end arrives
