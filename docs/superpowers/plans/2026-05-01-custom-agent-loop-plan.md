# Custom Agent Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace AI SDK automatic loop with a manual generator loop supporting live tool stdout streaming and tail-truncation.

**Architecture:** Custom `while(true)` loop calling `streamText`. Tools executed manually with child_process spawn for `run_command` and rolling buffer truncation. New `ToolUpdate` RPC events.

**Tech Stack:** TypeScript, Vercel AI SDK, Node.js child_process, Rust (TUI RPC).

---

### Task 1: Update RPC Types for Live Tool Output

**Files:**
- Modify: `src/rpc.rs`
- Modify: `agent/src/rpc.ts`

- [ ] **Step 1: Write the failing test / Verify rust compilation**
(No formal Rust tests for `rpc.rs` exist, but we verify compilation.)

Run: `cargo check`
Expected: PASS

- [ ] **Step 2: Add `ToolUpdate` event to Rust RPC definitions**

Modify `src/rpc.rs`:
```rust
pub enum PushEvent {
    // ...
    RetryResult { success: bool, attempt: u32 },
    ToolCall { name: String, input: String },
    ToolUpdate { tool_call_id: String, delta: String }, // ← new
    ToolResult { name: String, output: String, log_path: Option<String> }, // ← modified
}
```

- [ ] **Step 3: Update TypeScript RPC types**

Modify `agent/src/rpc.ts`:
```typescript
export type EventPayload =
  // ...
  | { type: 'tool_call'; name: string; input: string }
  | { type: 'tool_update'; tool_call_id: string; delta: string } // ← new
  | { type: 'tool_result'; name: string; output: string; log_path?: string } // ← modified
  | { type: 'error'; message: string };
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo check` and `npm run build --prefix agent`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/rpc.rs agent/src/rpc.ts
git commit -m "feat: add ToolUpdate RPC event"
```

### Task 2: Implement Live Output & Truncation in `run_command`

**Files:**
- Modify: `agent/src/tools/index.ts`

- [ ] **Step 1: Write minimal implementation for `run_command`**

Replace `execAsync` with a custom `spawn` implementation that tracks an in-memory rolling buffer (e.g. up to 1000 lines or ~1MB), streams `tool_update` events via `emitEvent`, and saves excess output to `/tmp/iter-cmd-...log`.

Modify `agent/src/tools/index.ts`:
```typescript
import { spawn } from 'child_process';
import { randomBytes } from 'crypto';
import { createWriteStream } from 'fs';
import { tmpdir } from 'os';
import { emitEvent } from '../rpc.js';

// ... inside run_command definition ...
    execute: async ({ cmd, cwd }, { toolCallId }) => {
      // ... blocked check ...
      return new Promise((resolve) => {
        const child = spawn(cmd, { cwd: cwd ?? process.cwd(), shell: true });
        
        let logPath: string | undefined;
        let logStream: ReturnType<typeof createWriteStream> | undefined;
        const chunks: Buffer[] = [];
        let totalBytes = 0;
        const MAX_BYTES = 1024 * 1024; // 1MB

        const handleData = (data: Buffer) => {
          // Stream to TUI
          emitEvent({ type: 'tool_update', tool_call_id: toolCallId, delta: data.toString('utf-8') });
          
          totalBytes += data.length;
          if (totalBytes > MAX_BYTES) {
            if (!logPath) {
              logPath = join(tmpdir(), `iter-cmd-${randomBytes(8).toString('hex')}.log`);
              logStream = createWriteStream(logPath);
              for (const c of chunks) logStream.write(c);
            }
          }
          if (logStream) logStream.write(data);
          
          chunks.push(data);
          // Keep only last ~1MB in memory
          let currentBytes = chunks.reduce((acc, c) => acc + c.length, 0);
          while (currentBytes > MAX_BYTES && chunks.length > 1) {
            const removed = chunks.shift()!;
            currentBytes -= removed.length;
          }
        };

        child.stdout.on('data', handleData);
        child.stderr.on('data', handleData);

        child.on('close', (code) => {
          if (logStream) logStream.end();
          const finalBuffer = Buffer.concat(chunks);
          let output = finalBuffer.toString('utf-8').trim();
          
          if (logPath) {
            output = `[... truncated ...]\n${output}\n\n(Full log saved to: ${logPath})`;
          }
          
          if (code !== 0) output = `EXIT ${code}:\n${output}`;
          if (!output) output = '(no output)';
          
          resolve(output);
        });
        
        child.on('error', (err) => resolve(`ERROR: ${err.message}`));
      });
    }
```

- [ ] **Step 2: Run test to verify it passes**

Run: `npm run build --prefix agent`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add agent/src/tools/index.ts
git commit -m "feat: live stdout streaming and truncation for run_command"
```

### Task 3: Refactor the LLM Agent Loop in `client.ts`

**Files:**
- Modify: `agent/src/llm/client.ts`

- [ ] **Step 1: Write the manual generator loop**

Remove `maxSteps: 10` from `streamText`. Wrap the logic in a `while(true)` loop. `streamText` executes tools automatically but STOPS after 1 step. We just loop and re-invoke it with the appended history.

Modify `agent/src/llm/client.ts`:
```typescript
  async streamResponse(userMessage: string, model?: string): Promise<void> {
    if (model) MODULE_NAME_OVERRIDE = model;

    this.history.pushUser(userMessage);
    emitEvent({ type: 'turn_start' });

    this.abortController = new AbortController();

    try {
      await retry(async () => {
        // We do not clear assistantText inside the loop to preserve cumulative output
        while (true) {
          const isThinkingModel = MODULE_NAME_OVERRIDE.includes(':thinking') || // ...
          
          const wrappedModel = wrapLanguageModel({ /* ... */ });

          const result = await streamText({
            model: wrappedModel,
            temperature: MODEL_TEMP,
            system: buildSystemPrompt(),
            messages: this.history.get(),
            tools,
            // NO maxSteps - defaults to 1, meaning it streams, executes tools, and returns
            abortSignal: this.abortController!.signal,
            onError: () => {},
            // ...
          });

          for await (const chunk of result.fullStream) {
            switch (chunk.type) {
              case 'reasoning-delta': // ...
              case 'text-delta': // ...
              case 'tool-call':
                emitEvent({
                  type: 'tool_call',
                  name: chunk.toolName,
                  input: JSON.stringify(chunk.args),
                });
                break;
              case 'tool-result':
                emitEvent({
                  type: 'tool_result',
                  name: chunk.toolName,
                  output: typeof chunk.output === 'string' ? chunk.output : JSON.stringify(chunk.output),
                });
                break;
              case 'error': throw chunk.error;
            }
          }

          const response = await result.response;
          // Append all assistant messages AND tool result messages to history
          for (const msg of response.messages) {
            this.history.push(msg);
          }

          // ... stat tracking ...

          // Continue looping only if there were tool calls
          const hasToolCalls = response.messages.some(m => m.role === 'assistant' && m.toolCalls && m.toolCalls.length > 0);
          if (!hasToolCalls) {
            break;
          }
        }
      }, this.abortController!.signal);
    } catch (error: unknown) {
      // ...
    }
  }
```

- [ ] **Step 2: Run test to verify it passes**

Run: `npm run build --prefix agent`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add agent/src/llm/client.ts
git commit -m "feat: custom manual agent loop to replace maxSteps"
```
