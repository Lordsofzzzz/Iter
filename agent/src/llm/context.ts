/**
 * Context optimisation transforms.
 *
 * Applied via transformContext before every LLM call.
 * Non-destructive: this.messages is never mutated — only the
 * snapshot passed to the LLM is modified.
 *
 * Two strategies (mirrors pi-dcp community extensions):
 *
 * 1. DEDUPLICATION
 *    Tool calls with identical (toolName + JSON-sorted args) keep only
 *    the most recent result. Earlier ones are replaced with a tombstone.
 *    Saves thousands of tokens when the model re-reads the same file
 *    multiple times per session.
 *
 * 2. AUTO-COMPACT
 *    When estimated token usage exceeds COMPACT_THRESHOLD, the oldest
 *    messages (everything before the most recent KEEP_RECENT messages)
 *    are replaced with a single [COMPACTED HISTORY] summary block.
 *    This is a simple truncation-based compaction — no LLM call needed.
 *    A proper LLM-driven summarisation would be more faithful but would
 *    cost tokens and latency; this keeps things offline and zero-cost.
 *    Pi uses the same lazy approach: "late intervention, only near limit".
 */

import type { Message, ToolResultMessage, AssistantMessage } from './types.js';
import { logToFile } from '../utils/logger.js';

// ── Constants ─────────────────────────────────────────────────────────────────

/** Rough chars-per-token ratio for fast local estimation. */
const CHARS_PER_TOKEN = 4;

/**
 * Token threshold above which compaction fires.
 * 160k out of 200k — leaves 40k headroom for the current turn's
 * output + tool results.
 */
const COMPACT_THRESHOLD = 160_000;

/**
 * Number of most-recent messages to always keep verbatim after compaction.
 * Covers roughly the last 3–5 turns depending on tool call density.
 */
const KEEP_RECENT = 20;

/**
 * Tool results older than this many messages from the end are eligible
 * for deduplication tombstoning regardless of whether they are duplicates.
 * Prevents the first occurrence of a unique result from ballooning context.
 * Set to 0 to disable age-based truncation (dedup-only mode).
 */
const OLD_RESULT_AGE  = 40;

/** Max chars kept from an old (non-recent, non-duplicate) tool result. */
const OLD_RESULT_KEEP = 300;

// ── Public API ────────────────────────────────────────────────────────────────

/**
 * Main context transform — run before every LLM call.
 * Returns a new array; never mutates the input.
 */
export function transformContext(messages: Message[]): Message[] {
  let result = deduplicateToolResults(messages);
  result     = truncateOldToolResults(result);
  result     = compactIfNeeded(result);
  return result;
}

// ── Strategy 1: Deduplication ─────────────────────────────────────────────────

/**
 * For each (toolName, args) fingerprint, keep only the LAST occurrence.
 * Earlier occurrences have their content replaced with a tombstone.
 *
 * We look at AssistantMessage tool calls to build the fingerprint so we
 * can pair it with the following ToolResultMessage.
 */
function deduplicateToolResults(messages: Message[]): Message[] {
  // Build a map: fingerprint → index of the LAST tool result with that fingerprint.
  // We iterate forward to find pairs (assistantMsg with toolCall → toolResultMsg).
  const lastIndex = new Map<string, number>();

  for (let i = 0; i < messages.length; i++) {
    const msg = messages[i];
    if (msg.role !== 'toolResult') continue;
    const fp = toolResultFingerprint(msg as ToolResultMessage);
    lastIndex.set(fp, i);
  }

  // Second pass: tombstone everything that isn't the last occurrence.
  return messages.map((msg, i) => {
    if (msg.role !== 'toolResult') return msg;
    const r  = msg as ToolResultMessage;
    const fp = toolResultFingerprint(r);
    if (lastIndex.get(fp) !== i) {
      // Not the last — replace content with tombstone.
      return {
        ...r,
        content: [{ type: 'text' as const, text: `[duplicate ${r.toolName} result — kept latest only]` }],
      };
    }
    return msg;
  });
}

/**
 * Fingerprint = toolName + full content text.
 * Two calls to read_file("foo.ts") will produce identical content → same fp.
 */
function toolResultFingerprint(msg: ToolResultMessage): string {
  const content = msg.content.map(c => c.text).join('');
  return `${msg.toolName}::${content}`;
}

// ── Strategy 2: Truncate old unique tool results ──────────────────────────────

/**
 * Tool results that are older than OLD_RESULT_AGE positions from the end
 * and aren't already tombstoned get truncated to OLD_RESULT_KEEP chars.
 * This bounds the per-result contribution to context without losing the
 * signal that the tool was called and what it roughly returned.
 */
function truncateOldToolResults(messages: Message[]): Message[] {
  if (OLD_RESULT_AGE === 0) return messages;

  return messages.map((msg, i) => {
    if (msg.role !== 'toolResult') return msg;
    const age = messages.length - 1 - i;
    if (age < OLD_RESULT_AGE) return msg; // recent — keep verbatim

    const r    = msg as ToolResultMessage;
    const text = r.content.map(c => c.text).join('');

    // Already tombstoned by dedup pass — don't double-process.
    if (text.startsWith('[duplicate ')) return msg;

    if (text.length <= OLD_RESULT_KEEP) return msg; // short enough already

    return {
      ...r,
      content: [{
        type: 'text' as const,
        text: text.slice(0, OLD_RESULT_KEEP) + `\n[...truncated — ${text.length} chars total]`,
      }],
    };
  });
}

// ── Strategy 3: Compaction ───────────────────────────────────────────────────

/**
 * Estimate token count for the message array.
 * Rough heuristic: total chars / CHARS_PER_TOKEN.
 * No API call — purely local, runs in < 1ms.
 */
function estimateTokens(messages: Message[]): number {
  let chars = 0;
  for (const msg of messages) {
    if (msg.role === 'user')        chars += msg.content.length;
    if (msg.role === 'assistant')   chars += JSON.stringify(msg.content).length;
    if (msg.role === 'toolResult')  chars += msg.content.map(c => c.text).join('').length;
  }
  return Math.ceil(chars / CHARS_PER_TOKEN);
}

/**
 * If estimated tokens exceed COMPACT_THRESHOLD, replace everything
 * before the last KEEP_RECENT messages with a single compaction notice.
 *
 * The notice tells the model that history was truncated so it doesn't
 * confabulate about earlier events it can no longer see.
 */
function compactIfNeeded(messages: Message[]): Message[] {
  const estimated = estimateTokens(messages);
  if (estimated <= COMPACT_THRESHOLD) return messages;

  const cutoff = Math.max(0, messages.length - KEEP_RECENT);
  if (cutoff === 0) return messages; // everything is recent — can't compact

  const droppedCount  = cutoff;
  const droppedTokens = estimateTokens(messages.slice(0, cutoff));

  logToFile(`[context] compacting: ~${droppedTokens} tokens dropped (${droppedCount} messages), keeping last ${messages.length - cutoff}`);

  const notice: Message = {
    role:      'user',
    content:   `[COMPACTED HISTORY: ${droppedCount} earlier messages (~${droppedTokens} tokens) were removed to fit the context window. The conversation continues from this point. You may not recall specific file contents or commands from the removed history — re-read files as needed.]`,
    timestamp: Date.now(),
  };

  return [notice, ...messages.slice(cutoff)];
}