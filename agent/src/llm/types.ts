/**
 * Core LLM message and event types.
 *
 * Mirrors pi-agent-core's type system exactly.
 * No dependency on the Vercel AI SDK.
 */

// ── Message types ─────────────────────────────────────────────────────────────

export interface TextContent {
  type: 'text';
  text: string;
}

export interface ThinkingContent {
  type: 'thinking';
  thinking: string;
}

export interface ToolCall {
  type: 'toolCall';
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

export interface Usage {
  input:       number;
  output:      number;
  cacheRead:   number;
  cacheWrite:  number;
  totalTokens: number;
}

export type StopReason = 'stop' | 'length' | 'toolUse' | 'error' | 'aborted';

export interface UserMessage {
  role:      'user';
  content:   string;
  timestamp: number;
}

export interface AssistantMessage {
  role:             'assistant';
  content:          (TextContent | ThinkingContent | ToolCall)[];
  usage:            Usage;
  stopReason:       StopReason;
  errorMessage?:    string;
  timestamp:        number;
  /** Raw reasoning_details from OpenRouter SSE — must be passed back unmodified on next turn. */
  reasoningDetails?: unknown[];
}

export interface ToolResultMessage {
  role:       'toolResult';
  toolCallId: string;
  toolName:   string;
  content:    TextContent[];
  isError:    boolean;
  timestamp:  number;
}

export type Message = UserMessage | AssistantMessage | ToolResultMessage;

// ── Tool types ────────────────────────────────────────────────────────────────

export interface ToolParameter {
  type:        string;
  description?: string;
  properties?: Record<string, ToolParameter>;
  required?:   string[];
  items?:      ToolParameter;
  enum?:       unknown[];
}

export interface ToolSchema {
  type:       'object';
  properties: Record<string, ToolParameter>;
  required?:  string[];
}

/** Partial result streamed during tool execution. */
export interface ToolUpdateResult {
  content: TextContent[];
}

/** Callback for streaming partial tool output to TUI. */
export type ToolUpdateCallback = (partial: ToolUpdateResult) => void;

/** Full result returned by a tool. */
export interface ToolResult {
  content:    TextContent[];
  isError?:   boolean;
  /** When true, agent stops after the current tool batch. */
  terminate?: boolean;
}

/** Tool execution mode per-tool or globally. */
export type ToolExecutionMode = 'sequential' | 'parallel';

/** A tool registered with the agent. */
export interface AgentTool {
  name:          string;
  label:         string;
  description:   string;
  parameters:    ToolSchema;
  executionMode?: ToolExecutionMode;
  /** Optional hook to normalize/reshape raw model args before validation. */
  prepareArguments?: (args: Record<string, unknown>) => Record<string, unknown>;
  execute: (
    toolCallId: string,
    args:       Record<string, unknown>,
    signal?:    AbortSignal,
    onUpdate?:  ToolUpdateCallback,
  ) => Promise<ToolResult>;
}

// ── Context ───────────────────────────────────────────────────────────────────

export interface AgentContext {
  systemPrompt: string;
  messages:     Message[];
  tools:        AgentTool[];
}

// ── Streaming event protocol ──────────────────────────────────────────────────

/**
 * Events emitted by the stream layer during an assistant response.
 * Mirrors pi-ai's AssistantMessageEvent union.
 */
export type AssistantMessageEvent =
  | { type: 'start';          partial: AssistantMessage }
  | { type: 'text_start';     contentIndex: number; partial: AssistantMessage }
  | { type: 'text_delta';     contentIndex: number; delta: string; partial: AssistantMessage }
  | { type: 'text_end';       contentIndex: number; content: string; partial: AssistantMessage }
  | { type: 'thinking_start'; contentIndex: number; partial: AssistantMessage }
  | { type: 'thinking_delta'; contentIndex: number; delta: string; partial: AssistantMessage }
  | { type: 'thinking_end';   contentIndex: number; content: string; partial: AssistantMessage }
  | { type: 'toolcall_start'; contentIndex: number; partial: AssistantMessage }
  | { type: 'toolcall_delta'; contentIndex: number; delta: string; partial: AssistantMessage }
  | { type: 'toolcall_end';   contentIndex: number; toolCall: ToolCall; partial: AssistantMessage }
  | { type: 'done';  reason: Extract<StopReason, 'stop' | 'length' | 'toolUse'>; message: AssistantMessage }
  | { type: 'error'; reason: Extract<StopReason, 'aborted' | 'error'>;           error: AssistantMessage };

// ── Agent loop event protocol ─────────────────────────────────────────────────

/**
 * Events emitted by the agent loop.
 * Mirrors pi-agent-core's AgentEvent union.
 */
export type AgentLoopEvent =
  | { type: 'agent_start' }
  | { type: 'agent_end';             messages: Message[] }
  | { type: 'turn_start' }
  | { type: 'turn_end';              message: AssistantMessage; toolResults: ToolResultMessage[] }
  | { type: 'message_start';         message: Message }
  | { type: 'message_update';        message: AssistantMessage; event: AssistantMessageEvent }
  | { type: 'message_end';           message: Message }
  | { type: 'tool_execution_start';  toolCallId: string; toolName: string; args: Record<string, unknown> }
  | { type: 'tool_execution_update'; toolCallId: string; toolName: string; partialResult: ToolUpdateResult }
  | { type: 'tool_execution_end';    toolCallId: string; toolName: string; result: ToolResult; isError: boolean };

// ── Loop config ───────────────────────────────────────────────────────────────

export interface BeforeToolCallContext {
  assistantMessage: AssistantMessage;
  toolCall:         ToolCall;
  args:             Record<string, unknown>;
  context:          AgentContext;
}

export interface BeforeToolCallResult {
  block?:  boolean;
  reason?: string;
}

export interface AfterToolCallContext {
  assistantMessage: AssistantMessage;
  toolCall:         ToolCall;
  args:             Record<string, unknown>;
  result:           ToolResult;
  isError:          boolean;
  context:          AgentContext;
}

export interface AfterToolCallResult {
  content?:   TextContent[];
  isError?:   boolean;
  terminate?: boolean;
}

export interface ShouldStopContext {
  message:     AssistantMessage;
  toolResults: ToolResultMessage[];
  context:     AgentContext;
  newMessages: Message[];
}

export interface AgentLoopConfig {
  /** Model string for OpenRouter, e.g. "minimax/minimax-m1:free" */
  model:       string;
  temperature: number;

  /**
   * Optional context transformation hook — called before every LLM request.
   * Use for context pruning, summarization, or dropping stale tool results
   * when the context window is filling up. Pi implements this.
   */
  transformContext?: (messages: Message[], signal?: AbortSignal) => Promise<Message[]>;

  /** Called before a tool executes. Return { block: true } to prevent execution. */
  beforeToolCall?: (ctx: BeforeToolCallContext, signal?: AbortSignal) => Promise<BeforeToolCallResult | undefined>;

  /** Called after a tool executes. Return overrides to patch result. */
  afterToolCall?: (ctx: AfterToolCallContext, signal?: AbortSignal) => Promise<AfterToolCallResult | undefined>;

  /** Return true to stop the loop after the current turn. */
  shouldStopAfterTurn?: (ctx: ShouldStopContext) => boolean | Promise<boolean>;

  /** Return steering messages to inject before the next LLM call. */
  getSteeringMessages?: () => Promise<Message[]>;

  /** Return follow-up messages to run after the agent would otherwise stop. */
  getFollowUpMessages?: () => Promise<Message[]>;

  /** Sequential or parallel tool execution. Default: 'parallel'. */
  toolExecution?: ToolExecutionMode;
}
