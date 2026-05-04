/**
 * Anthropic Messages API streamer.
 *
 * Speaks the native Anthropic Messages API with SSE streaming.
 * Supports: claude-3-5-sonnet, claude-3-7-sonnet, claude-sonnet-4, claude-opus-4, etc.
 */

import type {
  AgentContext,
  AgentTool,
  AssistantMessage,
  AssistantMessageEvent,
  Message,
  TextContent,
  ThinkingContent,
  ToolCall,
  Usage,
} from './types.js';

interface AnthropicTool {
  name: string;
  description?: string;
  input_schema: unknown;
}

function toAnthropicMessages(messages: Message[]): unknown[] {
  const result: unknown[] = [];

  for (const msg of messages) {
    if (msg.role === 'user') {
      result.push({ role: 'user', content: msg.content });
      continue;
    }

    if (msg.role === 'assistant') {
      const content: unknown[] = [];
      for (const part of msg.content) {
        if (part.type === 'text' && part.text) {
          content.push({ type: 'text', text: part.text });
        } else if (part.type === 'thinking' && part.thinking) {
          content.push({ type: 'thinking', thinking: part.thinking });
        } else if (part.type === 'toolCall') {
          content.push({ type: 'tool_use', id: part.id, name: part.name, input: part.arguments });
        }
      }
      if (content.length > 0) {
        result.push({ role: 'assistant', content });
      }
      continue;
    }

    if (msg.role === 'toolResult') {
      result.push({
        role: 'user',
        content: [{
          type: 'tool_result',
          tool_use_id: msg.toolCallId,
          content: msg.content.map(c => c.text).join('\n'),
          is_error: msg.isError,
        }],
      });
    }
  }

  return result;
}

function toAnthropicTools(tools: AgentTool[]): AnthropicTool[] {
  return tools.map(t => ({
    name: t.name,
    description: t.description,
    input_schema: t.parameters,
  }));
}

function emptyUsage(): Usage {
  return { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0 };
}

function blankPartial(): AssistantMessage {
  return { role: 'assistant', content: [], usage: emptyUsage(), stopReason: 'stop', timestamp: Date.now() };
}

