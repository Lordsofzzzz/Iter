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

import OpenAI from 'openai';
import type {
  AgentContext,
  AgentTool,
  AssistantMessage,
  AssistantMessageEvent,
  Message,
  TextContent,
  ThinkingContent,
  ToolCall,
  ToolResultMessage,
  Usage,
} from './types.js';
import { logToFile } from '../utils/logger.js';
import { getActiveProvider, resolveApiKey, inferProvider, stripProviderPrefix } from './provider.js';
import { getConfig } from '../config.js';
import { parseProviderError, isGenericRetryable } from './provider-error.js';
import { streamAnthropic } from './stream-anthropic.js';
import { streamGoogle } from './stream-google.js';

// ── Constants ─────────────────────────────────────────────────────────────────

const OPENROUTER_BASE = 'https://openrouter.ai/api/v1';

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
      yield* streamOpenRouter(model, context, {
        temperature: options.temperature,
        apiKey,
        signal: options.signal,
        baseUrl: provider.baseUrl,
      });
      return;
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function findLastIndex<T>(arr: T[], predicate: (val: T) => boolean): number {
  for (let i = arr.length - 1; i >= 0; i--) {
    if (predicate(arr[i])) return i;
  }
  return -1;
}

// ── XML tool-call parser (MiniMax / models that embed tools in text) ──────────

interface ParsedXmlToolCall {
  id:        string;
  name:      string;
  arguments: Record<string, unknown>;
}

/**
 * Some models (e.g. MiniMax) emit tool calls as XML embedded in delta.content
 * instead of delta.tool_calls.  Two formats observed:
 *
 *   <minimax:tool_call>
 *     <invoke name="read_file"><parameter name="path">…</parameter></invoke>
 *   </minimax:tool_call>
 *
 *   <tool_call>{"name":"read_file","arguments":{…}}</tool_call>
 *
 * Returns { textBefore, calls, textAfter } so callers can strip the XML
 * and still surface any surrounding prose.
 */
function extractXmlToolCalls(raw: string): {
  textBefore: string;
  calls:      ParsedXmlToolCall[];
  textAfter:  string;
} {
  const calls: ParsedXmlToolCall[] = [];

  // ── Format 1: <minimax:tool_call>…</minimax:tool_call> ────────────────────
  const minimaxRe = /<minimax:tool_call>([\s\S]*?)<\/minimax:tool_call>/g;
  let match: RegExpExecArray | null;
  let lastIndex = 0;
  let textBefore = '';
  let textAfter  = raw;

  const segments: string[] = [];
  let consumed = raw;

  // Replace all minimax blocks.
  consumed = consumed.replace(minimaxRe, (_, inner) => {
    // Each <invoke name="…"> inside.
    const invokeRe = /<invoke\s+name="([^"]+)">([\s\S]*?)<\/invoke>/g;
    let inv: RegExpExecArray | null;
    while ((inv = invokeRe.exec(inner)) !== null) {
      const toolName = inv[1];
      const paramsRaw = inv[2];
      const args: Record<string, unknown> = {};
      const paramRe = /<parameter\s+name="([^"]+)">([\s\S]*?)<\/parameter>/g;
      let p: RegExpExecArray | null;
      while ((p = paramRe.exec(paramsRaw)) !== null) {
        const key = p[1];
        const val = p[2].trim();
        // Try to parse as JSON, fall back to string.
        try { args[key] = JSON.parse(val); } catch { args[key] = val; }
      }
      calls.push({
        id:        `xml-${Date.now()}-${calls.length}`,
        name:      toolName,
        arguments: args,
      });
    }
    return ''; // strip from text
  });

  // ── Format 2: <tool_call>{…}</tool_call> ─────────────────────────────────
  consumed = consumed.replace(/<tool_call>([\s\S]*?)<\/tool_call>/g, (_, inner) => {
    try {
      const parsed = JSON.parse(inner.trim());
      calls.push({
        id:        `xml-${Date.now()}-${calls.length}`,
        name:      parsed.name ?? parsed.tool_name ?? '',
        arguments: parsed.arguments ?? parsed.params ?? {},
      });
    } catch { /* malformed — ignore */ }
    return '';
  });

  // Strip any leftover XML scaffolding tags that leak through.
  consumed = consumed
    .replace(/<\/?minimax:tool_call>/g, '')
    .replace(/<invoke[^>]*>/g, '')
    .replace(/<\/invoke>/g, '')
    .replace(/<parameter[^>]*>/g, '')
    .replace(/<\/parameter>/g, '');
  // NOTE: do NOT .trim() here — delta chunks may be single spaces and trimming drops them

  return { textBefore: consumed, calls, textAfter: '' };
}

