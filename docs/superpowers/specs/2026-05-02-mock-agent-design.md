# Design Spec: State-Machine Mock Agent for TUI Testing

## Purpose
To provide a reliable, predictable, and controllable mock of the TypeScript agent process. This allows testing the Rust TUI (`agent-tui`) across various edge cases, UI states, and event sequences without requiring an actual LLM or OpenRouter API key.

## Architecture
The mock agent will be a standalone Bun/TypeScript script (`mock_agent.ts`) that implements the JSONL-based RPC protocol used by `Iter`. It will be spawned by the TUI in place of the real agent.

## Core Behavior
- **RPC Interface**: Reads JSON commands from `stdin`, writes JSON events/responses to `stdout`.
- **Latency Simulation**: Uses `setTimeout` to mimic the asynchronous nature of LLM streaming and tool execution.
- **State Management**: Tracks if it's currently "streaming" to handle `abort` and `prompt` concurrency rules.

## Scenario Triggers (Keyword Based)
The mock will look for specific keywords in the prompt `content` to trigger scenarios:

| Keyword | Scenario | Sequence |
|---------|----------|----------|
| `error` | LLM Error | `turn_start` -> `error` event |
| `tool` | Tool Execution | `turn_start` -> `tool_call` -> `tool_update` (streaming) -> `tool_result` -> `turn_end` |
| `think` | Reasoning | `turn_start` -> `thinking_delta` -> `text_delta` -> `turn_end` |
| `stats` | Usage Stats | Updates internal token counters and emits `get_session_stats` response |
| `abort` | Abort Test | Starts a long stream that only finishes if not aborted |
| *default* | Echo Response | `turn_start` -> `text_delta` (Echoing prompt) -> `turn_end` |

## Simulated RPC Responses
- `get_state`: Returns hardcoded model metadata (`minimax-m2.5:free`).
- `get_session_stats`: Returns incrementing counters for input/output tokens.
- `clear`: Resets state and emits `agent_start`.
- `set_model`: Responds with `success: true`.

## Implementation Details
- **File**: `agent/mock_agent.ts`
- **Runner**: `bun run agent/mock_agent.ts`
- **Dependencies**: Minimal (only `readline` for stdin parsing).

## Success Criteria
1. The Rust TUI can successfully spawn the mock agent.
2. The TUI renders streaming text and thinking blocks.
3. The TUI correctly displays tool call progress and results.
4. The TUI's "Abort" button stops the mock's stream.
5. Error events from the mock are displayed in the TUI error area.