export async function* streamAnthropic(
  model: string,
  context: AgentContext,
  options: {
    temperature: number;
    baseUrl: string;
    apiKey: string;
    signal?: AbortSignal;
  },
): AsyncIterable<AssistantMessageEvent> {
  const partial = blankPartial();

  const isThinking =
    model.includes('claude-3-7') ||
    model.includes('claude-sonnet-4') ||
    model.includes('claude-opus-4') ||
    model.includes(':thinking');

  const body: Record<string, unknown> = {
    model: model.replace(/:thinking$/, ''),
    max_tokens: isThinking ? 16000 : 8192,
    stream: true,
    messages: toAnthropicMessages(context.messages),
  };

  if (context.systemPrompt) {
    body.system = context.systemPrompt;
  }

  if (context.tools && context.tools.length > 0) {
    body.tools = toAnthropicTools(context.tools);
    body.tool_choice = { type: 'auto' };
  }

  if (isThinking) {
    body.thinking = { type: 'enabled', budget_tokens: 10000 };
    body.temperature = 1;
  } else {
    body.temperature = options.temperature;
  }

  let response: Response;
  try {
    response = await fetch(`${options.baseUrl}/messages`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'x-api-key': options.apiKey,
        'anthropic-version': '2023-06-01',
        'anthropic-beta': 'interleaved-thinking-2025-05-14',
      },
      body: JSON.stringify(body),
      signal: options.signal,
    });
  } catch (err) {
    const isAbort = (err as Error)?.name === 'AbortError';
    const p = blankPartial();
    p.stopReason = isAbort ? 'aborted' : 'error';
    p.errorMessage = isAbort ? 'Aborted' : String(err);
    yield { type: 'error', reason: p.stopReason as 'aborted' | 'error', error: p };
    return;
  }

  if (!response.ok) {
    const text = await response.text().catch(() => '');
    const p = blankPartial();
    p.stopReason = 'error';
    p.errorMessage = `HTTP ${response.status}: ${text.slice(0, 300)}`;
    yield { type: 'error', reason: 'error', error: p };
    return;
  }

  yield { type: 'start', partial };

  const reader = response.body!.getReader();
  const dec = new TextDecoder();
  let buf = '';

  const textBlocks: Map<number, number> = new Map();
  const thinkBlocks: Map<number, number> = new Map();
  const toolBlocks: Map<number, { contentIdx: number; id: string; name: string; argsBuf: string }> = new Map();

  let stopReason: string = 'stop';

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buf += dec.decode(value, { stream: true });

      const lines = buf.split('\n');
      buf = lines.pop() ?? '';

      for (const line of lines) {
        if (!line.startsWith('data: ')) continue;
        const data = line.slice(6).trim();
        if (data === '[DONE]') break;

        let ev: Record<string, unknown>;
        try { ev = JSON.parse(data); } catch { continue; }

        const evType = ev.type as string;

        if (evType === 'message_start') {
          const msg = ev.message as Record<string, unknown> | undefined;
          const usage = msg?.usage as Record<string, number> | undefined;
          if (usage) {
            partial.usage.input = usage.input_tokens ?? 0;
            partial.usage.cacheRead = usage.cache_read_input_tokens ?? 0;
            partial.usage.cacheWrite = usage.cache_creation_input_tokens ?? 0;
          }
          continue;
        }

        if (evType === 'content_block_start') {
          const idx = ev.index as number;
          const block = ev.content_block as { type: string; id?: string; name?: string };

          if (block.type === 'text') {
            const contentIdx = partial.content.length;
            partial.content.push({ type: 'text', text: '' });
            textBlocks.set(idx, contentIdx);
            yield { type: 'text_start', contentIndex: contentIdx, partial: { ...partial } };
          } else if (block.type === 'thinking') {
            const contentIdx = partial.content.length;
            partial.content.push({ type: 'thinking', thinking: '' });
            thinkBlocks.set(idx, contentIdx);
            yield { type: 'thinking_start', contentIndex: contentIdx, partial: { ...partial } };
          } else if (block.type === 'tool_use') {
            const contentIdx = partial.content.length;
            partial.content.push({ type: 'toolCall', id: '', name: '', arguments: {} });
            toolBlocks.set(idx, { contentIdx, id: block.id ?? '', name: block.name ?? '', argsBuf: '' });
            yield { type: 'toolcall_start', contentIndex: contentIdx, partial: { ...partial } };
          }
          continue;
        }

        if (evType === 'content_block_delta') {
          const idx = ev.index as number;
          const delta = ev.delta as Record<string, unknown>;

          if (delta.type === 'text_delta') {
            const d = delta.text as string;
            const contentIdx = textBlocks.get(idx);
            if (contentIdx !== undefined) {
              (partial.content[contentIdx] as TextContent).text += d;
              yield { type: 'text_delta', contentIndex: contentIdx, delta: d, partial: { ...partial } };
            }
          } else if (delta.type === 'thinking_delta') {
            const d = delta.thinking as string;
            const contentIdx = thinkBlocks.get(idx);
            if (contentIdx !== undefined) {
              (partial.content[contentIdx] as ThinkingContent).thinking += d;
              yield { type: 'thinking_delta', contentIndex: contentIdx, delta: d, partial: { ...partial } };
            }
          } else if (delta.type === 'input_json_delta') {
            const d = delta.partial_json as string;
            const acc = toolBlocks.get(idx);
            if (acc) {
              acc.argsBuf += d;
              yield { type: 'toolcall_delta', contentIndex: acc.contentIdx, delta: d, partial: { ...partial } };
            }
          }
          continue;
        }

        if (evType === 'content_block_stop') {
          const idx = ev.index as number;

          if (textBlocks.has(idx)) {
            const contentIdx = textBlocks.get(idx)!;
            const content = (partial.content[contentIdx] as TextContent).text;
            yield { type: 'text_end', contentIndex: contentIdx, content, partial: { ...partial } };
          } else if (thinkBlocks.has(idx)) {
            const contentIdx = thinkBlocks.get(idx)!;
            const content = (partial.content[contentIdx] as ThinkingContent).thinking;
            yield { type: 'thinking_end', contentIndex: contentIdx, content, partial: { ...partial } };
          } else if (toolBlocks.has(idx)) {
            const acc = toolBlocks.get(idx)!;
            let args: Record<string, unknown> = {};
            try { args = JSON.parse(acc.argsBuf || '{}'); } catch {}
            const toolCall: ToolCall = { type: 'toolCall', id: acc.id, name: acc.name, arguments: args };
            partial.content[acc.contentIdx] = toolCall;
            yield { type: 'toolcall_end', contentIndex: acc.contentIdx, toolCall, partial: { ...partial } };
          }
          continue;
        }

        if (evType === 'message_delta') {
          const d = ev.delta as Record<string, unknown> | undefined;
          if (d?.stop_reason) stopReason = d.stop_reason as string;
          const usage = ev.usage as Record<string, number> | undefined;
          if (usage) {
            partial.usage.output = usage.output_tokens ?? 0;
            partial.usage.totalTokens = partial.usage.input + partial.usage.output;
          }
          continue;
        }
      }
    }
  } finally {
    reader.releaseLock();
  }

  if (stopReason === 'tool_use') {
    partial.stopReason = 'toolUse';
    yield { type: 'done', reason: 'toolUse', message: { ...partial } };
  } else if (stopReason === 'max_tokens') {
    partial.stopReason = 'length';
    yield { type: 'done', reason: 'length', message: { ...partial } };
  } else {
    partial.stopReason = 'stop';
    yield { type: 'done', reason: 'stop', message: { ...partial } };
  }
}