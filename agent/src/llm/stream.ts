/**
 * LLM streaming layer — multi-provider router.
 *
 * `streamLLM` is the single entry point. It reads the active provider config
 * and dispatches to the appropriate API-specific streamer:
 *
 *   openai-completions    → streamOpenRouter (handles OpenRouter, OpenAI, DeepSeek, Groq, Mistral…)
 *   anthropic-messages    → streamAnthropic  (native Anthropic Messages API)
 *   google-generative-ai  → streamGoogle     (Google Gemini API)
 */

import type {
  AgentContext,
  AssistantMessageEvent,
} from './types.js';
import { inferProvider, resolveApiKey, stripProviderPrefix } from './provider.js';
import { streamAnthropic } from './stream-anthropic.js';
import { streamGoogle } from './stream-google.js';
import { streamLLM as streamLLMVercel } from './stream-vercel.js';

// ── Provider-aware dispatch ───────────────────────────────────────────────────

/**
 * Stream an LLM response using the configured provider.
 * This is the single call site used by agent-loop.ts.
 */
export async function* streamLLM(
  model: string,
  context: AgentContext,
  options: {
    temperature: number;
    signal?: AbortSignal;
  },
): AsyncIterable<AssistantMessageEvent> {
  const provider = inferProvider(model);
  const apiKey = resolveApiKey(provider);

  console.error(`[stream] provider=${provider.id} api=${provider.apiType} model=${model}`);

  switch (provider.apiType) {
    case 'anthropic-messages':
      yield* streamAnthropic(stripProviderPrefix(model), context, {
        temperature: options.temperature,
        baseUrl: provider.baseUrl,
        apiKey,
        signal: options.signal,
      });
      return;

    case 'google-generative-ai':
      yield* streamGoogle(stripProviderPrefix(model), context, {
        temperature: options.temperature,
        baseUrl: provider.baseUrl,
        apiKey,
        signal: options.signal,
      });
      return;

    case 'openai-completions':
    default:
      yield* streamLLMVercel(model, context, {
        temperature: options.temperature,
        signal: options.signal,
      });
      return;
  }
}

// ── Internal usage type ───────────────────────────────────────────────────────
interface OROUsage {
  prompt_tokens?:                   number;
  completion_tokens?:               number;
  prompt_cache_hit_tokens?:         number;
  cache_creation_input_tokens?:     number;
  prompt_cache_miss_tokens?:        number;
}