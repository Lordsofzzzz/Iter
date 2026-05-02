/**
 * Agent loop — exact structural port of pi-agent-core's agent-loop.ts.
 *
 * Features:
 * - Outer loop: handles follow-up messages after agent would stop
 * - Inner loop: handles tool call turns + steering message injection
 * - beforeToolCall / afterToolCall hooks
 * - shouldStopAfterTurn hook
 * - Sequential and parallel tool execution modes
 * - Streams partial assistant messages into context during LLM response
 */

import { emitEvent as rpcEmitEvent } from '../rpc.js';
import { streamOpenRouter } from './stream.js';
import { validateToolArguments } from '../utils/validation.js';
import type {
  AfterToolCallResult,
  AgentContext,
  AgentLoopConfig,
  AgentLoopEvent,
  AgentTool,
  AssistantMessage,
  Message,
  ToolCall,
  ToolResult,
  ToolResultMessage,
  ToolUpdateResult,
} from './types.js';

// ── Public API ────────────────────────────────────────────────────────────────

export type AgentEventSink = (event: AgentLoopEvent) => void;

/**
 * Run the agent loop from a new user prompt.
 * Returns all new messages produced during this run.
 */
export async function runAgentLoop(
  userMessage: string,
  context:     AgentContext,
  config:      AgentLoopConfig,
  emit:        AgentEventSink,
  signal?:     AbortSignal,
): Promise<Message[]> {
  const userMsg: Message = { role: 'user', content: userMessage, timestamp: Date.now() };
  const newMessages: Message[] = [userMsg];

  const currentContext: AgentContext = {
    ...context,
    messages: [...context.messages, userMsg],
  };

  await emitAsync(emit, { type: 'agent_start' });
  await emitAsync(emit, { type: 'turn_start' });
  await emitAsync(emit, { type: 'message_start', message: userMsg });
  await emitAsync(emit, { type: 'message_end',   message: userMsg });

  await runLoop(currentContext, newMessages, config, signal, emit);

  return newMessages;
}

// ── Core loop ─────────────────────────────────────────────────────────────────

async function runLoop(
  ctx:         AgentContext,
  newMessages: Message[],
  config:      AgentLoopConfig,
  signal:      AbortSignal | undefined,
  emit:        AgentEventSink,
): Promise<void> {
  let firstTurn = true;

  // Check for steering messages already queued before we start.
  let pendingMessages = (await config.getSteeringMessages?.()) ?? [];

  // ── Outer loop: re-enter when follow-up messages arrive ─────────────────
  while (true) {
    let hasMoreToolCalls = true;

    // ── Inner loop: tool call turns + steering injection ──────────────────
    while (hasMoreToolCalls || pendingMessages.length > 0) {
      if (!firstTurn) {
        await emitAsync(emit, { type: 'turn_start' });
      } else {
        firstTurn = false;
      }

      // Inject pending steering messages before the next LLM call.
      if (pendingMessages.length > 0) {
        for (const msg of pendingMessages) {
          await emitAsync(emit, { type: 'message_start', message: msg });
          await emitAsync(emit, { type: 'message_end',   message: msg });
          ctx.messages.push(msg);
          newMessages.push(msg);
        }
        pendingMessages = [];
      }

      // Stream one LLM response.
      const message = await streamAssistantResponse(ctx, config, signal, emit);
      newMessages.push(message);

      // Stop on error or abort — pi pattern: return normally, caller checks stopReason.
      // Never throw here — throwing prevents newMessages from being persisted in client.ts finally.
      if (message.stopReason === 'error' || message.stopReason === 'aborted') {
        await emitAsync(emit, { type: 'turn_end', message, toolResults: [] });
        await emitAsync(emit, { type: 'agent_end', messages: newMessages });
        return;
      }

      // Execute any tool calls.
      const toolCalls = message.content.filter(
        (c): c is ToolCall => c.type === 'toolCall'
      );

      const toolResults: ToolResultMessage[] = [];
      hasMoreToolCalls = false;

      if (toolCalls.length > 0) {
        const batch = await executeToolCalls(ctx, message, toolCalls, config, signal, emit);
        toolResults.push(...batch.messages);
        hasMoreToolCalls = !batch.terminate;

        for (const r of batch.messages) {
          ctx.messages.push(r);
          newMessages.push(r);
        }
      }

      await emitAsync(emit, { type: 'turn_end', message, toolResults });

      // shouldStopAfterTurn hook.
      if (await config.shouldStopAfterTurn?.({ message, toolResults, context: ctx, newMessages })) {
        await emitAsync(emit, { type: 'agent_end', messages: newMessages });
        return;
      }

      pendingMessages = (await config.getSteeringMessages?.()) ?? [];
    }

    // Agent would stop here — check for follow-up messages.
    const followUps = (await config.getFollowUpMessages?.()) ?? [];
    if (followUps.length > 0) {
      pendingMessages = followUps;
      continue;
    }

    break;
  }

  await emitAsync(emit, { type: 'agent_end', messages: newMessages });
}

