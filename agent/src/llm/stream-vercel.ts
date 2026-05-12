/**
 * Vercel AI SDK streaming layer for OpenRouter.
 */

import { createOpenRouter } from '@openrouter/ai-sdk-provider';
import { streamText, tool } from 'ai';
import { z } from 'zod';
import type {
  AgentContext,
  AgentTool,
  AssistantMessage,
  AssistantMessageEvent,
  Message,
  TextContent,
  ToolCall,
  Usage,
} from './types.js';

function toVercelMessages(messages: Message[]): any[] {
  const result: any[] = [];

  for (const msg of messages) {
    if (msg.role === 'user') {
      result.push({ role: 'user', content: [{ type: 'text', text: msg.content as string }] });
      continue;
    }

    if (msg.role === 'assistant') {
      const content: any[] = [];

      for (const part of msg.content) {
        if (part.type === 'text') {
          content.push({ type: 'text', text: part.text });
        } else if (part.type === 'thinking') {
          continue;
        } else if (part.type === 'toolCall') {
          content.push({
            type: 'tool-call',
            toolCallId: part.id,
            toolName: part.name,
            input: part.arguments,
          });
        }
      }

      result.push({ role: 'assistant', content });
      continue;
    }

    if (msg.role === 'toolResult') {
      result.push({
        role: 'tool',
        content: msg.content.map(c => ({
          type: 'tool-result',
          toolCallId: msg.toolCallId,
          toolName: 'unknown',
          output: { type: 'text', value: c.text },
        })),
      });
    }
  }

  return result;
}

function agentToolToVercel(t: AgentTool): any {
  const paramProps: Record<string, any> = {};
  const props = t.parameters.properties;

  for (const [key, prop] of Object.entries(props)) {
    let zodType: any;

    if (prop.type === 'string') {
      zodType = z.string();
    } else if (prop.type === 'number') {
      zodType = z.number();
    } else {
      zodType = z.any();
    }

    if (prop.description) {
      zodType = zodType.describe(prop.description);
    }

    if (t.parameters.required?.includes(key)) {
      paramProps[key] = zodType;
    } else {
      paramProps[key] = zodType.optional();
    }
  }

  return tool({
    description: t.description,
    inputSchema: z.object(paramProps),
  });
}

function emptyUsage(): Usage {
  return { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0 };
}

