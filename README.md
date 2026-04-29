# Iter Coding Agent

> ⚠️ **Status: In Development** - This project is under active development and may have breaking changes.

A terminal-based AI coding assistant with a Rust TUI frontend and TypeScript agent backend.

## Overview

```
┌─────────────────────────────────────────────────────────────┐
│  Rust (ratatui) - Terminal UI                              │
│  ┌──────────┐  ┌──────────┐  ┌─────────────────────────┐  │
│  │ main.rs  │──│ agent.rs │──│ stdin/stdout JSONL       │  │
│  │ (TUI)    │  │ (spawn)  │  │ protocol                │  │
│  └──────────┘  └──────────┘  └───────────┬─────────────┘  │
│                                            │                │
│  ┌─────────────────────────────────────────┴─────────────┐  │
│  │ state/ (App state, messages, tokens, model info)      │  │
│  └───────────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ ui/ (chat, context, layout, theme)                    │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                         │ bun run
                         ▼
┌─────────────────────────────────────────────────────────────┐
│  TypeScript Agent                                           │
│  ┌──────────────────────────────────────────────────────┐    │
│  │ index.ts - command dispatcher (prompt, get_state)   │    │
│  └──────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────┐    │
│  │ llm/ - OpenRouter API client, history, stats         │    │
│  └──────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────┐    │
│  │ utils/ - retry, logger                               │    │
│  └──────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

## Features

- **Interactive TUI**: Full terminal UI with chat, context panel, and token tracking
- **Streaming Responses**: Real-time text delta streaming from the LLM
- **Token Tracking**: Input/output/cache token counts with cost estimation
- **Rate Limit Handling**: Automatic retry with exponential backoff and countdown display
- **Session Stats**: Turns, tools, cost, and context usage monitoring

## Prerequisites

- **Rust** (for TUI): `cargo`, `rustc`
- **Bun** (for agent): `bun`
- **OpenRouter API Key**: Set `OPENROUTER_API_KEY` environment variable

## Getting Started

### 1. Install Dependencies

```bash
# Rust dependencies
cargo build

# TypeScript dependencies
cd agent && bun install
```

### 2. Set API Key

```bash
export OPENROUTER_API_KEY="your-api-key-here"
```

### 3. Run the Application

```bash
cargo run
```

## Controls

| Key | Action |
|-----|--------|
| `Enter` | Send message to agent |
| `Ctrl+C` | Quit application |
| `Ctrl+U` | Clear input buffer |
| `Ctrl+L` | Clear chat history |
| `PgUp/PgDn` | Scroll chat history |
| `Up/Down` | Navigate history |

## Architecture

### Communication Protocol

The TUI and agent communicate via JSONL (JSON Lines) over stdin/stdout:

**Push Events** (Agent → TUI):
- `agent_start`, `turn_start`, `turn_end`, `agent_end`
- `text_delta` - streaming text chunks
- `error` - error messages
- `cooldown` - rate limit warning
- `retry_result` - retry attempt result

**Pull Responses** (TUI → Agent → TUI):
- `get_state` - current model configuration
- `get_session_stats` - token counts, cost, turns
- `prompt` - send user message
- `abort` - cancel current request
- `clear` - reset conversation

### File Structure

```
src/
├── main.rs        # Entry point, event loop
├── agent.rs       # Process spawning, message handling
├── rpc.rs         # Protocol types, parsing
├── state/
│   └── app.rs     # Application state
└── ui/
    ├── layout.rs  # Main layout
    ├── chat.rs   # Chat panel widget
    ├── context.rs # Context/tokens panel
    └── theme.rs  # Styling constants

agent/src/
├── index.ts       # Agent entry point
├── rpc.ts         # Protocol types
├── llm/
│   ├── client.ts  # OpenRouter client
│   ├── history.ts # Message history
│   └── stats.ts   # Session statistics
└── utils/
    ├── retry.ts   # Retry with backoff
    └── logger.ts  # File logging
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `OPENROUTER_API_KEY` | API key for OpenRouter | (required) |
| `MODEL_NAME` | LLM model to use | `google/gemma-4-31b-it:free` |

### Model Configuration

Default settings in `agent/src/llm/client.ts`:
- Model: `google/gemma-4-31b-it:free`
- Context: 200k tokens
- Temperature: 0.3

## Development Notice

This project is in active development. Expect:
- Breaking changes to internal APIs
- Incomplete features
- Evolving documentation

## License

MIT License

Copyright (c) 2024 Iter Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.