// ── LLM streaming ─────────────────────────────────────────────────────────────

async function streamAssistantResponse(
  ctx:    AgentContext,
  config: AgentLoopConfig,
  signal: AbortSignal | undefined,
  emit:   AgentEventSink,
): Promise<AssistantMessage> {
  // Apply context transformation if configured — used for pruning/summarization.
  const messages = config.transformContext
    ? await config.transformContext([...ctx.messages], signal)
    : ctx.messages;

  const events = streamOpenRouter(config.model, { ...ctx, messages }, {
    temperature: config.temperature,
    signal,
  });

  let partial: AssistantMessage | null = null;
  let addedPartial = false;

  for await (const ev of events) {
    switch (ev.type) {
      case 'start':
        partial = ev.partial;
        ctx.messages.push(partial);
        addedPartial = true;
        await emitAsync(emit, { type: 'message_start', message: { ...partial } });
        break;

      case 'text_delta':
      case 'thinking_delta':
      case 'toolcall_delta':
      case 'text_start':
      case 'text_end':
      case 'thinking_start':
      case 'thinking_end':
      case 'toolcall_start':
      case 'toolcall_end':
        if (partial) {
          partial = ev.partial;
          ctx.messages[ctx.messages.length - 1] = partial;
          await emitAsync(emit, { type: 'message_update', message: { ...partial }, event: ev });
        }
        break;

      case 'done': {
        const final = ev.message;
        if (addedPartial) ctx.messages[ctx.messages.length - 1] = final;
        else              ctx.messages.push(final);
        await emitAsync(emit, { type: 'message_end', message: final });
        return final;
      }

      case 'error': {
        const final = ev.error;
        if (addedPartial) ctx.messages[ctx.messages.length - 1] = final;
        else              ctx.messages.push(final);
        if (!addedPartial) await emitAsync(emit, { type: 'message_start', message: { ...final } });
        await emitAsync(emit, { type: 'message_end', message: final });
        return final;
      }
    }
  }

  // Fallback — should not reach here.
  const fallback: AssistantMessage = {
    role:        'assistant',
    content:     [],
    usage:       { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0 },
    stopReason:  'error',
    errorMessage: 'Stream ended without done/error event',
    timestamp:   Date.now(),
  };
  return fallback;
}

// ── Tool execution ────────────────────────────────────────────────────────────

interface ToolBatchResult {
  messages:  ToolResultMessage[];
  terminate: boolean;
}

async function executeToolCalls(
  ctx:       AgentContext,
  assistant: AssistantMessage,
  toolCalls: ToolCall[],
  config:    AgentLoopConfig,
  signal:    AbortSignal | undefined,
  emit:      AgentEventSink,
): Promise<ToolBatchResult> {
  const mode = config.toolExecution ?? 'parallel';

  // If any tool in the batch is marked sequential, force sequential for all.
  const hasSequential = toolCalls.some(tc => {
    const tool = ctx.tools.find(t => t.name === tc.name);
    return tool?.executionMode === 'sequential';
  });

  if (mode === 'sequential' || hasSequential) {
    return executeSequential(ctx, assistant, toolCalls, config, signal, emit);
  }
  return executeParallel(ctx, assistant, toolCalls, config, signal, emit);
}

