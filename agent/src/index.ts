/**
 * Iter Coding Agent - Entry Point
 *
 * TypeScript agent process that handles LLM interactions.
 * Communicates with the Rust TUI via JSONL over stdin/stdout.
 */

import { LLMClient, MODEL_NAME, MODEL_LIMIT, clearHistory, setModel } from './llm/index.js';
import { emitEvent, emitResponse, readStdinLines, SessionStatsData } from './rpc.js';
import { logToFile } from './utils/logger.js';

// ============================================================================
// Configuration
// ============================================================================

/** List of available free models on OpenRouter. */
const FREE_MODELS = [
  'minimax/minimax-m2y5:free',
  'google/gemma-3-27b-it:free',
  'meta-llama/llama-4-maverick:free',
  'deepseek/deepseek-r1-0528:free',
] as const;

// ============================================================================
// Global State
// ============================================================================

const llm = new LLMClient();
let isStreaming = false;
let currentModel = MODEL_NAME;

// ============================================================================
// Initialization
// ============================================================================

// Notify TUI that agent has started.
emitEvent({ type: 'agent_start' });

/**
 * Main stdin handler — parses JSON commands from TUI.
 */
readStdinLines(async (line: string) => {
  // Log incoming command from TUI
  logToFile(`[IN] ${line}`);

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

    case 'set_model': {
      const model = payload.model;
      if (!model) {
        emitResponse({ kind: 'response', command: 'set_model', id, success: false, error: 'model field required' });
        break;
      }
      setModel(model);
      emitResponse({ kind: 'response', command: 'set_model', id, success: true });
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
      // Push a system event so TUI knows history is gone
      emitEvent({ type: 'agent_start' });
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

      // ── Slash command dispatch ──────────────────────────────────────
      const text = payload.content.trim();
      if (text.startsWith('/')) {
        handleSlashCommand(text, id);
        break;
      }
      // ── End slash command dispatch ─────────────────────────────────

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
      await llm.streamResponse(payload.content, currentModel);
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
// Slash Command Handler
// ============================================================================

/**
 * Handles slash commands (/clear, /model).
 */
function handleSlashCommand(text: string, id?: string): void {
  const [cmd, ...args] = text.split(' ');

  switch (cmd) {
    case '/clear':
      clearHistory(llm);
      emitResponse({
        kind: 'response',
        command: 'clear',
        id,
        success: true,
      });
      emitEvent({ type: 'tool_result', name: 'clear', output: 'History cleared.' });
      // Push a system event so TUI knows history is gone
      emitEvent({ type: 'agent_start' });
      break;

    case '/model': {
      const idx = parseInt(args[0] ?? '', 10);
      if (isNaN(idx) || idx < 0 || idx >= FREE_MODELS.length) {
        // List available models
        const list = FREE_MODELS.map((m, i) => `${i}: ${m}`).join('\n');
        emitEvent({
          type: 'tool_result',
          name: 'model',
          output: `Available models:\n${list}\n\nUsage: /model <0-${FREE_MODELS.length - 1}>`,
        });
      } else {
        currentModel = FREE_MODELS[idx];
        emitEvent({
          type: 'tool_result',
          name: 'model',
          output: `Switched to: ${currentModel}`,
        });
      }
      break;
    }

    default:
      emitEvent({
        type: 'tool_result',
        name: 'unknown',
        output: `Unknown command: ${cmd}\nAvailable: /clear, /model [0-${FREE_MODELS.length - 1}]`,
      });
  }
}

// ============================================================================
// Type Guards
// ============================================================================

/** Type guard for command payload. */
function isCommandPayload(value: unknown): value is {
  id?: string;
  type: string;
  content?: string;
  model?: string;
} {
  return (
    typeof value === 'object' &&
    value !== null &&
    'type' in value &&
    typeof (value as Record<string, unknown>).type === 'string'
  );
}
