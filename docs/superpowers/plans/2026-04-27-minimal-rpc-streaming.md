# Minimal RPC Streaming Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a minimal, stateless TypeScript LLM backend that processes `message` JSONL input via stdin and streams `text_delta` JSONL output via stdout.

**Architecture:** A single entrypoint (`index.ts`) reads stdin, passes the message to an LLM client wrapper (`llm.ts`), and emits strict JSONL events (`rpc.ts`) following the Pi-inspired lifecycle sequence.

**Tech Stack:** TypeScript, Node.js, Vercel AI SDK

---

### Task 1: Package Initialization

**Files:**
- Create: `agent/package.json`
- Create: `agent/tsconfig.json`

- [ ] **Step 1: Write package.json**

Create `agent/package.json`:
```json
{
  "name": "mini-pi-backend",
  "version": "1.0.0",
  "type": "module",
  "scripts": {
    "start": "ts-node src/index.ts"
  },
  "dependencies": {
    "@ai-sdk/openai": "^0.0.14",
    "ai": "^3.1.11"
  },
  "devDependencies": {
    "@types/node": "^20.0.0",
    "ts-node": "^10.9.2",
    "typescript": "^5.0.0"
  }
}
```

- [ ] **Step 2: Write tsconfig.json**

Create `agent/tsconfig.json`:
```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "esModuleInterop": true,
    "strict": true,
    "skipLibCheck": true
  },
  "include": ["src/**/*"]
}
```

- [ ] **Step 3: Commit**

```bash
git add agent/package.json agent/tsconfig.json
git commit -m "chore: setup minimal typescript backend package"
```

### Task 2: RPC Types and Emitter

**Files:**
- Create: `agent/src/rpc.ts`

- [ ] **Step 1: Define the minimal RPC event types**

Create `agent/src/rpc.ts`:
```typescript
export type RPCEvent =
  | { type: 'agent_start' }
  | { type: 'turn_start' }
  | { type: 'text_delta'; delta: string }
  | { type: 'turn_end' }
  | { type: 'agent_end' }
  | { type: 'error'; message: string };

export function emitRPC(event: RPCEvent) {
  console.log(JSON.stringify(event));
}
```

- [ ] **Step 2: Commit**

```bash
git add agent/src/rpc.ts
git commit -m "feat: implement JSONL RPC event emitter"
```

### Task 3: Stateless LLM Client

**Files:**
- Create: `agent/src/llm.ts`

- [ ] **Step 1: Write the streaming LLM client**

Create `agent/src/llm.ts`:
```typescript
import { streamText } from 'ai';
import { openai } from '@ai-sdk/openai';
import { emitRPC } from './rpc.js';

export class LLMClient {
  async streamResponse(userMessage: string) {
    emitRPC({ type: 'turn_start' });
    
    try {
      const { textStream } = await streamText({
        model: openai('gpt-4o'), // requires process.env.OPENAI_API_KEY
        messages: [{ role: 'user', content: userMessage }],
      });

      for await (const chunk of textStream) {
        emitRPC({ type: 'text_delta', delta: chunk });
      }
    } catch (e: any) {
      emitRPC({ type: 'error', message: `LLM error: ${e.message}` });
    }

    emitRPC({ type: 'turn_end' });
    emitRPC({ type: 'agent_end' }); // Emitted here because it's stateless and turn_end = agent_end
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add agent/src/llm.ts
git commit -m "feat: add stateless streaming LLM client"
```

### Task 4: Stdin Entrypoint

**Files:**
- Create: `agent/src/index.ts`

- [ ] **Step 1: Wire stdin reading to the LLM client**

Create `agent/src/index.ts`:
```typescript
import * as readline from 'readline';
import { LLMClient } from './llm.js';
import { emitRPC } from './rpc.js';

const llm = new LLMClient();

emitRPC({ type: 'agent_start' });

const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout,
  terminal: false
});

rl.on('line', async (line) => {
  if (!line.trim()) return;
  try {
    const payload = JSON.parse(line);
    if (payload.type === 'message' && typeof payload.content === 'string') {
      await llm.streamResponse(payload.content);
    } else {
      emitRPC({ type: 'error', message: 'Payload must be { type: "message", content: "..." }' });
    }
  } catch (e) {
    emitRPC({ type: 'error', message: 'Invalid input JSON' });
  }
});
```

- [ ] **Step 2: Commit**

```bash
git add agent/src/index.ts
git commit -m "feat: add stdin jsonl entrypoint"
```

### Task 5: Testing Setup & Instruction

**Files:**
- None (just execution instructions)

- [ ] **Step 1: Install dependencies**

Run: `cd agent && npm install`

- [ ] **Step 2: Test the RPC script manually**

Run: `echo '{"type":"message", "content":"Say hi briefly"}' | ts-node agent/src/index.ts`
*(Requires OPENAI_API_KEY to be set in environment)*

Expected output:
```jsonl
{"type":"agent_start"}
{"type":"turn_start"}
{"type":"text_delta","delta":"Hi"}
{"type":"text_delta","delta":" there"}
{"type":"text_delta","delta":"!"}
{"type":"turn_end"}
{"type":"agent_end"}
```
*(Exact text deltas will vary depending on LLM output)*
