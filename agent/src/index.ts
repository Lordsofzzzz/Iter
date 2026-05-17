/**
 * Iter Coding Agent - Entry Point
 *
 * TypeScript agent process that handles LLM interactions.
 * Communicates with the Rust TUI via JSONL over stdin/stdout.
 */

import { LLMClient } from './llm/index.js';
import { listModels } from './llm/model-factory.js';
import { getActiveProvider, listProviders, setActiveProvider } from './llm/provider.js';
import { commandMetadata, emitEvent, emitResponse, parseCommandPayload, readStdinLines } from './rpc.js';
import { logToFile } from './utils/logger.js';

function clearHistory(client: LLMClient): void {
  client.clearHistory();
}

// ============================================================================
// Global State
// ============================================================================

const llm = new LLMClient();
let isStreaming = false;
let currentModel = llm.getModel();

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

  let command;
  try {
    command = parseCommandPayload(payload);
  } catch (err) {
    const meta = commandMetadata(payload);
    emitResponse({
      kind: 'response',
      command: meta.command,
      id: meta.id,
      success: false,
      error: String((err as Error)?.message ?? err),
    });
    return;
  }

  const { id } = command;

  // Dispatch command.
  switch (command.type) {

    case 'get_state':
      emitResponse({
        kind: 'response',
        command: 'get_state',
        id,
        success: true,
        data: {
          model_name:   llm.getModel(),
          model_limit:  llm.getModelLimit(),
          temp:         llm.getTemperature(),
          is_streaming: isStreaming,
        },
      });
      break;

    case 'get_session_stats': {
      const data = llm.getSessionStatsResponse(llm.getModelLimit());
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
      llm.setModel(command.model);
      currentModel = command.model;
      emitResponse({
        kind: 'response',
        command: 'set_model',
        id,
        success: true,
        data: { model_name: command.model, model_limit: llm.getModelLimit() },
      });
      break;
    }

    case 'set_provider': {
      const success = setActiveProvider(command.provider);
      if (!success) {
        emitResponse({
          kind: 'response',
          command: 'set_provider',
          id,
          success: false,
          error: `Unknown provider: ${command.provider}`,
        });
        break;
      }
      const provider = getActiveProvider();
      emitResponse({
        kind: 'response',
        command: 'set_provider',
        id,
        success: true,
        data: { provider: provider.id, model: llm.getModel() || undefined },
      });
      emitEvent({
        type: 'provider_changed',
        provider_id: provider.id,
        provider_name: provider.name,
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
      // Push a system event so TUI knows history is gone
      emitEvent({ type: 'agent_start' });
      break;

    case 'prompt':
    case 'message': {
      // ── Slash command dispatch ──────────────────────────────────────
      const text = command.content.trim();
      if (text.startsWith('/')) {
        emitResponse({
          kind: 'response',
          command: 'prompt',
          id,
          success: true,
        });
        handleSlashCommand(text);
        emitEvent({ type: 'turn_end', id });
        emitEvent({ type: 'agent_end', id, success: true });
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
        emitEvent({ type: 'agent_end', id, success: false, error: 'Agent busy' });
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
      try {
        await llm.streamResponse(command.content, currentModel, id);
      } finally {
        isStreaming = false;
      }
      break;
    }
  }
});

// ============================================================================
// Slash Command Handler
// ============================================================================

/**
 * Handles slash commands (/clear, /model).
 */
function handleSlashCommand(text: string): void {
  const [cmd, ...args] = text.split(' ');

  switch (cmd) {
    case '/clear':
      clearHistory(llm);
      emitEvent({ type: 'tool_result', name: 'clear', output: 'History cleared.' });
      // Push a system event so TUI knows history is gone
      emitEvent({ type: 'agent_start' });
      break;

    case '/model': {
      const modelName = args[0];
      if (!modelName) {
        emitEvent({
          type: 'tool_result',
          name: 'model',
          output: `Usage: /model <model-name>\nExample: /model anthropic/claude-sonnet-4-5\n\nUse /models to list available models.\nCurrent: ${currentModel || '(none)'}`,
        });
      } else {
        currentModel = modelName;
        llm.setModel(currentModel);
        emitEvent({
          type: 'tool_result',
          name: 'model',
          output: `Switched to: ${currentModel}`,
        });
      }
      break;
    }

    case '/provider': {
      const providerArg = args[0]?.toLowerCase();
      if (!providerArg) {
        const providers = listProviders();
        const list = providers.map(p => `${p.id}: ${p.name}`).join('\n');
        emitEvent({
          type: 'tool_result',
          name: 'provider',
          output: `Available providers:\n${list}\n\nUsage: /provider <name>\nCurrent: ${getActiveProvider().id}`,
        });
      } else {
        const success = setActiveProvider(providerArg);
        if (success) {
          llm.setModel('');
          currentModel = '';
          emitEvent({
            type: 'tool_result',
            name: 'provider',
            output: `Switched to: ${providerArg}. Select a model with /model <name>.`,
          });
        } else {
          emitEvent({
            type: 'tool_result',
            name: 'provider',
            output: `Unknown provider: ${providerArg}\nAvailable: ${listProviders().map(p => p.id).join(', ')}`,
          });
        }
      }
      break;
    }

    case '/models': {
      const modelsByProvider = listModels();
      const output = Object.entries(modelsByProvider)
        .map(([p, models]) => `${p}:\n  ${models.join('\n  ')}`)
        .join('\n\n');
      emitEvent({
        type: 'tool_result',
        name: 'models',
        output: `Available models:\n\n${output}`,
      });
      break;
    }

    default:
      emitEvent({
        type: 'tool_result',
        name: 'unknown',
        output: `Unknown command: ${cmd}\nAvailable: /clear, /model [0-3], /provider [name], /models`,
      });
  }
}
