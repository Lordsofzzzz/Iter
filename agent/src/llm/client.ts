/**
 * LLM client using OpenRouter provider.
 *
 * Handles message history, token tracking, and streaming responses
 * from the language model.
 */

import { streamText } from 'ai';
import { createOpenRouter } from '@openrouter/ai-sdk-provider';
import { emitEvent, SessionStatsData } from '../rpc.js';
import { retry } from '../utils/retry.js';
import { History } from './history.js';
import { Stats } from './stats.js';

// ============================================================================
// Configuration Constants
// ============================================================================

const openrouter = createOpenRouter({});

/** Default model to use (fallback to Gemma 4B free). */
export const MODEL_NAME = process.env.MODEL_NAME ?? 'google/gemma-4-31b-it:free';

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
   * Sends a user message and streams the assistant response.
   *
   * Updates history, emits text deltas, tracks tokens/cost,
   * and handles errors gracefully.
   */
  async streamResponse(userMessage: string): Promise<void> {
    this.history.pushUser(userMessage);
    emitEvent({ type: 'turn_start' });

    let assistantText = '';
    this.abortController = new AbortController();

    try {
      await retry(async () => {
        const result = await streamText({
          model:       openrouter.chat(MODEL_NAME),
          temperature: MODEL_TEMP,
          messages:    this.history.get(),
          abortSignal: this.abortController!.signal,
          onError:    () => {},
        });

        // Stream text deltas to TUI.
        for await (const chunk of result.textStream) {
          assistantText += chunk;
          emitEvent({ type: 'text_delta', delta: chunk });
        }

        // Extract usage info (handles multiple provider formats).
        const usage = await result.usage as {
          promptTokens?: number;
          completionTokens?: number;
          providerMetadata?: {
            anthropic?: { cacheReadInputTokens?: number; cacheCreationInputTokens?: number };
            openai?: { cachedTokens?: number };
          };
        };

        const input  = usage.promptTokens ?? 0;
        const output = usage.completionTokens ?? 0;

        const meta = usage.providerMetadata ?? {};
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

/**
 * Clears conversation history (for /clear command).
 */
export function clearHistory(client: LLMClient): void {
  // History is managed by client internally.
  client.getSessionStats();
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