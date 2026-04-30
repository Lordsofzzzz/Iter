/**
 * RPC wire protocol for TUI ↔ Agent communication.
 *
 * Uses JSONL (one JSON object per line) over stdout/stdin.
 * Two message directions:
 *   - Push: Agent → TUI (unprompted events like text deltas)
 *   - Pull: TUI → Agent → TUI (request/response pattern)
 */

import { logToFile } from './utils/logger';

// ============================================================================
// Push Events: Agent → TUI (unprompted)
// ============================================================================

/** Unprompted events from the agent. Identified by absence of `kind` field. */
export type PushEvent =
  | { type: 'agent_start' }
  | { type: 'turn_start' }
  | { type: 'text_delta'; delta: string }
  | { type: 'turn_end' }
  | { type: 'agent_end' }
  | { type: 'error'; message: string }
  | { type: 'cooldown'; wait_ms: number; retries_left: number }
  | { type: 'retry_result'; success: boolean; attempt: number }
  | { type: 'tool_call';   name: string; input: string }   // ← new
  | { type: 'tool_result'; name: string; output: string }; // ← new

// ============================================================================
// Pull Responses: TUI → Agent → TUI
// ============================================================================

/** Response to a TUI command. Always has `kind: "response"`. */
export type PullResponse =
  | { kind: 'response'; command: 'get_state';         id?: string; success: true;  data: StateData }
  | { kind: 'response'; command: 'get_session_stats'; id?: string; success: true;  data: SessionStatsData }
  | { kind: 'response'; command: 'prompt';            id?: string; success: true }
  | { kind: 'response'; command: 'abort';             id?: string; success: true }
  | { kind: 'response'; command: 'clear';             id?: string; success: true }
  | { kind: 'response'; command: 'set_model';        id?: string; success: true;  data?: { model: string } }
  | { kind: 'response'; command: string;              id?: string; success: false; error: string };

// ============================================================================
// Data Shapes
// ============================================================================

/** Model configuration data. */
export interface StateData {
  model_name:  string;
  model_limit: number;
  temp:        number;
  is_streaming: boolean;
}

/** Token usage breakdown. */
export interface TokenUsage {
  input:       number;
  output:      number;
  cache_read:  number;
  cache_write: number;
  total:       number;
}

/** Context window usage. */
export interface ContextInfo {
  tokens:  number;
  limit:   number;
  percent: number;
}

/** Session statistics for the UI. */
export interface SessionStatsData {
  tokens:        TokenUsage;
  context_usage: ContextInfo;
  cost:          number;
  turns:         number;
}

/** Union type for any RPC message. */
export type RpcMessage = PushEvent | PullResponse;

// ============================================================================
// Emitters (stdout)
// ============================================================================

/**
 * Writes a JSON object to stdout as a single line (JSONL format).
 * Uses LF-only framing to avoid corruption on Windows.
 */
function writeLine(obj: unknown): void {
  const json = JSON.stringify(obj) + '\n';
  process.stdout.write(json);
  logToFile(json.trim());
}

/** Emits a push event to the TUI. */
export function emitEvent(event: PushEvent): void {
  writeLine(event);
}

/** Emits a pull response to the TUI. */
export function emitResponse(response: PullResponse): void {
  writeLine(response);
}

/** Backward compatibility wrapper. */
export function emitRPC(event: RpcMessage): void {
  writeLine(event);
}

// ============================================================================
// Input Reader (stdin)
// ============================================================================

type LineHandler = (line: string) => void | Promise<void>;

/**
 * Reads stdin line-by-line using raw chunk parsing.
 *
 * NOTE: Does NOT use readline module — it splits on U+2028/U+2029 which
 * are valid inside JSON strings and would corrupt multi-line deltas.
 * Splits ONLY on \n (LF).
 */
export function readStdinLines(handler: LineHandler): void {
  let buffer = '';

  process.stdin.setEncoding('utf8');

  process.stdin.on('data', (chunk: string) => {
    buffer += chunk;
    let newline: number;

    // Split ONLY on \n, strip trailing \r for Windows compatibility.
    while ((newline = buffer.indexOf('\n')) !== -1) {
      const line = buffer.slice(0, newline).replace(/\r$/, '');
      buffer = buffer.slice(newline + 1);
      if (line.trim()) {
        handler(line);
      }
    }
  });

  process.stdin.on('end', () => process.exit(0));
}