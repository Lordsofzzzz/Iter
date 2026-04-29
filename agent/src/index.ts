/**
 * Iter Coding Agent - Entry Point
 *
 * TypeScript agent process that handles LLM interactions.
 * Communicates with the Rust TUI via JSONL over stdin/stdout.
 */

import { LLMClient, MODEL_NAME, MODEL_LIMIT, clearHistory } from './llm/index.js';
import { emitEvent, emitResponse, readStdinLines, SessionStatsData } from './rpc.js';

// ============================================================================
// Global State
// ============================================================================

const llm = new LLMClient();
let isStreaming = false;

// ============================================================================
// Initialization
// ============================================================================

// Notify TUI that agent has started.
emitEvent({ type: 'agent_start' });

/**
 * Main stdin handler — parses JSON commands from TUI.
 */
readStdinLines(async (line: string) => {
  let payload: unknown;

  try {
    payload = JSON.parse(line);
  } catch {
    emitEvent({ type: 'error', message: 'Invalid JSON on stdin' });
    return;
  }

  // Validate payload shape.
  if (!isCommandPayload(payload)) {
    emitEvent({ type: 'error', message: 'Invalid command payload' });
    return;
  }

  const id = typeof payload.id === 'string' ? payload.id : undefined;

  // Dispatch command.
  switch (payload.type) {

    case 'get_state':
      emitResponse({
        kind: 'response',
        command: 'get_state',
        id,
        success: true,
        data: {
          model_name:   MODEL_NAME,
          model_limit:  MODEL_LIMIT,
          temp:         0.3,
          is_streaming: isStreaming,
        },
      });
      break;

    case 'get_session_stats': {
      const data = llm.getSessionStatsResponse(MODEL_LIMIT);
      emitResponse({
        kind: 'response',
        command: 'get_session_stats',
        id,
        success: true,
        data,
      });
      break;
    }

    case 'abort':
      llm.abort();
      emitResponse({
        kind: 'response',
        command: 'abort',
        id,
        success: true,
      });
      break;

    case 'clear':
      clearHistory(llm);
      emitResponse({
        kind: 'response',
        command: 'clear',
        id,
        success: true,
      });
      break;

    case 'prompt':
    case 'message': {
      // Validate content field.
      if (typeof payload.content !== 'string') {
        emitResponse({
          kind: 'response',
          command: 'prompt',
          id,
          success: false,
          error: 'content must be a string',
        });
        break;
      }

      // Reject if already streaming.
      if (isStreaming) {
        emitResponse({
          kind: 'response',
          command: 'prompt',
          id,
          success: false,
          error: 'Agent busy',
        });
        break;
      }

      // Accept the prompt.
      emitResponse({
        kind: 'response',
        command: 'prompt',
        id,
        success: true,
      });

      isStreaming = true;
      await llm.streamResponse(payload.content);
      isStreaming = false;
      break;
    }

    default:
      emitResponse({
        kind: 'response',
        command: payload.type ?? 'unknown',
        id,
        success: false,
        error: `Unknown command: ${payload.type}`,
      });
  }
});

// ============================================================================
// Type Guards
// ============================================================================

/** Type guard for command payload. */
function isCommandPayload(value: unknown): value is { id?: string; type: string; content?: string } {
  return (
    typeof value === 'object' &&
    value !== null &&
    'type' in value &&
    typeof (value as Record<string, unknown>).type === 'string'
  );
}