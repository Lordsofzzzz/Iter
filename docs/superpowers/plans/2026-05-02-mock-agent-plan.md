# State-Machine Mock Agent Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a standalone TypeScript mock agent to test the Rust TUI's RPC handling and UI states.

**Architecture:** A single-file TypeScript script that reads JSONL from stdin, maintains a simple state machine for streaming, and emits JSONL events/responses to stdout with simulated delays.

**Tech Stack:** Bun/TypeScript (built-in `readline` and `process.stdout`).

---

### Task 1: Scaffolding and Basic RPC Loop

**Files:**
- Create: `agent/mock_agent.ts`

- [ ] **Step 1: Write the initial script with RPC loop**

```typescript
import { createInterface } from 'readline';

const rl = createInterface({ input: process.stdin });

function emit(payload: any) {
  console.log(JSON.stringify(payload));
}

rl.on('line', (line) => {
  try {
    const cmd = JSON.parse(line);
    handleCommand(cmd);
  } catch (e) {
    emit({ type: 'error', message: 'Invalid JSON' });
  }
});

function handleCommand(cmd: any) {
  const id = cmd.id;
  switch (cmd.type) {
    case 'get_state':
      emit({ kind: 'response', command: 'get_state', id, success: true, data: { model_name: 'mock-model', model_limit: 100000, is_streaming: false } });
      break;
    case 'prompt':
      emit({ kind: 'response', command: 'prompt', id, success: true });
      // Logic for triggers goes here in Task 2
      break;
    default:
      emit({ kind: 'response', command: cmd.type, id, success: false, error: 'Not implemented' });
  }
}
```

- [ ] **Step 2: Verify basic response**
Run: `echo '{"type":"get_state","id":"1"}' | bun run agent/mock_agent.ts`
Expected: `{"kind":"response","command":"get_state","id":"1","success":true,...}`

- [ ] **Step 3: Commit**
```bash
git add agent/mock_agent.ts
git commit -m "feat: initial mock agent scaffolding"
```

### Task 2: Implement Scenario Triggers (Echo & Error)

**Files:**
- Modify: `agent/mock_agent.ts`

- [ ] **Step 1: Add trigger logic to `handleCommand`**

```typescript
async function handleCommand(cmd: any) {
  const id = cmd.id;
  if (cmd.type === 'prompt') {
    emit({ kind: 'response', command: 'prompt', id, success: true });
    const content = cmd.content.toLowerCase();
    
    emit({ type: 'turn_start' });
    
    if (content.includes('error')) {
      await sleep(200);
      emit({ type: 'error', message: 'Simulated LLM Error' });
    } else {
      // Default Echo
      await sleep(200);
      emit({ type: 'text_delta', delta: `Mock echo: ${cmd.content}` });
      await sleep(100);
    }
    
    emit({ type: 'turn_end' });
    emit({ type: 'agent_end' });
  }
}

function sleep(ms: number) { return new Promise(r => setTimeout(r, ms)); }
```

- [ ] **Step 2: Verify error trigger**
Run: `echo '{"type":"prompt","content":"trigger error","id":"2"}' | bun run agent/mock_agent.ts`
Expected: Sequence including `{"type":"error",...}`

- [ ] **Step 3: Commit**
```bash
git commit -am "feat: add echo and error scenarios to mock agent"
```

### Task 3: Implement Tool Call Scenario

**Files:**
- Modify: `agent/mock_agent.ts`

- [ ] **Step 1: Add "tool" scenario to `handleCommand`**

```typescript
    if (content.includes('tool')) {
      emit({ type: 'text_delta', delta: 'Running a tool... ' });
      await sleep(500);
      emit({ type: 'tool_call', name: 'run_command', input: '{"cmd":"ls"}' });
      await sleep(500);
      emit({ type: 'tool_update', tool_call_id: 'tool-1', delta: 'Listing files...\n' });
      await sleep(300);
      emit({ type: 'tool_result', name: 'run_command', output: 'file1.txt\nfile2.txt' });
      await sleep(200);
      emit({ type: 'text_delta', delta: '\nTool finished.' });
    }
```

- [ ] **Step 2: Verify tool sequence**
Run: `echo '{"type":"prompt","content":"test tool","id":"3"}' | bun run agent/mock_agent.ts`
Expected: Sequence including `tool_call`, `tool_update`, `tool_result`.

- [ ] **Step 3: Commit**
```bash
git commit -am "feat: add tool call scenario to mock agent"
```

### Task 4: Implement Abort Handling

**Files:**
- Modify: `agent/mock_agent.ts`

- [ ] **Step 1: Add global `isStreaming` and `abort` handler**

```typescript
let isStreaming = false;
let stopStreaming = false;

// Inside handleCommand switch
    case 'abort':
      stopStreaming = true;
      emit({ kind: 'response', command: 'abort', id, success: true });
      break;

// Inside prompt logic
    isStreaming = true;
    stopStreaming = false;
    for (let i = 0; i < 10; i++) {
        if (stopStreaming) break;
        emit({ type: 'text_delta', delta: '.' });
        await sleep(500);
    }
    isStreaming = false;
```

- [ ] **Step 2: Verify abort**
(Requires manual verification of interleaving commands or a small test script)

- [ ] **Step 3: Commit**
```bash
git commit -am "feat: add abort support to mock agent"
```
