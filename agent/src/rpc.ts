import { logToFile } from './utils/logger';

// ── Push events (agent → TUI, unprompted) ──────────────────────────────
// No "kind" field. TUI identifies by absence of kind:"response".
export type PushEvent =
  | { type: 'agent_start' }
  | { type: 'turn_start' }
  | { type: 'text_delta'; delta: string }
  | { type: 'turn_end' }
  | { type: 'agent_end' }
  | { type: 'error'; message: string }
  | { type: 'cooldown'; wait_ms: number; retries_left: number }
  | { type: 'retry_result'; success: boolean; attempt: number };

// ── Pull responses (agent → TUI, reply to a command) ──────────────────
// Always has kind:"response" + command + success.
// TUI distinguishes from push events by checking kind === "response".
export type PullResponse =
  | { kind: 'response'; command: 'get_state';         id?: string; success: true;  data: StateData }
  | { kind: 'response'; command: 'get_session_stats'; id?: string; success: true;  data: SessionStatsData }
  | { kind: 'response'; command: 'prompt';            id?: string; success: true }
  | { kind: 'response'; command: 'abort';             id?: string; success: true }
  | { kind: 'response'; command: 'clear';             id?: string; success: true }
  | { kind: 'response'; command: string;              id?: string; success: false; error: string };

export interface StateData {
  model_name:  string;
  model_limit: number;
  temp:        number;
  is_streaming: boolean;
}

export interface SessionStatsData {
  tokens: {
    input:       number;
    output:      number;
    cache_read:  number;
    cache_write: number;
    total:       number;
  };
  context_usage: {
    tokens:  number;
    limit:   number;
    percent: number;
  };
  cost:  number;
  turns: number;
}

export type RpcMessage = PushEvent | PullResponse;

// ── emitters ────────────────────────────────────────────────────────────

// F1: write to stdout directly — never use console.log (adds \r\n on Windows)
// Strict LF-only framing. One JSON object per line, terminated by \n.
// ... existing code

function writeLine(obj: unknown): void {
  const json = JSON.stringify(obj) + '\n';
  process.stdout.write(json);
  logToFile(json.trim());
}

export function emitEvent(event: PushEvent): void {
  writeLine(event);
}

export function emitResponse(response: PullResponse): void {
  writeLine(response);
}

// Backward compatibility for current call sites.
export function emitRPC(event: RpcMessage): void {
  writeLine(event);
}

// ── F1: strict LF-only stdin reader ────────────────────────────────────
// readline MUST NOT be used — it splits on U+2028 / U+2029 which are
// valid inside JSON strings and would corrupt multi-line deltas.

type LineHandler = (line: string) => void | Promise<void>;

export function readStdinLines(handler: LineHandler): void {
  let buffer = '';

  process.stdin.setEncoding('utf8');

  process.stdin.on('data', (chunk: string) => {
    buffer += chunk;
    let newline: number;
    // F1: split ONLY on \n
    while ((newline = buffer.indexOf('\n')) !== -1) {
      const line = buffer.slice(0, newline).replace(/\r$/, ''); // strip optional \r
      buffer = buffer.slice(newline + 1);
      if (line.trim()) handler(line);
    }
  });

  process.stdin.on('end', () => process.exit(0));
}
