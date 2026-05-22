# Rust Port: Embed LLM Agent with Rig

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) for syntax tracking.

**Goal:** Replace the Node.js agent process with an in-process Rust agent using the Rig crate, eliminating IPC overhead.

**Architecture:** A tokio task runs the Rig agent loop (OpenRouter provider) and sends events through an mpsc channel to the TUI event loop. Tool execution happens in-process via Rig's `Tool` trait. The old `agent.rs` and `rpc.rs` modules are removed entirely.

**Tech Stack:** Rust, rig-core 0.37, tokio 1, reqwest 0.12, serde, crossterm

---

### Task 1: Add Rig + tokio dependencies

**Files:**
- Modify: `Cargo.toml`

**Details:**

- [ ] **Step 1: Update Cargo.toml**

Replace current dependencies with new ones. Keep existing deps (clap, crossterm, unicode-width, termimad, syntect) and add rig-core + tokio.

```toml
[package]
name = "iter"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "iter"
path = "src/main.rs"

[dependencies]
clap          = { version = "4.6", features = ["derive", "env"] }
serde         = { version = "1", features = ["derive"] }
serde_json    = "1"
crossterm     = "0.29"
unicode-width = "0.2"
termimad      = "0.34"
syntect       = { version = "5.3", default-features = false, features = ["default-syntaxes", "default-themes", "regex-fancy"] }

# New — async runtime + LLM agent
rig-core      = { version = "0.37", features = ["derive"] }
tokio         = { version = "1", features = ["rt", "macros", "sync", "process"] }
reqwest       = { version = "0.12", features = ["json", "stream"] }
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | head -20`
Expected: clean compilation (may warn about unused imports, that's fine)

---

### Task 2: Create tool implementations

**Files:**
- Create: `src/tools/mod.rs`

**Details:**

This module implements Rig's `Tool` trait for the 6 tools Iter needs: `read_file`, `write_file`, `edit`, `run_command`, `list_files`, `search_files`.

- [ ] **Step 1: Write tools/mod.rs**

```rust
//! Tool implementations: read_file, write_file, edit, run_command, list_files, search_files

use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;

// ── read_file ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ReadFileArgs {
    pub path: String,
}

#[derive(Debug, thiserror::Error)]
#[error("read_file error: {0}")]
pub struct ReadFileError(pub String);

pub struct ReadFile;

impl Tool for ReadFile {
    const NAME: &'static str = "read_file";

    type Error = ReadFileError;
    type Args = ReadFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: Self::NAME.into(),
            description: "Read the contents of a file at the given path".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        std::fs::read_to_string(&args.path).map_err(|e| ReadFileError(e.to_string()))
    }
}

// ── write_file ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct WriteFileArgs {
    pub path: String,
    pub content: String,
}

#[derive(Debug, thiserror::Error)]
#[error("write_file error: {0}")]
pub struct WriteFileError(pub String);

pub struct WriteFile;

impl Tool for WriteFile {
    const NAME: &'static str = "write_file";

    type Error = WriteFileError;
    type Args = WriteFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: Self::NAME.into(),
            description: "Write content to a file at the given path. Creates parent directories if needed.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if let Some(parent) = std::path::Path::new(&args.path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| WriteFileError(e.to_string()))?;
        }
        std::fs::write(&args.path, &args.content).map_err(|e| WriteFileError(e.to_string()))?;
        Ok(format!("wrote {} bytes to {}", args.content.len(), args.path))
    }
}

// ── edit (find-and-replace) ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct EditArgs {
    pub path: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, thiserror::Error)]
#[error("edit error: {0}")]
pub struct EditError(pub String);

pub struct Edit;

impl Tool for Edit {
    const NAME: &'static str = "edit";

    type Error = EditError;
    type Args = EditArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: Self::NAME.into(),
            description: "Replace first occurrence of `old` text with `new` text in a file".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" },
                    "old": { "type": "string", "description": "Text to find (first occurrence)" },
                    "new": { "type": "string", "description": "Replacement text" }
                },
                "required": ["path", "old", "new"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let content = std::fs::read_to_string(&args.path)
            .map_err(|e| EditError(format!("read: {e}")))?;
        if !content.contains(&args.old) {
            return Err(EditError("old text not found".into()));
        }
        let result = content.replacen(&args.old, &args.new, 1);
        std::fs::write(&args.path, &result)
            .map_err(|e| EditError(format!("write: {e}")))?;
        Ok(format!("edited {}", args.path))
    }
}

// ── run_command ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RunCommandArgs {
    pub command: String,
}

#[derive(Debug, thiserror::Error)]
#[error("run_command error: {0}")]
pub struct RunCommandError(pub String);

pub struct RunCommand;

impl Tool for RunCommand {
    const NAME: &'static str = "run_command";

    type Error = RunCommandError;
    type Args = RunCommandArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: Self::NAME.into(),
            description: "Run a shell command. Use with caution.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to execute"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&args.command)
            .output()
            .await
            .map_err(|e| RunCommandError(e.to_string()))?;

        let mut result = String::new();
        if !output.stdout.is_empty() {
            result.push_str(&String::from_utf8_lossy(&output.stdout));
        }
        if !output.stderr.is_empty() {
            result.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        if !output.status.success() {
            result.push_str(&format!("\nexit code: {}", output.status));
        }
        Ok(result)
    }
}

// ── list_files ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListFilesArgs {
    pub path: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("list_files error: {0}")]
pub struct ListFilesError(pub String);

pub struct ListFiles;

impl Tool for ListFiles {
    const NAME: &'static str = "list_files";

    type Error = ListFilesError;
    type Args = ListFilesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: Self::NAME.into(),
            description: "List files and directories at the given path (defaults to current directory)".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path (defaults to current directory)"
                    }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = args.path.unwrap_or_else(|| ".".to_string());
        let mut entries: Vec<_> = std::fs::read_dir(&path)
            .map_err(|e| ListFilesError(e.to_string()))?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());

        let mut out = String::new();
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            let kind = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                "dir "
            } else {
                "file"
            };
            out.push_str(&format!(" {kind}  {name}\n"));
        }
        Ok(out)
    }
}

// ── search_files ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SearchFilesArgs {
    pub pattern: String,
    pub path: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("search_files error: {0}")]
pub struct SearchFilesError(pub String);

pub struct SearchFiles;

impl Tool for SearchFiles {
    const NAME: &'static str = "search_files";

    type Error = SearchFilesError;
    type Args = SearchFilesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: Self::NAME.into(),
            description: "Recursively search for files matching a pattern in a directory".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Filename pattern to search for (supports globs like *.rs)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (defaults to current directory)"
                    }
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = args.path.unwrap_or_else(|| ".".to_string());
        let pattern = args.pattern;
        let glob_pattern = format!("{}/**/{}", path.trim_end_matches('/'), pattern);

        let mut results = String::new();
        let Ok(mut entries) = glob::glob(&glob_pattern) else {
            return Ok("no matches found".into());
        };
        for entry in entries.flatten() {
            results.push_str(&format!("{}\n", entry.display()));
        }
        if results.is_empty() {
            return Ok("no matches found".into());
        }
        Ok(results)
    }
}
```

Note: This uses `thiserror` and `glob` crates. We need to add them to Cargo.toml.

- [ ] **Step 2: Add thiserror and glob to Cargo.toml**

```toml
thiserror     = "1"
glob          = "0.3"
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1`
Expected: clean compilation

---

### Task 3: Create agent event types

**Files:**
- Create: `src/agent_event.rs`

**Details:**

Define the internal event types that the agent task sends to the TUI via the channel. This replaces the old `rpc.rs` types.

- [ ] **Step 1: Write agent_event.rs**

```rust
//! Internal event types for agent → TUI communication.
//! Replaces the old RPC wire protocol with direct in-process types.

use serde::Deserialize;
use std::time::Instant;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCall {
        name: String,
        input: String,
    },
    ToolResult {
        name: String,
        output: String,
        elapsed_ms: u64,
    },
    Error {
        message: String,
    },
    AgentEnd {
        success: bool,
        error: Option<String>,
    },
    TurnStart,
    TurnEnd,
    AgentStart,
    ProviderChanged {
        provider_id: String,
        provider_name: String,
    },
    ModelList {
        models: Vec<(String, String)>,
    },
}

/// Messages the TUI sends back to the agent task (new prompts, abort, config changes).
#[derive(Debug, Clone)]
pub enum TuiCommand {
    Prompt(String),
    Abort,
    SetModel(String),
    SetProvider(String),
    Clear,
}
```

---

### Task 4: Create context manager

**Files:**
- Create: `src/context.rs`

**Details:**

Manages the conversation history, auto-compacting at 80% context window threshold. Mirrors the JS `context.ts` behavior.

- [ ] **Step 1: Write context.rs**

```rust
//! Conversation history with auto-compaction.

use rig::message::Message;

pub struct Context {
    pub messages: Vec<Message>,
    pub context_window: u32,
}

impl Context {
    pub fn new(context_window: u32) -> Self {
        Self {
            messages: Vec::new(),
            context_window,
        }
    }

    pub fn add_message(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Compact history when usage exceeds threshold.
    /// Removes oldest tool results and user messages (but keeps system prompt and recent turns).
    pub fn compact_if_needed(&mut self, usage_pct: f32) {
        if usage_pct < 80.0 || self.messages.len() < 10 {
            return;
        }

        // Keep first message (system prompt) and last 4 messages (recent turn).
        let keep_front = 1;
        let keep_back = 4;
        if self.messages.len() <= keep_front + keep_back {
            return;
        }

        // Remove middle section: oldest tool results and user messages
        let mut compacted: Vec<Message> = self.messages.drain(..keep_front).collect();
        let back = self.messages.split_off(self.messages.len().saturating_sub(keep_back));
        // Keep at least one summary message from the removed section
        compacted.push(Message::user(format!(
            "[{} earlier messages compacted]",
            self.messages.len()
        )));
        compacted.extend(back);
        self.messages = compacted;
    }
}
```

---

### Task 5: Create the agent loop

**Files:**
- Create: `src/agent.rs` (replaces old one)

**Details:**

The tokio task that runs the Rig agent, handles tool execution, and sends events to the TUI channel.

- [ ] **Step 1: Write agent.rs**

```rust
//! In-process agent loop using Rig.
//! Spawned as a tokio task, communicates with TUI via mpsc channels.

use std::sync::Arc;
use tokio::sync::mpsc;

use rig::providers::openrouter;
use rig::streaming::StreamingChat;

use crate::agent_event::{AgentEvent, TuiCommand};
use crate::context::Context;
use crate::tools;

pub struct AgentConfig {
    pub model: String,
    pub api_key: String,
    pub system_prompt: String,
}

pub async fn run_agent_loop(
    config: AgentConfig,
    event_tx: mpsc::Sender<AgentEvent>,
    mut cmd_rx: mpsc::Receiver<TuiCommand>,
) {
    let client = openrouter::Client::new(&config.api_key).expect("failed to create OpenRouter client");

    let mut context = Context::new(128_000);
    let mut current_model = config.model.clone();

    let _ = event_tx.send(AgentEvent::AgentStart).await;

    // Add system prompt
    context.add_message(Message::system(&config.system_prompt));

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            TuiCommand::Prompt(prompt) => {
                let _ = event_tx.send(AgentEvent::TurnStart).await;
                process_prompt(&client, &current_model, &mut context, &event_tx).await;
                let _ = event_tx.send(AgentEvent::TurnEnd).await;
                let _ = event_tx.send(AgentEvent::AgentEnd { success: true, error: None }).await;
            }
            TuiCommand::Abort => {
                // TODO: abort via signal
            }
            TuiCommand::SetModel(model) => {
                current_model = model;
            }
            TuiCommand::SetProvider(provider) => {
                let _ = event_tx.send(AgentEvent::ProviderChanged {
                    provider_id: provider.clone(),
                    provider_name: provider,
                }).await;
            }
            TuiCommand::Clear => {
                context.clear();
                context.add_message(Message::system(&config.system_prompt));
            }
        }
    }
}

async fn process_prompt(
    client: &openrouter::Client,
    model: &str,
    context: &mut Context,
    event_tx: &mpsc::Sender<AgentEvent>,
) {
    let mut agent = client
        .agent(model)
        .preamble(&context.messages.first().map(|m| m.content()).unwrap_or_default())
        .max_tokens(8192)
        .temperature(0.7)
        .tool(tools::ReadFile)
        .tool(tools::WriteFile)
        .tool(tools::Edit)
        .tool(tools::RunCommand)
        .tool(tools::ListFiles)
        .tool(tools::SearchFiles)
        .build();

    let history: Vec<rig::message::Message> = context.messages.clone();

    let mut stream = agent.stream_chat(&prompt, &history).await;

    use futures::StreamExt;

    while let Some(item) = stream.next().await {
        match item {
            Ok(rig::agent::MultiTurnStreamItem::Text(text)) => {
                let _ = event_tx.send(AgentEvent::TextDelta(text)).await;
            }
            Ok(rig::agent::MultiTurnStreamItem::ToolCall(tc)) => {
                let _ = event_tx.send(AgentEvent::ToolCall {
                    name: tc.name().to_string(),
                    input: tc.args().to_string(),
                }).await;
            }
            Ok(rig::agent::MultiTurnStreamItem::ToolResult(tr)) => {
                let _ = event_tx.send(AgentEvent::ToolResult {
                    name: tr.name().to_string(),
                    output: tr.output().to_string(),
                    elapsed_ms: tr.duration().as_millis() as u64,
                }).await;
            }
            Ok(rig::agent::MultiTurnStreamItem::FinalResponse(fin)) => {
                // Update context with new messages from the agent
                for msg in fin.history() {
                    context.add_message(msg.clone());
                }
            }
            Err(e) => {
                let _ = event_tx.send(AgentEvent::Error {
                    message: e.to_string(),
                }).await;
                break;
            }
            _ => {}
        }
    }
}
```

Note: This uses `futures` crate for `StreamExt`. Need to add it to Cargo.toml.

- [ ] **Step 2: Add futures to Cargo.toml**

```toml
futures       = "0.3"
```

---

### Task 6: Refactor main.rs

**Files:**
- Modify: `src/main.rs`
- Modify: `src/state/mod.rs`

**Details:**

Remove old RPC/agent process code. Replace with tokio runtime + channel-based agent task. Keep the TUI rendering code (input.rs, print_tool_call, etc.) largely intact.

- [ ] **Step 1: Write the new main.rs**

Replace the entire file. The key changes:
- Remove `mod rpc;` and old `mod agent;` 
- Add `mod agent_event;` `mod context;` `mod tools;`
- Use tokio runtime
- Spawn agent on tokio task with channels
- Remove `stream_response()`, `handle_push_event()`, `send_prompt()`, `drain_startup()`
- TUI still uses `input.rs` for reading input, but sends prompts via `TuiCommand` channel

New `main.rs` structure (full file):

```rust
//! Iter — inline AI coding agent.
//! Single binary with embedded Rig agent — no Node.js process.

mod agent;
mod agent_event;
mod cli;
mod context;
mod highlight;
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
use input::{InputBox, InputResult};
use state::State;

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    if let Some(workdir) = &cli.global.workdir {
        std::env::set_current_dir(workdir)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    }

    let model = cli.global.model.unwrap_or_else(|| "google/gemini-2.0-flash-001".into());
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .unwrap_or_else(|_| {
            eprint!("OPENROUTER_API_KEY not set. Enter key: ");
            let mut key = String::new();
            io::stdin().read_line(&mut key).ok();
            key.trim().to_string()
        });

    let mut state = State::new();
    state.model_name = model.clone();
    state.provider_name = "openrouter".into();

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(256);
    let (cmd_tx, cmd_rx) = mpsc::channel::<TuiCommand>(16);

    let agent_config = agent::AgentConfig {
        model: model.clone(),
        api_key,
        system_prompt: format!(
            "You are a helpful coding assistant. Current directory: {}. OS: {}.",
            std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
            std::env::consts::OS,
        ),
    };

    // Spawn agent on tokio
    rt.spawn(async move {
        agent::run_agent_loop(agent_config, event_tx, cmd_rx).await;
    });

    // TUI event loop (runs on main thread)
    interactive_loop(&mut state, &rt, &mut event_rx, cmd_tx)
}

fn interactive_loop(
    state: &mut State,
    rt: &tokio::runtime::Runtime,
    event_rx: &mut mpsc::Receiver<AgentEvent>,
    cmd_tx: mpsc::Sender<TuiCommand>,
) -> io::Result<()> {
    use crossterm::execute;
    println!(
        "{}  {}  {}",
        "iter".with(Color::DarkGreen).bold(),
        "0.2.0".with(Color::DarkGrey),
        state.model_name.as_str().with(Color::DarkGrey),
    );
    println!("{}", "─────────────────────────────────────────".with(Color::DarkGrey));

    let mut input = InputBox::new();
    let mut turn_id = 1usize;

    // Buffer for response text (for markdown rendering at the end)
    let mut response_buf = String::new();
    let mut is_streaming = false;

    loop {
        if !is_streaming {
            let hint = "describe your task...  (ctrl-k abort, ctrl-c quit)";
            match input.read(hint, state)? {
                InputResult::Quit => break,
                InputResult::Abort => {
                    let _ = cmd_tx.try_send(TuiCommand::Abort);
                    is_streaming = false;
                }
                InputResult::Submit(prompt) => {
                    let _ = io::stdout().flush();
                    response_buf.clear();
                    is_streaming = true;
                    let cmd_tx = cmd_tx.clone();
                    rt.block_on(async {
                        let _ = cmd_tx.send(TuiCommand::Prompt(prompt)).await;
                        // Drain events
                        let mut buf = String::new();
                        while let Some(event) = event_rx.recv().await {
                            match event {
                                AgentEvent::TextDelta(delta) => {
                                    buf.push_str(&delta);
                                    print!("{delta}");
                                    io::stdout().flush().ok();
                                }
                                AgentEvent::ToolCall { name, input } => {
                                    state.pending_tool_call = Some((name.clone(), input.clone()));
                                    state.tool_start_time = Some(Instant::now());
                                    state.tool_calls += 1;
                                    print_tool_call(&name, &input);
                                }
                                AgentEvent::ToolResult { name, output, elapsed_ms } => {
                                    state.pending_tool_call = None;
                                    print_tool_result(&name, &output, elapsed_ms);
                                }
                                AgentEvent::Error { message } => {
                                    eprintln!("\nerror: {message}");
                                }
                                AgentEvent::AgentEnd { .. } => {
                                    if !buf.is_empty() {
                                        // Final markdown render
                                        print_response(&buf);
                                    }
                                    println!();
                                    break;
                                }
                                AgentEvent::ThinkingDelta(delta) => {
                                    if state.show_thinking {
                                        print!("{}", delta.with(Color::DarkGrey).italic());
                                        io::stdout().flush().ok();
                                    }
                                }
                                _ => {}
                            }
                        }
                    });
                    is_streaming = false;
                    turn_id += 1;
                }
            }
        }
    }

    Ok(())
}

// ── Rendering helpers (ported from old main.rs unchanged) ─────────────────────

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

fn print_response(text: &str) {
    let mut md_buf = String::new();
    let mut lines = text.split('\n').peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if !md_buf.is_empty() {
                termimad::print_text(md_buf.trim_end_matches('\n'));
                md_buf.clear();
            }
            let lang = trimmed.trim_start_matches('`').trim();
            println!("\x1b[2m{line}\x1b[0m");
            let mut body = String::new();
            let mut closed = false;
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    let highlighted = highlight::highlight_code(lang, &body);
                    print!("{highlighted}");
                    println!("\x1b[2m{inner}\x1b[0m");
                    closed = true;
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }
            if !closed {
                print!("{body}");
            }
        } else {
            md_buf.push_str(line);
            md_buf.push('\n');
        }
    }
    if !md_buf.is_empty() {
        termimad::print_text(md_buf.trim_end_matches('\n'));
    }
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
```

- [ ] **Step 2: Update state/mod.rs** (simplify — remove RPC-specific fields that are no longer relevant)

State stays mostly the same but we can remove `models` (no longer fetched from agent).

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1`
Expected: clean compilation

