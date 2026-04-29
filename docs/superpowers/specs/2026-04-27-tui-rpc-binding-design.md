# TUI RPC Binding Design (Spawned Agent)

## Goal
Bind the Rust TUI to the Node agent via RPC JSONL by spawning the agent process, sending messages on Enter, and streaming `text_delta` events into the chat view.

## Architecture
- The TUI spawns `ts-node agent/src/index.ts` as a child process.
- A background thread reads agent stdout line-by-line, parses JSONL events, and forwards them to the UI loop via a channel.
- The UI loop polls the channel each tick to update state (messages, streaming flag).
- The TUI writes JSONL messages to the agent stdin when the user presses Enter.
- While the agent is streaming, input is disabled and additional sends are ignored.

## Data Flow
1. User types a prompt and presses Enter.
2. TUI sends `{ "type": "message", "content": "..." }` to agent stdin.
3. Agent emits JSONL events:
   - `agent_start` (once)
   - `turn_start`
   - repeated `text_delta`
   - `turn_end`
   - `agent_end`
4. TUI appends deltas to the latest assistant message in the chat.
5. When `agent_end` arrives, `streaming` is set to `false`, input re-enabled.

## Error Handling
- If the agent fails to spawn, a System message is appended in chat and input remains enabled.
- If a JSON line from stdout fails to parse, it is ignored and logged to stderr.
- If an event of type `error` is received, it is displayed as a System message and streaming ends.

## Testing
- Manual: run the TUI, send a message, confirm streaming deltas appear and input locks during streaming.
- Failure: run TUI without agent dependencies to confirm error message is surfaced in the UI.