// ── Wire format types ─────────────────────────────────────────────────────────

interface ORDelta {
  role?:              string;
  content?:           string | null;
  reasoning?:         string | null;
  tool_calls?:        ORToolCallDelta[];
  /** OpenRouter reasoning_details — present on models with native reasoning support (e.g. MiniMax). */
  reasoning_details?: unknown[];
}

interface ORToolCallDelta {
  index:    number;
  id?:      string;
  type?:    string;
  function?: { name?: string; arguments?: string };
}

// ── Message converter ─────────────────────────────────────────────────────────

/** Convert our internal Message[] to OpenRouter wire format. */
export function toOpenRouterMessages(messages: Message[]): unknown[] {
  const result: unknown[] = [];

  for (const msg of messages) {
    if (msg.role === 'user') {
      result.push({ role: 'user', content: msg.content });
      continue;
    }

    if (msg.role === 'assistant') {
      const parts: unknown[] = [];
      const toolCalls: unknown[] = [];

      for (const part of msg.content) {
        if (part.type === 'text') {
          parts.push({ type: 'text', text: part.text });
        } else if (part.type === 'thinking') {
          // Do NOT send thinking back to the model — it poisons context.
          // MiniMax embeds thinking in delta.reasoning, not as a user-visible message.
          // Sending it as text causes the model to see its own internal monologue as
          // part of the conversation and get confused / loop.
          continue;
        } else if (part.type === 'toolCall') {
          toolCalls.push({
            id:       part.id,
            type:     'function',
            function: { name: part.name, arguments: JSON.stringify(part.arguments) },
          });
        }
      }

      const orMsg: Record<string, unknown> = { role: 'assistant' };
      if (parts.length > 0) {
        orMsg.content = parts.length === 1 && (parts[0] as any).type === 'text'
          ? (parts[0] as any).text
          : parts;
      }
      if (toolCalls.length > 0) orMsg.tool_calls = toolCalls;

      // Pass reasoning_details back unmodified — OpenRouter requires this for MiniMax
      // to maintain reasoning context across turns. Without this, model quality degrades.
      // See: https://openrouter.ai/docs/use-cases/reasoning-tokens#preserving-reasoning-blocks
      const assistantMsg = msg as AssistantMessage;
      if (assistantMsg.reasoningDetails && assistantMsg.reasoningDetails.length > 0) {
        orMsg.reasoning_details = assistantMsg.reasoningDetails;
      }

      result.push(orMsg);
      continue;
    }

    if (msg.role === 'toolResult') {
      result.push({
        role:         'tool',
        tool_call_id: msg.toolCallId,
        content:      msg.content.map(c => c.text).join('\n'),
      });
    }
  }

  return result;
}

/** Convert AgentTool[] to OpenRouter tool definitions. */
function toOpenRouterTools(tools: AgentTool[]): unknown[] {
  return tools.map(t => ({
    type:     'function',
    function: {
      name:        t.name,
      description: t.description,
      parameters:  t.parameters,
    },
  }));
}

// ── Stream function ───────────────────────────────────────────────────────────

/** Empty usage sentinel. */
function emptyUsage(): Usage {
  return { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0 };
}

/** Build a blank partial AssistantMessage. */
function blankPartial(): AssistantMessage {
  return {
    role:       'assistant',
    content:    [],
    usage:      emptyUsage(),
    stopReason: 'stop',
    timestamp:  Date.now(),
  };
}

/**
 * Stream an assistant response from OpenRouter.
 *
 * Returns an async iterable of AssistantMessageEvent — identical protocol
 * to pi-ai's streamSimple. The caller (agent loop) iterates these events.
 */