---

### Task 7: Remove old files

**Files:**
- Delete: `src/rpc.rs`
- Optionally: `agent/` directory (entire TS agent)

- [ ] **Step 1: Remove rpc.rs**

Run: `rm src/rpc.rs`

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1`
Expected: clean compilation

- [ ] **Step 3: Remove old profiling instrumentation**

Remove the `trace!` macro and any `ITER_PROFILE` checks from `input.rs` and `main.rs` if they exist (they were added earlier for profiling).

---

### Task 8: Test end-to-end

**Steps:**

- [ ] **Step 1: Build release binary**

Run: `cargo build --release 2>&1 | tail -5`
Expected: binary at `target/release/iter`

- [ ] **Step 2: Quick smoke test (non-interactive)**

Run:
```bash
OPENROUTER_API_KEY="sk-or-..." ./target/release/iter --model "google/gemini-2.0-flash-001" ask "Say hello in one word"
```
Expected: prints agent header + "Hello" + newline

- [ ] **Step 3: Run benchmark**

Run: `OPENROUTER_API_KEY="sk-or-..." ITER_PROFILE=1 ./target/release/iter --model "google/gemini-2.0-flash-001" ask "Say hello in one word"`
Expected: measures first-token latency and total time

- [ ] **Step 4: Compare with old performance**

Compare the new first-token latency vs the old ~1292ms from profiling. Expect improvement due to eliminated IPC/serialization.
