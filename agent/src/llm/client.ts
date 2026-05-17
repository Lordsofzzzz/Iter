/**
 * LLM client — pi-style agent loop over direct OpenRouter SSE.
 *
 * No Vercel AI SDK. Uses:
 *   - stream.ts   → direct fetch to OpenRouter /v1/chat/completions
 *   - agent-loop.ts → pi-identical while-loop with tool execution hooks
 */

import { emitEvent, SessionStatsData } from '../rpc.js';
import { getRetryConfig, isRetryEnabled, getTemperature } from '../config.js';
import { isGenericRetryable } from './provider-error.js';
import { Stats } from './stats.js';
import { runAgentLoop }                 from './agent-loop.js';
import { buildSystemPrompt }            from '../system-prompt.js';
import { tools }                        from '../tools/index.js';
import type { AgentLoopEvent, Message, AssistantMessage } from './types.js';
import { transformContext } from './context.js';
import { createModel, listModels, clearModelCache } from './model-factory.js';
import { getActiveProvider, setActiveProvider, listProviders } from './provider.js';

// ── Config ────────────────────────────────────────────────────────────────────

const FALLBACK_LIMIT = 128_000;

export const MODEL_TEMP = getTemperature();

// model_id → context window size, populated at startup from OpenRouter
const _contextWindowMap = new Map<string, number>();

export function getModelLimit(): number {
  return FALLBACK_LIMIT;
}

export function setModelLimit(model: string, limit: number): void {
  _contextWindowMap.set(model, limit);
}

// ── Client ────────────────────────────────────────────────────────────────────

export class LLMClient {
  private messages: Message[] = [];
  private readonly stats           = new Stats();
  private abortController: AbortController | null = null;
  private cachedSystemPrompt: string | null = null;
  private activeRequestId: string | undefined;
  // Session-level retry counter — lives outside the loop, resets on success (pi pattern)
  private _retryAttempt = 0;

  // Instance-level model state (replaces module globals)
  private _model = process.env.MODEL_NAME ?? '';
  private _modelLimit = FALLBACK_LIMIT;

  getModel(): string {
    return this._model;
  }

  getModelLimit(): number {
    return this._modelLimit;
  }

  setModel(model: string, limit?: number): void {
    this._model = model;
    this._modelLimit = limit ?? _contextWindowMap.get(model) ?? FALLBACK_LIMIT;
  }

  getTemperature(): number {
    return MODEL_TEMP;
  }

  private getSystemPrompt(): string {
    if (!this.cachedSystemPrompt) {
      this.cachedSystemPrompt = buildSystemPrompt();
    }
    return this.cachedSystemPrompt;
  }

  getSessionStats() { return this.stats.get(); }

  getSessionStatsResponse(modelLimit: number): SessionStatsData {
    const s    = this.stats.get();
    const currentChars = this.messages.reduce((acc, msg) => {
      if (msg.role === 'user') return acc + msg.content.length;
      if (msg.role === 'assistant') return acc + JSON.stringify(msg.content).length;
      if (msg.role === 'toolResult') return acc + msg.content.map(c => c.text).join('').length;
      return acc;
    }, 0);
    const currentTokens = Math.ceil(currentChars / 4);
    return {
      tokens: {
        input:       s.tokens.input,
        output:      s.tokens.output,
        cache_read:  s.tokens.cache_read,
        cache_write: s.tokens.cache_write,
        total:       s.tokens.total,
      },
      context_usage: {
        tokens:  currentTokens,
        limit:   modelLimit,
        percent: parseFloat(((currentTokens / modelLimit) * 100).toFixed(1)),
      },
      cost:  s.cost,
      turns: s.turns,
    };
  }

  abort(): void {
    this.abortController?.abort();
    this.abortController = null;
  }

  clearHistory(): void {
    this.messages = [];
    this.stats.reset();
    this.cachedSystemPrompt = null; // refresh git branch etc. on next turn
  }

