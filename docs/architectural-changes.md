## Architectural Changes

### Before
```
User types prompt
  → Rust TUI sends JSONL command to TS agent
    → TS agent receives command
      → LLMClient.streamResponse()
        → runAgentLoop()
          → stream.ts routes to:
             • stream-vercel.ts (OpenAI-compat: OpenRouter, OpenAI, etc.)
               → Vercel AI SDK streamText() → Vercel internal SSE parse
               → toVercelMessages() → Vercel format → SDK converts to wire
               → agentToolToVercel() → zod schemas → SDK wraps
               → SDK yields fullStream events → we re-process into our events
             • stream-anthropic.ts (raw fetch) — unchanged
             • stream-google.ts (raw fetch) — unchanged
        → Events flow back: client.ts → emitEvent() → stdout JSONL
          → Rust TUI receives events
            • TextDelta → BUFFERED ONLY, NOT RENDERED ← CRITICAL BUG
            • ThinkingDelta → rendered immediately
            • ToolCall/ToolResult → rendered immediately
            → AgentEnd → print_response() dumps everything at once
```

### After
```
User types prompt
  → Rust TUI sends JSONL command to TS agent
    → TS agent receives command
      → LLMClient.streamResponse()
        → runAgentLoop()
          → stream.ts routes to:
             • stream-vercel.ts (ALL OpenAI-compat providers)
               → Raw fetch() + SSE line-by-line parsing
               → Direct wire format — no Vercel SDK, no zod, no intermediate formats
             • stream-anthropic.ts (raw fetch) — unchanged
             • stream-google.ts (raw fetch) — unchanged
        → Events flow back: client.ts → emitEvent() → stdout JSONL
          → Rust TUI receives events
            • TextDelta → WRITTEN TO STDOUT IMMEDIATELY ← FIXED
            • ThinkingDelta → rendered immediately
            • ToolCall/ToolResult → rendered immediately
            → AgentEnd → print_response() for proper markdown rendering
```

### Files Changed
| File | Before | After |
|---|---|---|
| `agent/src/llm/stream-vercel.ts` | 290 lines of Vercel SDK (`streamText`, `tool`, `zod`, `fullStream`) | 175 lines of raw `fetch()` + SSE parser |
| `agent/src/llm/stream.ts` | Passed only `model` + `temperature` to vercel path | Passes `baseUrl`, `apiKey`, `providerId` too |
| `agent/src/llm/model-factory.ts` | 131 lines, 8 `createXxxModel()` functions, dynamic ESM imports | 13 lines, just `listModels()` |
| `agent/src/llm/client.ts` | Imported `createModel`, `clearModelCache` (unused) | Only imports `listModels` |
| `agent/src/llm/index.ts` | Re-exported `createModel`, `clearModelCache` | Only re-exports `listModels` |
| `agent/package.json` | 5 dependencies: `ai`, `@ai-sdk/*` × 3, `@openrouter/ai-sdk-provider` | Zero runtime deps |
| `src/main.rs` | `TextDelta` → only buffered | `TextDelta` → printed immediately + buffered |

### Removed Layers
1. **Vercel AI SDK** (`ai` package) — was adding intermediate event processing layer
2. **Vercel provider packages** (`@ai-sdk/openai`, `@ai-sdk/anthropic`, `@ai-sdk/google`) — were unused for Anthropic/Google, replaced for OpenAI-compat
3. **OpenRouter provider** (`@openrouter/ai-sdk-provider`) — was creating LanguageModel instances
4. **Zod** — was wrapping tool schemas unnecessarily
5. **`createModel()` factory** — was doing ESM dynamic imports on every first call

### Net Effect
- **Same speed as pi** for the LLM layer (both use raw `fetch()` + SSE)
- **No framework upgrade risk** (zero SDK dependencies)
- **Screen no longer freezes** during streaming (TextDelta renders incrementally)
