/**
 * LLM client using OpenRouter provider.
 *
 * Handles message history, token tracking, and streaming responses
 * from the language model.
 */

import { streamText, wrapLanguageModel, extractReasoningMiddleware } from 'ai';
import { createOpenRouter } from '@openrouter/ai-sdk-provider';
import { emitEvent, SessionStatsData } from '../rpc.js';
import { logToFile } from '../utils/logger.js';
import { retry } from '../utils/retry.js';
import { History } from './history.js';
import { Stats } from './stats.js';
import { tools } from '../tools/index.js';
import { buildSystemPrompt } from '../system-prompt.js';

// ============================================================================
// Configuration Constants
// ============================================================================

const openrouter = createOpenRouter({});

/** Default model to use (fallback to Gemma 4B free). */
export let MODEL_NAME = process.env.MODEL_NAME ?? 'minimax/minimax-m2.5:free';

/** Override the active model at runtime. */
export function setModel(model: string): void {
  MODULE_NAME_OVERRIDE = model;
}

// Internal mutable override (avoids re-exporting a `let` binding issues).
let MODULE_NAME_OVERRIDE: string = MODEL_NAME;

/** Context window size in tokens. */
export const MODEL_LIMIT = 200_000;

/** Temperature for generation (0.0 = deterministic, 1.0 = creative). */
export const MODEL_TEMP = 0.3;

/** Cost per 1M tokens [input, output]. Currently free. */
const COST_PER_MILLION: [number, number] = [0.0, 0.0];

// ============================================================================
// Client Implementation
// ============================================================================

/** LLM client managing conversation history and session stats. */
export class LLMClient {
  private history = new History();
  private stats = new Stats();
  private abortController: AbortController | null = null;

  /**
   * Returns current session statistics.
   */
  getSessionStats(): ReturnType<Stats['get']> {
    return this.stats.get();
  }

  /**
   * Returns session stats formatted for TUI response.
   */
  getSessionStatsResponse(modelLimit: number): SessionStatsData {
    const s = this.stats.get();
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

  /**
   * Aborts any in-progress streaming request.
   */
  abort(): void {
    this.abortController?.abort();
    this.abortController = null;
  }

  /**
   * Clears conversation history.
   */
  clearHistory(): void {
    this.history.clear();
    this.stats.reset();
  }

  /**
   * Sends a user message and streams the assistant response.
   *
   * Updates history, emits text deltas, tracks tokens/cost,
   * and handles errors gracefully.
   */
  async streamResponse(userMessage: string, model?: string): Promise<void> {
    if (model) MODULE_NAME_OVERRIDE = model;
    this.history.pushUser(userMessage);
    emitEvent({ type: 'turn_start' });

    let assistantText = '';
    this.abortController = new AbortController();

    try {
      await retry(async () => {
        assistantText = '';  // Reset on each attempt to avoid duplication on retry.
        
        const model = wrapLanguageModel({
          model: openrouter.chat(MODULE_NAME_OVERRIDE),
          middleware: extractReasoningMiddleware({ tagName: 'thinking' }),
        });

        const isThinkingModel = MODULE_NAME_OVERRIDE.includes(':thinking') ||
          MODULE_NAME_OVERRIDE.includes('deepseek-r') ||
          MODULE_NAME_OVERRIDE.includes('qwq') ||
          MODULE_NAME_OVERRIDE.includes('minimax') ||
          MODULE_NAME_OVERRIDE.includes('r1') ||
          MODULE_NAME_OVERRIDE.includes('reasoning');

        const result = await streamText({
          model,
          temperature: MODEL_TEMP,
          system:      buildSystemPrompt(),
          messages:    this.history.get(),
          tools,
          abortSignal: this.abortController!.signal,
          onError:     () => {},
          ...(isThinkingModel ? {
            providerOptions: {
              openrouter: { reasoning: { max_tokens: 8000 } },
            },
          } : {}),
        });

        // Stream text and reasoning deltas to TUI.
        for await (const chunk of result.fullStream) {
          logToFile(`[CHUNK] ${JSON.stringify(chunk)}`);
          switch (chunk.type) {
            case 'reasoning-delta': {
              const delta: string = (chunk as any).textDelta ?? (chunk as any).text ?? '';
              if (delta) emitEvent({ type: 'thinking_delta', delta });
              break;
            }
            case 'text-delta': {
              const delta: string = (chunk as any).textDelta ?? (chunk as any).text ?? '';
              if (delta) {
                assistantText += delta;
                emitEvent({ type: 'text_delta', delta });
              }
              break;
            }
          }
        }

        // Extract usage info (handles multiple provider formats).
        const usage = await result.usage;

        // AI SDK v4 uses inputTokens/outputTokens; fall back to v3 names for compatibility.
        const input  = (usage as any)?.inputTokens ?? usage?.promptTokens ?? 0;
        const output = (usage as any)?.outputTokens ?? usage?.completionTokens ?? 0;

        const meta = usage?.providerMetadata ?? {};
        const anthropicMeta = meta?.anthropic ?? {};
        const openaiMeta = meta?.openai ?? {};

        const cacheRead  = (anthropicMeta.cacheReadInputTokens ?? openaiMeta.cachedTokens ?? 0) as number;
        const cacheWrite = (anthropicMeta.cacheCreationInputTokens ?? 0) as number;

        // Update session stats.
        this.stats.addTokens(input, output, cacheRead, cacheWrite);

        const turnCost = ((input / 1_000_000) * COST_PER_MILLION[0])
                      + ((output / 1_000_000) * COST_PER_MILLION[1]);
        this.stats.addCost(turnCost);
        this.stats.incrementTurns();

        // Save assistant response to history.
        if (assistantText) {
          this.history.pushAssistant(assistantText);
        }
      }, this.abortController!.signal);

    } catch (error: unknown) {
      // Remove user message on failure (allows retry).
      this.history.pop();

      // Handle abort separately (not an error).
      const isAbort = error instanceof Error && error.name === 'AbortError';
      if (!isAbort) {
        const message = extractErrorMessage(error);
        emitEvent({ type: 'error', message: `LLM error: ${message}` });
      }
    } finally {
      this.abortController = null;
    }

    emitEvent({ type: 'turn_end' });
    emitEvent({ type: 'agent_end' });
  }
}

// ============================================================================
// Private Helpers
// ============================================================================

/**
 * Extracts a clean error message from various error formats.
 */
function extractErrorMessage(error: unknown): string {
  let message = error instanceof Error ? error.message : String(error);

  // Try to extract inner error from JSON response.
  try {
    const jsonMatch = message.match(/\{.*\}/s);
    if (jsonMatch) {
      const parsed = JSON.parse(jsonMatch[0]);
      const inner = parsed?.error?.message ?? parsed?.message;
      if (inner && typeof inner === 'string') {
        message = inner.split('.')[0].trim();
      }
    }
  } catch {
    // Keep original message.
  }

  // Normalize whitespace and truncate.
  return message.replace(/\r?\n/g, ' ').slice(0, 200);
}