export async function* streamLLM(
  model: string,
  context: AgentContext,
  options: {
    temperature: number;
    signal?: AbortSignal;
  },
): AsyncIterable<AssistantMessageEvent> {
  const apiKey = process.env.OPENROUTER_API_KEY ?? '';

  console.error(`[stream-vercel] model=${model}`);

  const provider = createOpenRouter({ apiKey });

  const baseModel: any = provider.chat(model);

  const vercelTools: Record<string, any> = {};
  for (const t of context.tools) {
    vercelTools[t.name] = agentToolToVercel(t);
  }

  const messages = toVercelMessages(context.messages);
  const system = context.systemPrompt;

  const result: any = streamText({
    model: baseModel,
    tools: context.tools.length > 0 ? vercelTools : undefined,
    temperature: options.temperature,
    system,
    messages,
    providerOptions: {
      openrouter: {
        reasoning: { effort: 'medium' },
      },
    },
    stopWhen: (info: any) => {
      return info.stepCount >= 20;
    },
  });

  let partial: AssistantMessage = {
    role: 'assistant',
    content: [],
    usage: emptyUsage(),
    stopReason: 'stop',
    timestamp: Date.now(),
  };
  let started = false;
  let textBuffer = '';
  let textIndex = -1;
  let thinkingBuffer = '';
  let thinkingIndex = -1;
  let finishReasonFromStream: string | undefined;

  for await (const chunk of result.fullStream) {
    switch (chunk.type) {
      case 'tool-input-start':
      case 'tool-input-delta':
      case 'tool-input-end':
        break;
      case 'text-start': {
        if (!started) {
          started = true;
          yield { type: 'start', partial: { ...partial } };
        }
        textIndex = partial.content.length;
        partial.content.push({ type: 'text', text: '' });
        yield { type: 'text_start', contentIndex: textIndex, partial: { ...partial } };
        break;
      }
      case 'text-delta': {
        const ti = textIndex >= 0 ? textIndex : findLastIndex(partial.content, (c: any) => c.type === 'text');
        if (ti >= 0) {
          textBuffer += chunk.text;
          (partial.content[ti] as TextContent).text = textBuffer;
          yield { type: 'text_delta', contentIndex: ti, delta: chunk.text, partial: { ...partial } };
        }
        break;
      }
      case 'text-end': {
        const ti = textIndex >= 0 ? textIndex : findLastIndex(partial.content, (c: any) => c.type === 'text');
        if (ti >= 0) {
          textBuffer = (partial.content[ti] as TextContent).text;
          yield { type: 'text_end', contentIndex: ti, content: textBuffer, partial: { ...partial } };
        }
        break;
      }
      case 'tool-call': {
        const idx = partial.content.length;
        const toolCall: ToolCall = {
          type: 'toolCall',
          id: chunk.toolCallId || '',
          name: chunk.toolName,
          arguments: (chunk.input as any) || {},
        };
        partial.content.push(toolCall);
        yield { type: 'toolcall_start', contentIndex: idx, partial: { ...partial } };
        yield { type: 'toolcall_end', contentIndex: idx, toolCall, partial: { ...partial } };
        break;
      }
      case 'reasoning-start': {
        if (!started) {
          started = true;
          yield { type: 'start', partial: { ...partial } };
        }
        thinkingIndex = partial.content.length;
        partial.content.push({ type: 'thinking', thinking: '' });
        yield { type: 'thinking_start', contentIndex: thinkingIndex, partial: { ...partial } };
        break;
      }
      case 'reasoning-delta': {
        if (!started) {
          started = true;
          yield { type: 'start', partial: { ...partial } };
        }
        const delta = typeof chunk.delta === 'string'
          ? chunk.delta
          : (typeof (chunk as any).text === 'string' ? (chunk as any).text : '');
        if (!delta) break;
        const ti = thinkingIndex >= 0 ? thinkingIndex : findLastIndex(partial.content, (c: any) => c.type === 'thinking');
        if (ti >= 0) {
          thinkingBuffer += delta;
          (partial.content[ti] as any).thinking = thinkingBuffer;
          yield { type: 'thinking_delta', contentIndex: ti, delta, partial: { ...partial } };
        }
        break;
      }
      case 'reasoning-end': {
        const ti = thinkingIndex >= 0 ? thinkingIndex : findLastIndex(partial.content, (c: any) => c.type === 'thinking');
        if (ti >= 0) {
          thinkingBuffer = (partial.content[ti] as any).thinking;
          yield { type: 'thinking_end', contentIndex: ti, content: thinkingBuffer, partial: { ...partial } };
        }
        break;
      }
      case 'finish': {
        finishReasonFromStream = chunk.finishReason ?? finishReasonFromStream;
        break;
      }
      case 'error': {
        partial.stopReason = 'error';
        partial.errorMessage = chunk.error?.message ?? 'Stream error';
        yield { type: 'error', reason: 'error', error: { ...partial } };
        return;
      }
    }
  }

  const response: any = await result.response;
  const usageResult: any = await result.usage;
  const finishReason: any = finishReasonFromStream ?? await result.finishReason;

  if (usageResult?.inputTokens || usageResult?.outputTokens) {
    partial.usage = {
      input:      usageResult.inputTokens       ?? 0,
      output:     usageResult.outputTokens      ?? 0,
      cacheRead:  usageResult.cachedInputTokens ?? 0,
      cacheWrite: 0,
      totalTokens: (usageResult.inputTokens ?? 0) + (usageResult.outputTokens ?? 0),
    };
  }

  const hasToolCalls = partial.content.some((c: any) => c.type === 'toolCall');
  partial.stopReason = (hasToolCalls || finishReason === 'tool-calls') ? 'toolUse' : 'stop';
  yield { type: 'done', reason: partial.stopReason, message: { ...partial } };
}

function findLastIndex<T>(arr: T[], predicate: (val: T) => boolean): number {
  for (let i = arr.length - 1; i >= 0; i--) {
    if (predicate(arr[i])) return i;
  }
  return -1;
} 