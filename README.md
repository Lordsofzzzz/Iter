# Iter Coding Agent

> Status: In Development — breaking changes possible.

A command-line AI coding assistant. Single Rust binary: TUI, agent loop, tool execution, and LLM calls all in-process via [rig-core](https://github.com/0xPlaygrounds/rig).

## Architecture

```
iter (single binary)
  main.rs          CLI entry point, TUI event loop
  cli.rs           clap command definitions
  agent.rs         Rig agent loop, streams events to TUI via mpsc
  agent_event.rs   AgentEvent / TuiCommand channel types
  context.rs       Message history with auto-compaction at 80% usage
  tools.rs         Tool impls: read_file, write_file, edit, run_command, list_files, search_files
  input.rs         Inline bordered input box with slash-command picker
  state/mod.rs     Minimal runtime state for status bar
```

LLM calls go through OpenRouter. Any OpenRouter model ID is accepted.

## Prerequisites

- Rust (`cargo`, `rustc`)
- `OPENROUTER_API_KEY` environment variable

## Getting Started

```bash
export OPENROUTER_API_KEY="your-key-here"
cargo build --release
```

Run a single prompt:

```bash
cargo run --bin iter -- ask "summarize this repository"
```

Interactive mode (no prompt):

```bash
cargo run --bin iter -- ask
```

Use a specific model or working directory:

```bash
cargo run --bin iter -- --model "google/gemini-2.5-pro" -C /path/to/project ask "inspect the code"
```

## Options

| Flag | Env | Description |
|------|-----|-------------|
| `-m`, `--model` | `MODEL_NAME` | OpenRouter model ID (default: `deepseek/deepseek-v4-flash:free`) |
| `-C`, `--workdir` | — | Change working directory before running |
| `--show-thinking` | — | Print model reasoning/thinking tokens |

## Keyboard Shortcuts (interactive mode)

| Key | Action |
|-----|--------|
| `↵` | Submit prompt |
| `^k` | Abort current request |
| `^u` | Clear input |
| `^c` / `^d` | Quit |
| `/` | Slash-command picker |

## Slash Commands

| Command | Effect |
|---------|--------|
| `/model` | Switch active model |
| `/provider` | Switch provider |
| `/clear` | Clear conversation history |
| `/abort` | Abort current request |
| `/help` | Show available commands |

## Tools available to the agent

- `read_file` — read a file
- `write_file` — write/create a file
- `edit` — replace first occurrence of text in a file
- `run_command` — execute a shell command (stdout+stderr interleaved)
- `list_files` — list a directory
- `search_files` — glob search for files

## License

MIT
