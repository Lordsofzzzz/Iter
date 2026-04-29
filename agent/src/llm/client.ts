import { streamText } from 'ai';
import { createOpenRouter } from '@openrouter/ai-sdk-provider';
import { emitEvent, SessionStatsData } from '../rpc.js';
import { retry } from '../utils/retry.js';
import { History } from './history.js';
import { Stats } from './stats.js';

const openrouter = createOpenRouter({});

export const MODEL_NAME  = process.env.MODEL_NAME ?? 'google/gemma-4-31b-it:free';
export const MODEL_LIMIT = 200_000;
export const MODEL_TEMP  = 0.3;

const COST_PER_1M: [number, number] = [0.0, 0.0];

export class LLMClient {
  private history = new History();
  private stats = new Stats();
  private abortController: AbortController | null = null;

  getSessionStats(): ReturnType<Stats['get']> {
    return this.stats.get();
  }

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

  abort(): void {
    this.abortController?.abort();
    this.abortController = null;
  }

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

        for await (const chunk of result.textStream) {
          assistantText += chunk;
          emitEvent({ type: 'text_delta', delta: chunk });
        }

        const usage = await result.usage;
        const usageAny = usage as any;

        const input  = usageAny.promptTokens     ?? usageAny.inputTokens  ?? 0;
        const output = usageAny.completionTokens ?? usageAny.outputTokens ?? 0;

        const meta          = usageAny.providerMetadata ?? {};
        const anthropicMeta = meta?.anthropic ?? {};
        const openaiMeta    = meta?.openai ?? {};

        const cache_read  = (anthropicMeta.cacheReadInputTokens ?? openaiMeta.cachedTokens ?? 0) as number;
        const cache_write = (anthropicMeta.cacheCreationInputTokens ?? 0) as number;

        this.stats.addTokens(input, output, cache_read, cache_write);

        const turnCost = ((input / 1_000_000) * COST_PER_1M[0])
                      + ((output / 1_000_000) * COST_PER_1M[1]);
        this.stats.addCost(turnCost);
        this.stats.incrementTurns();

        if (assistantText) {
          this.history.pushAssistant(assistantText);
        }
      }, this.abortController!.signal);

    } catch (error: unknown) {
      this.history.pop();
      const isAbort = error instanceof Error && error.name === 'AbortError';
      if (!isAbort) {
        const raw = error instanceof Error ? error.message : String(error);
        let message = raw;
        try {
          const jsonMatch = raw.match(/\{.*\}/s);
          if (jsonMatch) {
            const parsed = JSON.parse(jsonMatch[0]);
            const inner = parsed?.error?.message ?? parsed?.message;
            if (inner && typeof inner === 'string') {
              message = inner.split('.')[0].trim();
            }
          }
        } catch { /* keep raw */ }
        message = message.replace(/\r?\n/g, ' ').slice(0, 200);
        emitEvent({ type: 'error', message: `LLM error: ${message}` });
      }
    } finally {
      this.abortController = null;
    }

    emitEvent({ type: 'turn_end' });
    emitEvent({ type: 'agent_end' });
  }
}

export function clearHistory(client: LLMClient): void {
  client.getSessionStats();
}
