/**
 * OpenRouter streaming layer.
 *
 * Talks directly to OpenRouter's /api/v1/chat/completions endpoint
 * using SSE. No Vercel AI SDK. Returns an async iterable of
 * AssistantMessageEvent — same event protocol as pi-ai.
 */

import http from 'http';
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

// ── Constants ─────────────────────────────────────────────────────────────────

const OPENROUTER_BASE = 'https://openrouter.ai/api/v1';

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

interface ORChunk {
  choices?: Array<{
    delta:          ORDelta;
    finish_reason?: string | null;
  }>;
  usage?: {
    prompt_tokens?:      number;
    completion_tokens?:  number;
    prompt_cache_hit_tokens?: number;
    cache_creation_input_tokens?: number;
    prompt_cache_miss_tokens?: number;
  };
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
  model:   string,
  context: AgentContext,
  options: {
    temperature: number;
    apiKey?:     string;
    signal?:     AbortSignal;
  },
): AsyncIterable<AssistantMessageEvent> {
  const apiKey = options.apiKey ?? process.env.OPENROUTER_API_KEY ?? '';
  console.error('[DEBUG] API Key present:', !!apiKey, 'Key prefix:', apiKey.substring(0, 20));

  const body: Record<string, unknown> = {
    model,
    temperature: options.temperature,
    stream:      true,
    messages:    toOpenRouterMessages(context.messages),
  };

  if (context.systemPrompt) {
    (body.messages as unknown[]).unshift({ role: 'system', content: context.systemPrompt });
  }

  if (context.tools && context.tools.length > 0) {
    body.tools       = toOpenRouterTools(context.tools);
    body.tool_choice = 'auto';
  }

  // Reasoning: ask for extended thinking if model supports it.
  const isThinkingModel =
    model.includes(':thinking') ||
    model.includes('deepseek-r') ||
    model.includes('qwq')        ||
    model.includes('r1')         ||
    model.includes('reasoning');

  if (isThinkingModel) {
    body.reasoning = { effort: 'medium' };
  }

  let response: Response;
  console.error('[DEBUG] Starting fetch to OpenRouter');
  try {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 25000);
    if (options.signal) {
      options.signal.addEventListener('abort', () => controller.abort(), { once: true });
    }
    response = await fetch(`${OPENROUTER_BASE}/chat/completions`, {
      method:  'POST',
      headers: {
        'Content-Type':  'application/json',
        'Authorization': `Bearer ${apiKey}`,
        'HTTP-Referer':  'https://github.com/iter-coding-agent',
        'X-Title':       'Iter',
      },
      body:   JSON.stringify(body),
      signal: controller.signal,
    });
    clearTimeout(timeout);
  } catch (err) {
    console.error('[DEBUG] Fetch error:', err);
    const isAbort = (err as Error)?.name === 'AbortError';
    const partial = blankPartial();
    partial.stopReason    = isAbort ? 'aborted' : 'error';
    partial.errorMessage  = isAbort ? 'Aborted' : String((err as Error)?.message ?? err);
    yield { type: 'error', reason: partial.stopReason as 'aborted' | 'error', error: partial };
    return;
  }

  if (!response.ok) {
    console.error('[DEBUG] Response not OK:', response.status);
    const text    = await response.text().catch(() => '');

    const partial = blankPartial();
    partial.stopReason   = 'error';
    partial.errorMessage = `HTTP ${response.status}: ${text.slice(0, 200)}`;
    yield { type: 'error', reason: 'error', error: partial };
    return;
  }

  console.error('[DEBUG] Response OK, starting SSE parse');
  // ── SSE parsing state ──────────────────────────────────────────────────────
  const partial = blankPartial();
  let emittedStart = false;

  // Per-content-block accumulators.
  let textBuffer     = '';
  let thinkingBuffer = '';

  // State machine for MiniMax reasoning stream:
  // MiniMax sends tool calls after </thinking> inside delta.reasoning.
  // thinkingClosed = true once we detect the tool-call boundary — all subsequent
  // reasoning deltas are routed to the XML tool call parser, not the thinking block.
  // We detect the boundary by the PRESENCE of XML tool call tags, NOT by </thinking>
  // text (which could legitimately appear inside code examples in thinking content).
  let thinkingClosed = false;
  // Overflow buffer: holds reasoning text that arrived in the same chunk as the
  // boundary marker but before we identified it.
  let reasoningOverflow = '';

  // Tool call accumulators keyed by index.
  const tcAccum: Map<number, {
    id:        string;
    name:      string;
    argsRaw:   string;
    contentIdx: number;
    finalized?: boolean;   // true = already emitted (XML path), skip in finalization loop
  }> = new Map();

  let finishReason: string | null = null;
  let usageFromChunk: OROUsage | null = null;
  // Accumulate reasoning_details blocks from OpenRouter SSE.
  // Must be passed back unmodified on next turn — OpenRouter docs explicitly require this for MiniMax.
  const reasoningDetailsAcc: unknown[] = [];

  // ── Read SSE stream ────────────────────────────────────────────────────────
  const reader = response.body!.getReader();
  const decoder = new TextDecoder();
  let buf = '';

  try {
    outer: while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buf += decoder.decode(value, { stream: true });

      // Split on SSE line boundaries.
      const lines = buf.split('\n');
      buf = lines.pop()!; // keep incomplete line

      for (const line of lines) {
        if (!line.startsWith('data: ')) continue;

        const data = line.slice(6).trim();
        if (data === '[DONE]') break outer;

        let chunk: ORChunk;
        try { chunk = JSON.parse(data); } catch { continue; }

        // Log raw delta for diagnosis (only when DEBUG_SSE=1).
        if (process.env['DEBUG_SSE'] === '1') {
          const rawDelta = chunk.choices?.[0]?.delta;
          if (rawDelta?.content !== undefined && rawDelta.content !== null) {
            const contentType = Array.isArray(rawDelta.content) ? 'array' : typeof rawDelta.content;
            logToFile(`[SSE] delta.content type=${contentType} value=${JSON.stringify(rawDelta.content).slice(0, 120)}`);
          }
        }


        // Capture usage if provided.
        if (chunk.usage) usageFromChunk = chunk.usage as OROUsage;

        // Capture reasoning_details for passback (OpenRouter MiniMax requirement).
        if (chunk.choices?.[0]?.delta?.reasoning_details) {
          for (const rd of chunk.choices[0].delta.reasoning_details as unknown[]) {
            reasoningDetailsAcc.push(rd);
          }
        }

        const choice = chunk.choices?.[0];
        if (!choice) continue;

        if (choice.finish_reason) finishReason = choice.finish_reason;

        const delta = choice.delta;

        // ── Emit start on first delta ────────────────────────────────────
        if (!emittedStart) {
          emittedStart = true;
          yield { type: 'start', partial: { ...partial } };
        }

        // ── Reasoning / thinking delta ───────────────────────────────────
        // MiniMax quirk: tool call XML arrives via delta.reasoning, appended
        // AFTER </thinking> within the same reasoning stream.
        //
        // The WRONG approach (old): split on the text "</thinking>" — fragile
        // because a model might write </thinking> inside a code block example
        // in its actual thinking content.
        //
        // The RIGHT approach: detect tool-call XML tags as the boundary marker.
        // Tool call XML is unambiguous — <minimax:tool_call> or <invoke name=
        // cannot appear as legitimate thinking prose. Once we see these tags,
        // the thinking phase is definitively over (thinkingClosed = true) and
        // all subsequent reasoning deltas are routed through extractXmlToolCalls.
        if (delta.reasoning) {
          const raw = reasoningOverflow + delta.reasoning;
          reasoningOverflow = '';

          // Detect the tool-call boundary by presence of XML tool call markers.
          // We look for the earliest position of any tool-call opening tag.
          const TOOL_MARKERS = ['<minimax:tool_call>', '<invoke name=', '<tool_call>'];
          let boundaryIdx = -1;
          let matchedMarker = '';
          if (!thinkingClosed) {
            for (const marker of TOOL_MARKERS) {
              const idx = raw.indexOf(marker);
              if (idx !== -1 && (boundaryIdx === -1 || idx < boundaryIdx)) {
                boundaryIdx = idx;
                matchedMarker = marker;
              }
            }
          }

          // Helper: emit a chunk as XML tool calls (post-boundary path).
          // Declared as an inline async generator so yield works correctly.
          const emitAsXml = async function*(chunk: string) {
            const { textBefore, calls } = extractXmlToolCalls(chunk);
            if (textBefore.trim()) {
              const isFirstText = partial.content.filter(c => c.type === 'text').length === 0;
              if (isFirstText) {
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

          if (!thinkingClosed && boundaryIdx === -1) {
            // Pure thinking content — no tool call markers anywhere.
            // But guard against a partial marker split across two chunks:
            // e.g. chunk 1 ends with "<minimax:" and chunk 2 starts with "tool_call>".
            // We hold back the last N chars as overflow when the chunk ends with a
            // partial match of any marker prefix.
            let safeUpto = raw.length;
            for (const marker of TOOL_MARKERS) {
              // Find the longest suffix of `raw` that is a prefix of `marker`.
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
              const thinkingIdx = findLastIndex(partial.content, c => c.type === 'thinking');
              thinkingBuffer += thinkPart;
              (partial.content[thinkingIdx] as ThinkingContent).thinking = thinkingBuffer;
              yield { type: 'thinking_delta', contentIndex: thinkingIdx, delta: thinkPart, partial: { ...partial } };
            }

          } else if (!thinkingClosed && boundaryIdx !== -1) {
            // Boundary found in this chunk. Everything before it is thinking content,
            // everything from the marker onwards is tool call XML.
            thinkingClosed = true;

            const thinkPart = raw.slice(0, boundaryIdx)
              // Strip trailing </thinking> if present — it's structural, not content.
              .replace(/<\/thinking>\s*$/, '').trimEnd();
            const xmlPart   = raw.slice(boundaryIdx);

            if (thinkPart) {
              if (thinkingBuffer === '') {
                const idx = partial.content.length;
                partial.content.push({ type: 'thinking', thinking: '' });
                yield { type: 'thinking_start', contentIndex: idx, partial: { ...partial } };
              }
              const thinkingIdx = findLastIndex(partial.content, c => c.type === 'thinking');
              thinkingBuffer += thinkPart;
              (partial.content[thinkingIdx] as ThinkingContent).thinking = thinkingBuffer;
              yield { type: 'thinking_delta', contentIndex: thinkingIdx, delta: thinkPart, partial: { ...partial } };
            }

            if (xmlPart.trim()) {
              yield* emitAsXml(xmlPart);
            }

          } else {
            // thinkingClosed — all subsequent reasoning deltas are tool call XML.
            if (raw.trim()) {
              yield* emitAsXml(raw);
            }
          }
        }

        // ── Text delta ───────────────────────────────────────────────────
        // delta.content may be a string (standard) or array-of-parts (some models).
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

          // Surface any non-XML prose.
          if (textBefore) {
            const isFirstText = partial.content.filter(c => c.type === 'text').length === 0;
            if (isFirstText) {
              const idx = partial.content.length;
              partial.content.push({ type: 'text', text: '' });
              yield { type: 'text_start', contentIndex: idx, partial: { ...partial } };
            }
            const textIdx = findLastIndex(partial.content, c => c.type === 'text');
            textBuffer += textBefore;
            (partial.content[textIdx] as TextContent).text = textBuffer;
            yield { type: 'text_delta', contentIndex: textIdx, delta: textBefore, partial: { ...partial } };
          }

          // Convert XML-embedded tool calls into proper toolCall content blocks.
          for (const xmlTc of calls) {
            const contentIdx = partial.content.length;
            partial.content.push({ type: 'toolCall', id: '', name: '', arguments: {} });
            yield { type: 'toolcall_start', contentIndex: contentIdx, partial: { ...partial } };
            const toolCall: ToolCall = { type: 'toolCall', id: xmlTc.id, name: xmlTc.name, arguments: xmlTc.arguments };
            partial.content[contentIdx] = toolCall;
            // Store with finalized=true so the post-stream finalization loop skips it.
            tcAccum.set(10000 + contentIdx, { id: xmlTc.id, name: xmlTc.name, argsRaw: JSON.stringify(xmlTc.arguments), contentIdx, finalized: true });
            yield { type: 'toolcall_end', contentIndex: contentIdx, toolCall, partial: { ...partial } };
          }
        }

        // ── Tool call deltas ─────────────────────────────────────────────
        if (delta.tool_calls) {
          for (const tc of delta.tool_calls) {
            if (!tcAccum.has(tc.index)) {
              const contentIdx = partial.content.length;
              // Placeholder — filled in at toolcall_end.
              partial.content.push({ type: 'toolCall', id: '', name: '', arguments: {} });
              tcAccum.set(tc.index, { id: '', name: '', argsRaw: '', contentIdx });
              yield { type: 'toolcall_start', contentIndex: contentIdx, partial: { ...partial } };
            }

            const acc = tcAccum.get(tc.index)!;
            if (tc.id)                acc.id          += tc.id;
            if (tc.function?.name)    acc.name         += tc.function.name;
            if (tc.function?.arguments) {
              acc.argsRaw += tc.function.arguments;
              yield { type: 'toolcall_delta', contentIndex: acc.contentIdx, delta: tc.function.arguments, partial: { ...partial } };
            }
          }
        }
      }
    }
  } finally {
    reader.releaseLock();
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
