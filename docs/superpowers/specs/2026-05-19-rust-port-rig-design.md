# Rust Port: Embed LLM Agent with Rig

## Motivation

Eliminate the Node.js agent process (RPC bridge) to remove IPC overhead,
serialization latency, and process-spawn complexity. The single Rust binary
will handle TUI, agent loop, tool execution, and LLM API calls in-process.

## Architecture

```
┌─────────────────────────────────────────────┐
│                  iter binary                 │
│                                              │
│  TUI (input.rs, main.rs event loop)          │
│     │  rx.recv()                            │
│     ▼                                       │
│  Agent Channel (mpsc::Receiver<UiEvent>)     │
│     ▲  tx.send(event)                       │
│  Agent Task (tokio::spawn)                   │
│     ├── Rig Agent (openrouter provider)      │
│     │   └── stream_chat() → MultiTurnStream  │
│     ├── Tool Executor                        │
│     │   ├── read_file, write_file, edit      │
│     │   ├── run_command                      │
│     │   ├── list_files, search_files         │
│     └── Context Manager                      │
│         └── Message history + compaction     │
│                                              │
│  Config (clap + env)                          │
└─────────────────────────────────────────────┘
```

## Key Changes

| File | Action |
|------|--------|
| `src/main.rs` | Remove RPC channel, replace with in-process agent task + channel |
| `src/agent.rs` | **Remove** — replaced by Rig agent + tool executor |
| `src/rpc.rs` | **Remove** — no more JSONL wire protocol |
| `src/input.rs` | Minor changes (context compaction call, no RPC state) |
| `src/tools/` | **New** — Tool trait impls for read_file, write_file, edit, run_command, list_files, search_files |
| `src/agent_loop.rs` | **New** — Agent loop bridging Rig streaming to TuiEvent channel |
| `src/context.rs` | **New** — Message history with auto-compaction at 80% threshold |
| `Cargo.toml` | Add rig-core, tokio, other deps. Remove unused deps |

## Provider Strategy

- **Default: OpenRouter** (routes to any model via OpenAI-compatible API)
- OpenRouter provider selected via `openrouter::Client::new(api_key)`
- Model selection: pass any OpenRouter model ID string

## Event Flow (Agent → TUI)

Rig's `MultiTurnStreamItem` maps to Iter's `UiEvent`:

```
MultiTurnStreamItem::Text(text)         → UiEvent::TextDelta { delta: text }
MultiTurnStreamItem::ToolCall(name,args)→ UiEvent::ToolCall { name, input }
MultiTurnStreamItem::ToolResult(...)    → UiEvent::ToolResult { ... }
MultiTurnStreamItem::FinalResponse(fin) → UiEvent::AgentEnd { ... }
```

## Agent Loop Pseudocode

```
spawn on tokio task:
  loop:
    build Rig agent with tools
    stream = agent.stream_chat(prompt, history)
    for each item in stream:
      match item:
        Text(text)           → tx.send(TextDelta(text))
        ToolCall(id, fn,args)→ tx.send(ToolCall(name, args))
                               execute tool locally
                               tx.send(ToolResult(name, output))
        FinalResponse(resp)  → tx.send(AgentEnd)
                               break
    wait for next user prompt (channel from TUI)
```

## Async Bridge

- `tokio::spawn` the agent on a tokio runtime
- Agent sends `UiEvent` through `mpsc::Sender`
- TUI loop reads from `mpsc::Receiver` (same `rx.recv()` as today)
- TUI sends new prompts through a separate channel (`mpsc::Sender<String>`)

## Cargo Dependencies (new)

```toml
rig-core = { version = "0.37", features = ["derive"] }
tokio = { version = "1", features = ["rt", "macros", "sync"] }
reqwest = { version = "0.12", features = ["json", "stream"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

## Files to Remove

- `src/agent.rs` (entirely)
- `src/rpc.rs` (entirely)
- `agent/` directory (entire TS agent)
