# Iter Coding Agent

> Status: In Development - this project may have breaking changes.

A command-line AI coding assistant with a Rust CLI wrapper and TypeScript agent backend.

## Overview

```
Rust CLI
  main.rs     command entry point
  agent.rs    TypeScript agent process management
  rpc.rs      stdin/stdout JSONL protocol types
  state/      minimal session state

TypeScript Agent
  agent/src/index.ts
  agent/src/llm/
  agent/src/tools/
```

The Rust binary starts the TypeScript agent with `bun`, sends commands over JSONL, streams response text to stdout, and writes agent stderr logs to `agent/logs`.

## Features

- Single-prompt CLI execution
- Streaming response output
- Tool call and retry notices on stderr
- OpenRouter-backed model configuration through the TypeScript agent

## Prerequisites

- Rust: `cargo`, `rustc`
- Bun: `bun`
- OpenRouter API key in `OPENROUTER_API_KEY`

## Getting Started

Install dependencies:

```bash
cargo build
cd agent && bun install
```

Set an API key:

```bash
export OPENROUTER_API_KEY="your-api-key-here"
```

Show the command surface:

```bash
cargo run --bin iter -- --help
```

Run a single prompt:

```bash
cargo run --bin iter -- ask "summarize this repository"
```

Use a specific model or working directory:

```bash
cargo run --bin iter -- --model "google/gemini-2.5-pro" -C /path/to/project ask "inspect the code"
```

## Architecture

The CLI and agent communicate via JSONL over stdin/stdout.
Commands carry an `id`; responses and terminal turn events echo that `id`.
A submitted prompt is complete only after `agent_end` with `{ success: true }`
or `{ success: false, error }`.

Push events from the agent include:

- `agent_start`, `turn_start`, `turn_end`, `agent_end`
- `text_delta`
- `thinking_delta`
- `tool_call`, `tool_result`, `tool_update`
- `error`
- `cooldown`, `retry_result`, `auto_retry_start`, `auto_retry_end`


Pull responses from the agent include:

- `get_state`
- `get_session_stats`
- `set_model`
- `prompt`

## File Structure

```
src/
├── main.rs        # CLI entry point
├── cli.rs         # clap command definitions
├── agent.rs       # process spawning and message handling
├── rpc.rs         # protocol types and parsing
└── state/
    └── mod.rs     # minimal runtime state

agent/src/
├── index.ts       # agent entry point
├── rpc.ts         # protocol types
├── llm/
│   ├── client.ts
│   ├── history.ts
│   └── stats.ts
└── utils/
    ├── retry.ts
    └── logger.ts
```

## Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `OPENROUTER_API_KEY` | API key for OpenRouter | required |
| `MODEL_NAME` | LLM model to use | agent default |

## License

MIT License
