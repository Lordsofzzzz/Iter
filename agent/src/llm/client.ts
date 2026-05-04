/**
 * LLM client — pi-style agent loop over direct OpenRouter SSE.
 *
 * No Vercel AI SDK. Uses:
 *   - stream.ts   → direct fetch to OpenRouter /v1/chat/completions
 *   - agent-loop.ts → pi-identical while-loop with tool execution hooks
 */

import { emitEvent, SessionStatsData } from '../rpc.js';
import { retry, isRetryEnabled, getRetryConfig } from '../config.js';
import { Stats } from './stats.js';
import { runAgentLoop }                 from './agent-loop.js';
import { buildSystemPrompt }            from '../system-prompt.js';
import { tools }                        from '../tools/index.js';
import type { AgentLoopEvent, Message, AssistantMessage } from './types.js';
import { transformContext } from './context.js';

// ── Config ────────────────────────────────────────────────────────────────────

export let MODEL_NAME = process.env.MODEL_NAME ?? 'minimax/minimax-m2.5:free';
export const MODEL_TEMP = 0.3;

const FALLBACK_LIMIT = 128_000;

// model_id → context window size, populated at startup from OpenRouter
const _contextWindowMap = new Map<string, number>();
let _activeModel = MODEL_NAME;
let _activeModelLimit = FALLBACK_LIMIT;

export function getModelLimit(): number {
  return _activeModelLimit;
}

export function setModel(model: string): void {
  _activeModel = model;
  _activeModelLimit = _contextWindowMap.get(model) ?? FALLBACK_LIMIT;
}

const MODELS_DEV_URL = 'https://models.dev/api.json';
const MAX_SANE_CONTEXT = 1_000_000; // cap against bad data (models.dev issue #2531)

/** Fetch model list from models.dev and populate context window map. */
export async function fetchModelLimits(): Promise<Array<{ id: string; name: string }>> {
  console.error('[models] fetching', MODELS_DEV_URL);
  try {
    const res = await fetch(MODELS_DEV_URL, {
      headers: { 'Accept': 'application/json' },
      signal: AbortSignal.timeout(8_000),
    });
    if (!res.ok) return [];

    // models.dev/api.json structure: { "provider": { models: { "model-id": { name, tool_call, limit: { context } } } } }
    const data = await res.json() as Record<string, {
      models?: Record<string, {
        name?:      string;
        tool_call?: boolean;
        limit?:     { context?: number; output?: number };
      }>;
    }>;

    const models: Array<{ id: string; name: string }> = [];
    for (const [_provider, providerData] of Object.entries(data)) {
      if (!providerData?.models) continue;
      for (const [id, info] of Object.entries(providerData.models)) {
        if (!info || typeof info !== 'object') continue;
        // Only include tool-capable models
        if (!info.tool_call) continue;
        const ctx = info.limit?.context;
        if (ctx && ctx > 0) {
          _contextWindowMap.set(id, Math.min(ctx, MAX_SANE_CONTEXT));
        }
        models.push({ id, name: info.name ?? id });
      }
    }

    _activeModelLimit = _contextWindowMap.get(_activeModel) ?? FALLBACK_LIMIT;
    console.error('[models] loaded', models.length, 'tool-capable models');
    return models;
  } catch (e) {
    console.error('[models] fetch error:', e);
    return [];
  }
}

// ── Client ────────────────────────────────────────────────────────────────────

export class LLMClient {
  // Plain array — no History wrapper class. Pi stores messages directly on state.messages.
  // Using a wrapper class caused Bun runtime issues where .push() was undefined.
  private messages: Message[] = [];
  private readonly stats           = new Stats();
  private abortController: AbortController | null = null;
  private cachedSystemPrompt: string | null = null;
  // Retry counter that resets after each successful LLM call (pi-mono pattern)
  private retryCount = 0;
  private get maxRetries() { return getRetryConfig().maxRetries; }

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

  async streamResponse(userMessage: string, model?: string): Promise<void> {
    if (model) _activeModel = model;

    this.abortController = new AbortController();

    try {
      await retry(async () => {
        // Snapshot history for this run.
        const contextMessages = [...this.messages];

        const newMessages = await runAgentLoop(
          userMessage,
          {
            systemPrompt: this.getSystemPrompt(),
            messages:     contextMessages,
            tools,
          },
          {
            model:          _activeModel,
            temperature:    MODEL_TEMP,
            toolExecution:  'parallel',
            transformContext: async (msgs) => transformContext(msgs),
          },
          (event: AgentLoopEvent) => this.handleLoopEvent(event),
          this.abortController!.signal,
        );

        // Persist all new messages — runAgentLoop always returns normally now (pi pattern).
        for (const msg of newMessages) {
          this.messages.push(msg);
        }

        // Sum token stats from ALL assistant messages.
        for (const msg of newMessages) {
          if (msg.role === 'assistant' && (msg as AssistantMessage).usage) {
            const u = (msg as AssistantMessage).usage;
            this.stats.addTokens(u.input, u.output, u.cacheRead, u.cacheWrite);
          }
        }

        // Check for retryable errors — re-throw to trigger retry with backoff.
        const lastAssistant = [...newMessages]
          .reverse()
          .find((m): m is AssistantMessage => m.role === 'assistant');

        if (lastAssistant?.stopReason === 'error') {
          const msg = lastAssistant.errorMessage ?? 'unknown';
          // Check if retryable (429 or server error) — re-throw to trigger backoff.
          if (msg.includes('429') || /5\d{2}/.test(msg) || /server\s*error/i.test(msg)) {
            this.retryCount++;
            const err = new Error(msg);
            (err as any).statusCode = msg.includes('429') ? 429 : 500;
            throw err;
          }
          emitEvent({ type: 'error', message: `LLM error: ${msg}` });
        }

        // Reset retry counter on successful response (pi-mono pattern)
        this.retryCount = 0;
        this.stats.incrementTurns();

      }, this.abortController!.signal, this.maxRetries - this.retryCount);

    } catch (error: unknown) {
      const isAbort = (error as Error)?.name === 'AbortError';
      if (!isAbort) {
        emitEvent({ type: 'error', message: `LLM error: ${extractErrorMessage(error)}` });
      }
    } finally {
      this.abortController = null;
    }
  }

  // ── Loop event → RPC event bridge ─────────────────────────────────────────

  private handleLoopEvent(event: AgentLoopEvent): void {
    switch (event.type) {

      case 'agent_start':
        break;

      case 'turn_start':
        emitEvent({ type: 'turn_start' });
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
        break;

      case 'agent_end':
        emitEvent({ type: 'turn_end' });
        emitEvent({ type: 'agent_end' });
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