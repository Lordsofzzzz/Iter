# Minimal RPC Streaming Backend Design

## Goal
Implement a minimal, stateless TypeScript LLM backend that communicates with the Rust TUI via strict JSONL over stdin/stdout, matching the core Pi streaming protocol.

## Architecture & Data Flow
- A single `index.ts` script acts as the entrypoint.
- Reads `stdin` line-by-line. Expects `{"type":"message", "content":"..."}`.
- Uses Vercel AI SDK (`streamText`) to call the LLM (stateless context).
- Streams output as `text_delta` chunks to `stdout` in strict JSONL format.
- Emits a specific lifecycle sequence: `agent_start` → `turn_start` → `text_delta`* → `turn_end` → `agent_end`.

## RPC Schema
**Input (stdin):**
- `{"type":"message", "content":"..."}`

**Output (stdout):**
1. `{"type":"agent_start"}` (Emitted once on startup)
2. `{"type":"turn_start"}` (Emitted when a message begins processing)
3. `{"type":"text_delta", "delta":"..."}` (Emitted for each text chunk from the LLM)
4. `{"type":"turn_end"}` (Emitted when the LLM stream completes)
5. `{"type":"agent_end"}` (Emitted to signify the agent is idle again)

**Errors:**
- `{"type":"error", "message":"..."}`

## Error Handling
- Invalid JSON parsing on stdin emits an error and the agent continues listening.
- LLM API failures emit an error event and immediately end the turn.
- No auto-recovery or retries in this minimal version.

## Testing
- Manual testing via CLI: pipe a JSON string into the script and verify the correct sequence of JSONL events on stdout.
- Integration testing: connect the Rust TUI and verify live typing works.
