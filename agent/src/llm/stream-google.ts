/**
 * Google Generative AI (Gemini) streamer.
 *
 * Uses the native Google REST API with server-sent JSON (not SSE).
 * Supports: gemini-2.5-pro, gemini-2.5-flash, gemini-2.0-flash, gemini-1.5-pro, etc.
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

interface GeminiPart {
  text?: string;
  thought?: boolean;
  functionCall?: {
    name: string;
    args: Record<string, unknown>;
  };
  functionResponse?: {
    name: string;
    response: unknown;
  };
}

interface GeminiContent {
  role: 'user' | 'model';
  parts: GeminiPart[];
}

function toGeminiContents(messages: Message[]): GeminiContent[] {
  const result: GeminiContent[] = [];

  for (const msg of messages) {
    if (msg.role === 'user') {
      result.push({ role: 'user', parts: [{ text: msg.content }] });
      continue;
    }

    if (msg.role === 'assistant') {
      const parts: GeminiPart[] = [];
      for (const part of msg.content) {
        if (part.type === 'text') parts.push({ text: part.text });
        if (part.type === 'thinking') parts.push({ text: part.thinking, thought: true });
        if (part.type === 'toolCall') parts.push({ functionCall: { name: part.name, args: part.arguments } });
      }
      if (parts.length > 0) result.push({ role: 'model', parts });
      continue;
    }

    if (msg.role === 'toolResult') {
      result.push({
        role: 'user',
        parts: [{
          functionResponse: {
            name: msg.toolName,
            response: { output: msg.content.map(c => c.text).join('\n') },
          },
        }],
      });
    }
  }

  return result;
}

function toGeminiTools(tools: AgentTool[]): unknown[] {
  return [{
    functionDeclarations: tools.map(t => ({
      name: t.name,
      description: t.description,
      parameters: t.parameters,
    })),
  }];
}

function emptyUsage(): Usage {
  return { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0 };
}

function blankPartial(): AssistantMessage {
  return { role: 'assistant', content: [], usage: emptyUsage(), stopReason: 'stop', timestamp: Date.now() };
}

export async function* streamGoogle(
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

  const cleanModel = model.replace(/:thinking$/, '');
  const isThinking = model.includes(':thinking') || model.includes('2.5');

  const body: Record<string, unknown> = {
    contents: toGeminiContents(context.messages),
    generationConfig: {
      temperature: options.temperature,
      maxOutputTokens: isThinking ? 24576 : 8192,
      ...(isThinking ? { thinkingConfig: { thinkingBudget: 8192, includeThoughts: true } } : {}),
    },
  };

  if (context.systemPrompt) {
    body.systemInstruction = { parts: [{ text: context.systemPrompt }] };
  }

  if (context.tools && context.tools.length > 0) {
    body.tools = toGeminiTools(context.tools);
    body.toolConfig = { functionCallingConfig: { mode: 'AUTO' } };
  }

  const url = `${options.baseUrl}/models/${cleanModel}:streamGenerateContent?alt=sse&key=${options.apiKey}`;

  let response: Response;
  try {
    response = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
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

  let textContentIdx: number | null = null;
  let thinkContentIdx: number | null = null;
  let hasToolCalls = false;

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
        if (!data || data === '[DONE]') continue;

        let chunk: Record<string, unknown>;
        try { chunk = JSON.parse(data); } catch { continue; }

        const usageMeta = chunk.usageMetadata as Record<string, number> | undefined;
        if (usageMeta) {
          partial.usage.input = usageMeta.promptTokenCount ?? partial.usage.input;
          partial.usage.output = usageMeta.candidatesTokenCount ?? partial.usage.output;
          partial.usage.totalTokens = usageMeta.totalTokenCount ?? partial.usage.totalTokens;
        }

        const candidates = chunk.candidates as Array<Record<string, unknown>> | undefined;
        if (!candidates?.length) continue;

        const candidate = candidates[0];
        const content = candidate.content as GeminiContent | undefined;
        const finishReason = candidate.finishReason as string | undefined;

        if (content?.parts) {
          for (const part of content.parts) {
            if (part.thought && part.text) {
              if (thinkContentIdx === null) {
                thinkContentIdx = partial.content.length;
                partial.content.push({ type: 'thinking', thinking: '' });
                yield { type: 'thinking_start', contentIndex: thinkContentIdx, partial: { ...partial } };
              }
              (partial.content[thinkContentIdx] as ThinkingContent).thinking += part.text;
              yield { type: 'thinking_delta', contentIndex: thinkContentIdx, delta: part.text, partial: { ...partial } };
              continue;
            }

            if (part.text) {
              if (textContentIdx === null) {
                textContentIdx = partial.content.length;
                partial.content.push({ type: 'text', text: '' });
                yield { type: 'text_start', contentIndex: textContentIdx, partial: { ...partial } };
              }
              (partial.content[textContentIdx] as TextContent).text += part.text;
              yield { type: 'text_delta', contentIndex: textContentIdx, delta: part.text, partial: { ...partial } };
              continue;
            }

            if (part.functionCall) {
              hasToolCalls = true;
              const tc = part.functionCall;
              const contentIdx = partial.content.length;
              const toolCall: ToolCall = {
                type: 'toolCall',
                id: `gemini-${Date.now()}-${contentIdx}`,
                name: tc.name,
                arguments: tc.args ?? {},
              };
              partial.content.push(toolCall);
              yield { type: 'toolcall_start', contentIndex: contentIdx, partial: { ...partial } };
              yield { type: 'toolcall_end', contentIndex: contentIdx, toolCall, partial: { ...partial } };
            }
          }
        }

        if (finishReason) {
          if (thinkContentIdx !== null) {
            const content = (partial.content[thinkContentIdx] as ThinkingContent).thinking;
            yield { type: 'thinking_end', contentIndex: thinkContentIdx, content, partial: { ...partial } };
          }
          if (textContentIdx !== null) {
            const content = (partial.content[textContentIdx] as TextContent).text;
            yield { type: 'text_end', contentIndex: textContentIdx, content, partial: { ...partial } };
          }
        }
      }
    }
  } finally {
    reader.releaseLock();
  }

  if (hasToolCalls) {
    partial.stopReason = 'toolUse';
    yield { type: 'done', reason: 'toolUse', message: { ...partial } };
  } else {
    partial.stopReason = 'stop';
    yield { type: 'done', reason: 'stop', message: { ...partial } };
  }
}