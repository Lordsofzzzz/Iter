# RPC v2 Design (Strict Cutover)

## Goal
Replace the existing JSONL wire format with a strict v2 schema that separates push events from pull responses, supports request IDs, and never silently drops malformed lines. This is a **strict cutover**: only the new schema is accepted after the change.

## Scope
- Agent emits **push events** without a `kind` field.
- Agent emits **pull responses** with `kind: "response"` + `command` + `success` (+ optional `id`).
- TUI routes messages based on `kind` presence and surfaces unknown/malformed lines in the chat view.
- TUI continues to send line-delimited JSON commands (e.g., `get_state`, `get_session_stats`, `prompt`, `abort`, `clear`).

## Architecture
- **Agent (TypeScript)**
  - `readStdinLines()` implements strict LF-only framing, splitting exclusively on `\n` and stripping optional `\r`.
  - `emitEvent()` for push events, `emitResponse()` for pull responses — both write to stdout directly using `process.stdout.write`.
  - All responses to pull commands are wrapped in `kind: "response"` and echo any request `id`.
- **TUI (Rust)**
  - Parser peeks for `kind == "response"` to route pull responses vs push events.
  - Unknown/unparseable lines are preserved and shown in chat (no silent drops).

## Data Model
### Push Events (agent → TUI, unprompted)
No `kind` field:
- `agent_start`
- `turn_start`
- `text_delta { delta }`
- `turn_end`
- `agent_end`
- `error { message }`

### Pull Responses (agent → TUI, reply to command)
Always `{ kind: "response", command, success, id? }`:
- `get_state` → `StateData`
- `get_session_stats` → `SessionStatsData`
- `prompt`, `abort`, `clear` → success only
- Unknown command → `success:false` + `error`

### StateData
```
model_name:   string
model_limit:  number
temp:         number
is_streaming: boolean
```

### SessionStatsData
```
tokens: {
  input:       number
  output:      number
  cache_read:  number
  cache_write: number
  total:       number
}
context_usage: {
  tokens:  number
  limit:   number
  percent: number
}
cost:  number
turns: number
```

## Error Handling
- **Agent:** malformed input JSON → push `error` event and ignore the line.
- **TUI:** malformed JSON lines are shown as `Unknown { raw }` in chat.

## Compatibility
- **No backward compatibility.** Old format is rejected or surfaced as unknown.

## Testing
- Manual: send valid commands and confirm v2 responses include `kind: "response"` and `id` echoes.
- Manual: inject malformed JSON line and confirm it appears as an Unknown chat entry.
- Manual: ensure push events still stream correctly during a prompt.