async function executeSequential(
  ctx:       AgentContext,
  assistant: AssistantMessage,
  toolCalls: ToolCall[],
  config:    AgentLoopConfig,
  signal:    AbortSignal | undefined,
  emit:      AgentEventSink,
): Promise<ToolBatchResult> {
  const messages:   ToolResultMessage[] = [];
  const terminated: boolean[]           = [];

  for (const tc of toolCalls) {
    await emitAsync(emit, { type: 'tool_execution_start', toolCallId: tc.id, toolName: tc.name, args: tc.arguments });

    const prep = await prepareToolCall(ctx, assistant, tc, config, signal);

    let finalized: FinalizedToolCall;
    if (prep.kind === 'immediate') {
      finalized = { toolCall: tc, result: prep.result, isError: prep.isError };
    } else {
      const executed = await executePrepared(prep, signal, emit);
      finalized = await finalizeToolCall(ctx, assistant, prep, executed, config, signal);
    }

    await emitAsync(emit, {
      type:       'tool_execution_end',
      toolCallId: finalized.toolCall.id,
      toolName:   finalized.toolCall.name,
      result:     finalized.result,
      isError:    finalized.isError,
    });

    const resultMsg = makeToolResultMessage(finalized);
    await emitAsync(emit, { type: 'message_start', message: resultMsg });
    await emitAsync(emit, { type: 'message_end',   message: resultMsg });
    messages.push(resultMsg);
    terminated.push(finalized.result.terminate === true);
  }

  return {
    messages,
    terminate: terminated.length > 0 && terminated.every(Boolean),
  };
}

async function executeParallel(
  ctx:       AgentContext,
  assistant: AssistantMessage,
  toolCalls: ToolCall[],
  config:    AgentLoopConfig,
  signal:    AbortSignal | undefined,
  emit:      AgentEventSink,
): Promise<ToolBatchResult> {
  // Prepare phase is sequential (emit start events in order, validate args).
  type Entry = FinalizedToolCall | (() => Promise<FinalizedToolCall>);
  const entries: Entry[] = [];

  for (const tc of toolCalls) {
    await emitAsync(emit, { type: 'tool_execution_start', toolCallId: tc.id, toolName: tc.name, args: tc.arguments });

    const prep = await prepareToolCall(ctx, assistant, tc, config, signal);

    if (prep.kind === 'immediate') {
      const finalized: FinalizedToolCall = { toolCall: tc, result: prep.result, isError: prep.isError };
      await emitAsync(emit, {
        type:       'tool_execution_end',
        toolCallId: finalized.toolCall.id,
        toolName:   finalized.toolCall.name,
        result:     finalized.result,
        isError:    finalized.isError,
      });
      entries.push(finalized);
    } else {
      // Execution runs concurrently.
      entries.push(async () => {
        const executed = await executePrepared(prep, signal, emit);
        const finalized = await finalizeToolCall(ctx, assistant, prep, executed, config, signal);
        await emitAsync(emit, {
          type:       'tool_execution_end',
          toolCallId: finalized.toolCall.id,
          toolName:   finalized.toolCall.name,
          result:     finalized.result,
          isError:    finalized.isError,
        });
        return finalized;
      });
    }
  }

  // Run all async entries concurrently.
  const resolved = await Promise.all(
    entries.map(e => typeof e === 'function' ? e() : Promise.resolve(e))
  );

  // Emit tool result messages in original order.
  const messages: ToolResultMessage[] = [];
  for (const finalized of resolved) {
    const resultMsg = makeToolResultMessage(finalized);
    await emitAsync(emit, { type: 'message_start', message: resultMsg });
    await emitAsync(emit, { type: 'message_end',   message: resultMsg });
    messages.push(resultMsg);
  }

  return {
    messages,
    terminate: resolved.length > 0 && resolved.every(r => r.result.terminate === true),
  };
}

// ── Tool call preparation / execution / finalization ──────────────────────────

