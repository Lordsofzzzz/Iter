# Custom Agent Loop & Tool Truncation Design

**Goal:** Replace the Vercel AI SDK automatic multi-step loop with a custom manual loop inspired by `pi-mono`. This enables live streaming of tool `stdout` to the TUI and protects the LLM context window using robust tool-output truncation.

## Architecture

We are replacing `maxSteps: 10` in `streamText` with a manual `while(true)` generator loop in `agent/src/llm/client.ts`.

1. The loop calls `streamText` with the current `History` and yields tokens/tool-calls to the UI.
2. The loop waits for the assistant's turn to complete.
3. If no `toolCalls` exist, the `while` loop breaks (turn complete).
4. If `toolCalls` exist, the assistant's response is pushed to `History`.
5. The loop iterates through the requested tools, manually executing them.
6. The final tool results are pushed to `History`, and the loop restarts.

## Data Flow & Events

To support live tool execution streaming in the TUI, we need new RPC events between the TypeScript agent and the Rust TUI (`src/rpc.rs`):

- **New Event:** `PushEvent::ToolUpdate { tool_call_id: String, delta: String }` 
- **Updated Event:** `PushEvent::ToolResult { name: String, output: String, log_path: Option<String> }` (Added `log_path` to indicate truncation to the UI).

## Tool Implementation & Truncation (`agent/src/tools/index.ts`)

The `run_command` tool will be significantly refactored:
- Replace `execAsync` with a `spawn()` process.
- Add an `onUpdate: (chunk: string) => void` parameter to the tool's execution context.
- Capture `stdout` and `stderr` live, yielding chunks back to `client.ts` via `onUpdate`.
- Maintain a rolling memory buffer of **1000 lines** (or ~1MB). 
- If output exceeds the buffer, stream the excess to a temporary file (`/tmp/iter-cmd-<id>.log`).
- Return only the **truncated tail** of the output (e.g. `[... truncated N lines ...]\n{last 1000 lines}`) to the LLM, along with the log path.

The `read_file` tool will receive similar tail-truncation logic to protect against reading massive files into context.

## Error Handling

- If a spawned command fails, its non-zero exit code will be caught, but the rolling buffer of `stdout/stderr` will still be truncated and returned as the tool's result to the LLM so it can debug the failure.
- If the SDK throws an error during the text stream, we pop the user's message and abort (existing logic).

## Testing Strategy
- The loop and tools are core infrastructure. We will rely heavily on manual TUI testing with long-running commands (e.g., `npm install`, `find /`, `cat largefile`) to ensure the truncation buffer kicks in and the log file is generated.