export async function* streamOpenRouter(
  model: string,
  context: AgentContext,
  options: {
    temperature: number;
    apiKey?: string;
    signal?: AbortSignal;
    baseUrl?: string;
    apiType?: 'openai-completions';
  },
): AsyncIterable<AssistantMessageEvent> {
  const apiKey  = options.apiKey ?? process.env.OPENROUTER_API_KEY ?? '';
  const baseUrl = options.baseUrl ?? OPENROUTER_BASE;
  const cfg     = getConfig();

  console.error('[DEBUG] API Key present:', !!apiKey, 'Key prefix:', apiKey.substring(0, 20));

  // ── OpenAI SDK client — built-in retry (maxRetries=2) + Retry-After handling ──
  const client = new OpenAI({
    apiKey,
    baseURL:                baseUrl,
    dangerouslyAllowBrowser: false,
    defaultHeaders: {
      'HTTP-Referer': 'https://github.com/iter-coding-agent',
      'X-Title':      'Iter',
    },
    maxRetries: 2,
    timeout:    cfg.timeoutMs,
  });

  // ── Build request params ─────────────────────────────────────────────────────
  const messages = toOpenRouterMessages(context.messages);
  if (context.systemPrompt) {
    (messages as unknown[]).unshift({ role: 'system', content: context.systemPrompt });
  }

  const params: Record<string, unknown> = {
    model,
    temperature:    options.temperature,
    stream:         true,
    stream_options: { include_usage: true },
    messages,
  };

  if (context.tools && context.tools.length > 0) {
    params.tools       = toOpenRouterTools(context.tools);
    params.tool_choice = 'auto';
  }

  const isThinkingModel =
    model.includes(':thinking') ||
    model.includes('deepseek-r') ||
    model.includes('qwq')        ||
    model.includes('r1')         ||
    model.includes('reasoning');

  if (isThinkingModel) {
    params.reasoning = { effort: 'medium' };
  }

  // ── Stream state ─────────────────────────────────────────────────────────────
  const partial = blankPartial();
  let emittedStart = false;

  let textBuffer        = '';
  let thinkingBuffer    = '';
  let thinkingClosed    = false;
  let reasoningOverflow = '';

  const tcAccum: Map<number, {
    id:         string;
    name:       string;
    argsRaw:    string;
    contentIdx: number;
    finalized?: boolean;
  }> = new Map();

  let finishReason: string | null = null;
  let usageFromChunk: OROUsage | null = null;
  const reasoningDetailsAcc: unknown[] = [];

  // ── Inline helper: emit XML-embedded tool calls ───────────────────────────
  const emitAsXml = async function*(raw: string) {
    const { textBefore, calls } = extractXmlToolCalls(raw);
    if (textBefore.trim()) {
      if (partial.content.filter(c => c.type === 'text').length === 0) {
        const idx = partial.content.length;
        partial.content.push({ type: 'text', text: '' });
        yield { type: 'text_start' as const, contentIndex: idx, partial: { ...partial } };
      }
      const textIdx = findLastIndex(partial.content, c => c.type === 'text');
      textBuffer += textBefore;
      (partial.content[textIdx] as TextContent).text = textBuffer;
      yield { type: 'text_delta' as const, contentIndex: textIdx, delta: textBefore, partial: { ...partial } };
    }
    for (const xmlTc of calls) {
      const contentIdx = partial.content.length;
      partial.content.push({ type: 'toolCall', id: '', name: '', arguments: {} });
      yield { type: 'toolcall_start' as const, contentIndex: contentIdx, partial: { ...partial } };
      const toolCall: ToolCall = { type: 'toolCall', id: xmlTc.id, name: xmlTc.name, arguments: xmlTc.arguments };
      partial.content[contentIdx] = toolCall;
      tcAccum.set(10000 + contentIdx, { id: xmlTc.id, name: xmlTc.name, argsRaw: JSON.stringify(xmlTc.arguments), contentIdx, finalized: true });
      yield { type: 'toolcall_end' as const, contentIndex: contentIdx, toolCall, partial: { ...partial } };
    }
  };

  // ── SDK stream ───────────────────────────────────────────────────────────────
  try {
    console.error('[DEBUG] Starting OpenAI SDK stream');
    // Cast needed: SDK types don't expose custom fields (reasoning, stream_options, etc.)
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const response = await client.chat.completions.create(
      params as any,
      { signal: options.signal },
    ) as unknown as AsyncIterable<{ choices?: Array<{ delta?: ORDelta; finish_reason?: string }>; usage?: OROUsage }>;

    for await (const chunk of response) {
      if (chunk.usage) usageFromChunk = chunk.usage as OROUsage;

      const choice = Array.isArray(chunk.choices) ? chunk.choices[0] : undefined;
      if (!choice) continue;

      if (choice.finish_reason) finishReason = choice.finish_reason;

      const delta = choice.delta as ORDelta & { reasoning_details?: unknown[] };

      // Capture reasoning_details for passback (OpenRouter MiniMax requirement).
      if (delta.reasoning_details && Array.isArray(delta.reasoning_details)) {
        for (const rd of delta.reasoning_details) reasoningDetailsAcc.push(rd);
      }

      // Emit start on first delta.
      if (!emittedStart) {
        emittedStart = true;
        yield { type: 'start', partial: { ...partial } };
      }

      // ── Reasoning / thinking delta ───────────────────────────────────────
      if (delta.reasoning) {
        const raw = reasoningOverflow + delta.reasoning;
        reasoningOverflow = '';

        // Detect XML tool-call boundary (MiniMax quirk: tool calls arrive via delta.reasoning).
        const TOOL_MARKERS = ['<minimax:tool_call>', '<invoke name=', '<tool_call>'];
        let boundaryIdx = -1;
        if (!thinkingClosed) {
          for (const marker of TOOL_MARKERS) {
            const idx = raw.indexOf(marker);
            if (idx !== -1 && (boundaryIdx === -1 || idx < boundaryIdx)) boundaryIdx = idx;
          }
        }

        if (!thinkingClosed && boundaryIdx === -1) {
          // Pure thinking — hold back possible partial marker at end.
          let safeUpto = raw.length;
          for (const marker of TOOL_MARKERS) {
            for (let len = Math.min(marker.length - 1, raw.length); len >= 1; len--) {
              if (raw.endsWith(marker.slice(0, len))) {
                safeUpto = Math.min(safeUpto, raw.length - len);
                break;
              }
            }
          }
          const thinkPart = raw.slice(0, safeUpto);
          reasoningOverflow = raw.slice(safeUpto);
          if (thinkPart) {
            if (thinkingBuffer === '') {
              const idx = partial.content.length;
              partial.content.push({ type: 'thinking', thinking: '' });
              yield { type: 'thinking_start', contentIndex: idx, partial: { ...partial } };
            }
            const ti = findLastIndex(partial.content, c => c.type === 'thinking');
            thinkingBuffer += thinkPart;
            (partial.content[ti] as ThinkingContent).thinking = thinkingBuffer;
            yield { type: 'thinking_delta', contentIndex: ti, delta: thinkPart, partial: { ...partial } };
          }
        } else if (!thinkingClosed && boundaryIdx !== -1) {
          thinkingClosed = true;
          const thinkPart = raw.slice(0, boundaryIdx).replace(/<\/thinking>\s*$/, '').trimEnd();
          const xmlPart   = raw.slice(boundaryIdx);
          if (thinkPart) {
            if (thinkingBuffer === '') {
              const idx = partial.content.length;
              partial.content.push({ type: 'thinking', thinking: '' });
              yield { type: 'thinking_start', contentIndex: idx, partial: { ...partial } };
            }
            const ti = findLastIndex(partial.content, c => c.type === 'thinking');
            thinkingBuffer += thinkPart;
            (partial.content[ti] as ThinkingContent).thinking = thinkingBuffer;
            yield { type: 'thinking_delta', contentIndex: ti, delta: thinkPart, partial: { ...partial } };
          }
          if (xmlPart.trim()) yield* emitAsXml(xmlPart);
        } else {
          if (raw.trim()) yield* emitAsXml(raw);
        }
      }

      // ── Text delta ────────────────────────────────────────────────────────
      const rawContent = delta.content;
      const contentStr: string | null | undefined =
        Array.isArray(rawContent)
          ? (rawContent as Array<{type?: string; text?: string}>)
              .filter(p => p.type === 'text' || p.text !== undefined)
              .map(p => p.text ?? '')
              .join('')
          : rawContent;

      if (contentStr) {
        const { textBefore, calls } = extractXmlToolCalls(contentStr);
        if (textBefore) {
          if (partial.content.filter(c => c.type === 'text').length === 0) {
            const idx = partial.content.length;
            partial.content.push({ type: 'text', text: '' });
            yield { type: 'text_start', contentIndex: idx, partial: { ...partial } };
          }
          const ti = findLastIndex(partial.content, c => c.type === 'text');
          textBuffer += textBefore;
          (partial.content[ti] as TextContent).text = textBuffer;
          yield { type: 'text_delta', contentIndex: ti, delta: textBefore, partial: { ...partial } };
        }
        for (const xmlTc of calls) {
          const contentIdx = partial.content.length;
          partial.content.push({ type: 'toolCall', id: '', name: '', arguments: {} });
          yield { type: 'toolcall_start', contentIndex: contentIdx, partial: { ...partial } };
          const toolCall: ToolCall = { type: 'toolCall', id: xmlTc.id, name: xmlTc.name, arguments: xmlTc.arguments };
          partial.content[contentIdx] = toolCall;
          tcAccum.set(10000 + contentIdx, { id: xmlTc.id, name: xmlTc.name, argsRaw: JSON.stringify(xmlTc.arguments), contentIdx, finalized: true });
          yield { type: 'toolcall_end', contentIndex: contentIdx, toolCall, partial: { ...partial } };
        }
      }

      // ── Tool call deltas ──────────────────────────────────────────────────
      if (delta.tool_calls) {
        for (const tc of delta.tool_calls) {
          if (!tcAccum.has(tc.index)) {
            const contentIdx = partial.content.length;
            partial.content.push({ type: 'toolCall', id: '', name: '', arguments: {} });
            tcAccum.set(tc.index, { id: '', name: '', argsRaw: '', contentIdx });
            yield { type: 'toolcall_start', contentIndex: contentIdx, partial: { ...partial } };
          }
          const acc = tcAccum.get(tc.index)!;
          if (tc.id)                  acc.id      += tc.id;
          if (tc.function?.name)      acc.name    += tc.function.name;
          if (tc.function?.arguments) {
            acc.argsRaw += tc.function.arguments;
            yield { type: 'toolcall_delta', contentIndex: acc.contentIdx, delta: tc.function.arguments, partial: { ...partial } };
          }
        }
      }
    }
  } catch (err) {
    console.error('[DEBUG] SDK stream error:', err);
    const isAbort = (err as Error)?.name === 'AbortError' || options.signal?.aborted;
    partial.stopReason   = isAbort ? 'aborted' : 'error';
    partial.errorMessage = isAbort ? 'Aborted' : String((err as Error)?.message ?? err);
    yield { type: 'error', reason: partial.stopReason as 'aborted' | 'error', error: partial };
    return;
  }


  // ── Finalise thinking block ────────────────────────────────────────────────
  if (thinkingBuffer) {
    const idx = findLastIndex(partial.content, c => c.type === 'thinking');
    yield { type: 'thinking_end', contentIndex: idx, content: thinkingBuffer, partial: { ...partial } };
  }

  // ── Finalise text block ────────────────────────────────────────────────────
  if (textBuffer) {
    const idx = findLastIndex(partial.content, c => c.type === 'text');
    yield { type: 'text_end', contentIndex: idx, content: textBuffer, partial: { ...partial } };
  }

  // ── Finalise tool calls ────────────────────────────────────────────────────
  for (const [, acc] of tcAccum) {
    if (acc.finalized) continue; // already emitted via XML path
    let args: Record<string, unknown> = {};
    try { args = JSON.parse(acc.argsRaw || '{}'); } catch { /* keep empty */ }

    const toolCall: ToolCall = { type: 'toolCall', id: acc.id, name: acc.name, arguments: args };
    partial.content[acc.contentIdx] = toolCall;
    yield { type: 'toolcall_end', contentIndex: acc.contentIdx, toolCall, partial: { ...partial } };
  }

  // ── Usage ─────────────────────────────────────────────────────────────────
  if (usageFromChunk) {
    partial.usage = {
      input:       usageFromChunk.prompt_tokens ?? 0,
      output:      usageFromChunk.completion_tokens ?? 0,
      cacheRead:   usageFromChunk.prompt_cache_hit_tokens ?? 0,
      cacheWrite:  usageFromChunk.cache_creation_input_tokens ?? 0,
      totalTokens: (usageFromChunk.prompt_tokens ?? 0) + (usageFromChunk.completion_tokens ?? 0),
    };
  }

  // ── Stop reason ────────────────────────────────────────────────────────────
  // If XML tool calls were parsed from text, treat as toolUse even if model sent 'stop'.
  const hasXmlToolCalls = [...tcAccum.values()].some(a => a.finalized);

  // Attach accumulated reasoning_details so caller can pass them back on next turn.
  if (reasoningDetailsAcc.length > 0) {
    partial.reasoningDetails = reasoningDetailsAcc;
  }

  if (finishReason === 'tool_calls' || hasXmlToolCalls) {
    partial.stopReason = 'toolUse';
    yield { type: 'done', reason: 'toolUse', message: { ...partial } };
  } else if (finishReason === 'length') {
    partial.stopReason = 'length';
    yield { type: 'done', reason: 'length', message: { ...partial } };
  } else {
    partial.stopReason = 'stop';
    yield { type: 'done', reason: 'stop', message: { ...partial } };
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