  async streamResponse(userMessage: string, model?: string, requestId?: string): Promise<void> {
    this.activeRequestId = requestId;
    let success = true;
    let terminalError: string | undefined;

    if (model) this.setModel(model);

    if (!this._model) {
      success = false;
      terminalError = 'No model selected. Use /model <name> to set one.';
      emitEvent({ type: 'error', id: requestId, message: terminalError });
      emitEvent({ type: 'agent_end', id: requestId, success, error: terminalError });
      this.activeRequestId = undefined;
      return;
    }

    this.abortController = new AbortController();
    const signal = this.abortController.signal;

    const retryCfg   = getRetryConfig();
    const maxRetries = isRetryEnabled() ? retryCfg.maxRetries : 0;

    try {
      // ── Pi-style session-level retry ──────────────────────────────────────
      // The agent loop is dumb (single LLM call, emits agent_end on error).
      // We check the result here and re-run with backoff — counter survives
      // across tool-call turns. Mirrors pi-mono agent-session._handleRetryableError.
      while (true) {
        const newMessages = await runAgentLoop(
          userMessage,
          {
            systemPrompt: this.getSystemPrompt(),
            messages:     [...this.messages],
            tools,
          },
          {
            model:            this.getModel(),
            temperature:      this.getTemperature(),
            toolExecution:    'parallel',
            transformContext: async (msgs) => transformContext(msgs),
          },
          (event: AgentLoopEvent) => this.handleLoopEvent(event),
          signal,
        );

        // Persist messages.
        for (const msg of newMessages) this.messages.push(msg);

        // Sum token stats.
        for (const msg of newMessages) {
          if (msg.role === 'assistant' && (msg as AssistantMessage).usage) {
            const u = (msg as AssistantMessage).usage;
            this.stats.addTokens(u.input, u.output, u.cacheRead, u.cacheWrite);
          }
        }

        const lastAssistant = [...newMessages]
          .reverse()
          .find((m): m is AssistantMessage => m.role === 'assistant');

        if (lastAssistant?.stopReason === 'error' || lastAssistant?.stopReason === 'aborted') {
          const errMsg = lastAssistant.errorMessage ?? (lastAssistant.stopReason === 'aborted' ? 'Aborted' : 'unknown');

          if (lastAssistant.stopReason === 'error' && this._retryAttempt < maxRetries && isGenericRetryable(0, errMsg)) {
            this._retryAttempt++;
            const delayMs = retryCfg.baseDelayMs * Math.pow(2, this._retryAttempt - 1);

            emitEvent({
              type:         'auto_retry_start',
              attempt:      this._retryAttempt,
              maxAttempts:  maxRetries,
              delayMs,
              errorMessage: errMsg,
            });

            // Pop the failed assistant message so next run doesn't see it.
            if (this.messages.at(-1)?.role === 'assistant') this.messages.pop();

            // Abortable sleep.
            await new Promise<void>((resolve, reject) => {
              if (signal.aborted) { reject(new DOMException('Aborted', 'AbortError')); return; }
              const t = setTimeout(resolve, delayMs);
              signal.addEventListener('abort', () => { clearTimeout(t); reject(new DOMException('Aborted', 'AbortError')); }, { once: true });
            });

            continue; // re-run loop
          }

          // Non-retryable or exhausted.
          if (this._retryAttempt > 0) {
            emitEvent({ type: 'auto_retry_end', success: false, attempt: this._retryAttempt, finalError: errMsg });
          }
          success = false;
          terminalError = `LLM error: ${errMsg}`;
          emitEvent({ type: 'error', id: requestId, message: terminalError });
        } else if (this._retryAttempt > 0) {
          emitEvent({ type: 'auto_retry_end', success: true, attempt: this._retryAttempt });
        }

        this._retryAttempt = 0;
        this.stats.incrementTurns();
        break;
      }

    } catch (error: unknown) {
      this._retryAttempt = 0;
      const isAbort = (error as Error)?.name === 'AbortError';
      success = false;
      terminalError = isAbort ? 'Aborted' : `LLM error: ${extractErrorMessage(error)}`;
      if (!isAbort) emitEvent({ type: 'error', id: requestId, message: terminalError });
    } finally {
      this.abortController = null;
      emitEvent({ type: 'agent_end', id: requestId, success, ...(terminalError ? { error: terminalError } : {}) });
      this.activeRequestId = undefined;
    }
  }


  // ── Loop event → RPC event bridge ─────────────────────────────────────────

  private handleLoopEvent(event: AgentLoopEvent): void {
    switch (event.type) {

      case 'agent_start':
        break;

      case 'turn_start':
        emitEvent({ type: 'turn_start', id: this.activeRequestId });
        break;

      case 'message_update':
        // Stream text and thinking deltas.
        if (event.event.type === 'text_delta') {
          emitEvent({ type: 'text_delta', delta: event.event.delta });
        } else if (event.event.type === 'thinking_delta') {
          emitEvent({ type: 'thinking_delta', delta: event.event.delta });
        }
        break;

      case 'tool_execution_start':
        emitEvent({
          type:  'tool_call',
          name:  event.toolName,
          input: JSON.stringify(event.args),
        });
        break;

      case 'tool_execution_update':
        // Live streaming delta from run_command.
        emitEvent({
          type:         'tool_update',
          tool_call_id: event.toolCallId,
          delta:        event.partialResult.content.map(c => c.text).join(''),
        });
        break;

      case 'tool_execution_end': {
        const output = event.result.content.map(c => c.text).join('\n');
        emitEvent({ type: 'tool_result', name: event.toolName, output });
        break;
      }

      case 'turn_end':
        emitEvent({ type: 'turn_end', id: this.activeRequestId });
        break;

      case 'agent_end':
        break;
    }
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function extractErrorMessage(error: unknown): string {
  let msg = (error instanceof Error) ? error.message : String(error);
  try {
    const m = msg.match(/\{.*\}/s);
    if (m) {
      const p = JSON.parse(m[0]);
      const inner = p?.error?.message ?? p?.message;
      if (typeof inner === 'string') msg = inner.split('.')[0].trim();
    }
  } catch { /* keep original */ }
  return msg.replace(/\r?\n/g, ' ').slice(0, 200);
}
