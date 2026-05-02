/**
 * LLM client — pi-style agent loop over direct OpenRouter SSE.
 *
 * No Vercel AI SDK. Uses:
 *   - stream.ts   → direct fetch to OpenRouter /v1/chat/completions
 *   - agent-loop.ts → pi-identical while-loop with tool execution hooks
 */

import { emitEvent, SessionStatsData } from '../rpc.js';
import { retry }                        from '../utils/retry.js';
import { Stats }                        from './stats.js';
import { runAgentLoop }                 from './agent-loop.js';
import { buildSystemPrompt }            from '../system-prompt.js';
import { tools }                        from '../tools/index.js';
import type { AgentLoopEvent, Message, AssistantMessage } from './types.js';

// ── Config ────────────────────────────────────────────────────────────────────

export let MODEL_NAME = process.env.MODEL_NAME ?? 'minimax/minimax-m2.5:free';
export const MODEL_LIMIT = 200_000;
export const MODEL_TEMP  = 0.3;

let _activeModel = MODEL_NAME;

export function setModel(model: string): void {
  _activeModel = model;
}

// ── Client ────────────────────────────────────────────────────────────────────

export class LLMClient {
  // Plain array — no History wrapper class. Pi stores messages directly on state.messages.
  // Using a wrapper class caused Bun runtime issues where .push() was undefined.
  private messages: Message[] = [];
  private readonly stats           = new Stats();
  private abortController: AbortController | null = null;
  private cachedSystemPrompt: string | null = null;

  private getSystemPrompt(): string {
    if (!this.cachedSystemPrompt) {
      this.cachedSystemPrompt = buildSystemPrompt();
    }
    return this.cachedSystemPrompt;
  }

  getSessionStats() { return this.stats.get(); }

  getSessionStatsResponse(modelLimit: number): SessionStatsData {
    const s    = this.stats.get();
    const used = s.tokens.input + s.tokens.output;
    return {
      tokens: {
        input:       s.tokens.input,
        output:      s.tokens.output,
        cache_read:  s.tokens.cache_read,
        cache_write: s.tokens.cache_write,
        total:       s.tokens.total,
      },
      context_usage: {
        tokens:  used,
        limit:   modelLimit,
        percent: parseFloat(((used / modelLimit) * 100).toFixed(1)),
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
            model:        _activeModel,
            temperature:  MODEL_TEMP,
            toolExecution: 'parallel',
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

        // Surface error/abort from stopReason — pi pattern.
        const lastAssistant = [...newMessages]
          .reverse()
          .find((m): m is AssistantMessage => m.role === 'assistant');

        if (lastAssistant?.stopReason === 'error') {
          emitEvent({ type: 'error', message: `LLM error: ${lastAssistant.errorMessage ?? 'unknown'}` });
        }

        this.stats.incrementTurns();

      }, this.abortController!.signal);

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
    const m = msg.match(/\\{.*\\}/s);
    if (m) {
      const p = JSON.parse(m[0]);
      const inner = p?.error?.message ?? p?.message;
      if (typeof inner === 'string') msg = inner.split('.')[0].trim();
    }
  } catch { /* keep original */ }
  return msg.replace(/\\r?\\n/g, ' ').slice(0, 200);
}