type PreparedImmediate = { kind: 'immediate'; result: ToolResult; isError: boolean };
type PreparedReady     = { kind: 'prepared';  toolCall: ToolCall; tool: AgentTool; args: Record<string, unknown> };
type Prepared          = PreparedImmediate | PreparedReady;

interface FinalizedToolCall {
  toolCall: ToolCall;
  result:   ToolResult;
  isError:  boolean;
}

async function prepareToolCall(
  ctx:       AgentContext,
  _assistant: AssistantMessage,
  tc:        ToolCall,
  config:    AgentLoopConfig,
  signal:    AbortSignal | undefined,
): Promise<Prepared> {
  const tool = ctx.tools.find(t => t.name === tc.name);

  if (!tool) {
    return {
      kind:    'immediate',
      result:  errorResult(`Tool "${tc.name}" not found`),
      isError: true,
    };
  }

  try {
    if (config.beforeToolCall) {
      const before = await config.beforeToolCall(
        { assistantMessage: _assistant, toolCall: tc, args: tc.arguments, context: ctx },
        signal,
      );
      if (before?.block) {
        return {
          kind:    'immediate',
          result:  errorResult(before.reason ?? 'Tool execution was blocked'),
          isError: true,
        };
      }
    }

    // prepareArguments hook — normalize/reshape raw model args before validation.
    // Mirrors pi's tool.prepareArguments() step.
    const rawArgs = tool.prepareArguments
      ? tool.prepareArguments(tc.arguments)
      : tc.arguments;

    // Validate and coerce args against the tool's JSON schema.
    const validatedArgs = validateToolArguments(tool, rawArgs);

    return { kind: 'prepared', toolCall: tc, tool, args: validatedArgs };

  } catch (err) {
    return {
      kind:    'immediate',
      result:  errorResult(String((err as Error)?.message ?? err)),
      isError: true,
    };
  }
}

interface ExecutedToolCall {
  result:  ToolResult;
  isError: boolean;
}

async function executePrepared(
  prep:   PreparedReady,
  signal: AbortSignal | undefined,
  emit:   AgentEventSink,
): Promise<ExecutedToolCall> {
  try {
    const result = await prep.tool.execute(
      prep.toolCall.id,
      prep.args,
      signal,
      (partial: ToolUpdateResult) => {
        emit({
          type:          'tool_execution_update',
          toolCallId:    prep.toolCall.id,
          toolName:      prep.toolCall.name,
          partialResult: partial,
        });
      },
    );
    return { result, isError: false };
  } catch (err) {
    return { result: errorResult(String((err as Error)?.message ?? err)), isError: true };
  }
}

async function finalizeToolCall(
  ctx:       AgentContext,
  assistant: AssistantMessage,
  prep:      PreparedReady,
  executed:  ExecutedToolCall,
  config:    AgentLoopConfig,
  signal:    AbortSignal | undefined,
): Promise<FinalizedToolCall> {
  let { result, isError } = executed;

  if (config.afterToolCall) {
    try {
      const after: AfterToolCallResult | undefined = await config.afterToolCall(
        { assistantMessage: assistant, toolCall: prep.toolCall, args: prep.args, result, isError, context: ctx },
        signal,
      );
      if (after) {
        result = {
          content:   after.content   ?? result.content,
          terminate: after.terminate ?? result.terminate,
        };
        isError = after.isError ?? isError;
      }
    } catch (err) {
      result  = errorResult(String((err as Error)?.message ?? err));
      isError = true;
    }
  }

  return { toolCall: prep.toolCall, result, isError };
}

function makeToolResultMessage(finalized: FinalizedToolCall): ToolResultMessage {
  return {
    role:       'toolResult',
    toolCallId: finalized.toolCall.id,
    toolName:   finalized.toolCall.name,
    content:    finalized.result.content,
    isError:    finalized.isError,
    timestamp:  Date.now(),
  };
}

function errorResult(message: string): ToolResult {
  return { content: [{ type: 'text', text: message }], isError: true };
}

// ── Helper ────────────────────────────────────────────────────────────────────

async function emitAsync(emit: AgentEventSink, event: AgentLoopEvent): Promise<void> {
  await Promise.resolve(emit(event));
}
