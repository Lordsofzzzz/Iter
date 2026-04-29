/**
 * Retry utility with exponential backoff.
 *
 * Handles rate limiting (429) with automatic retry,
 * emits cooldown events to TUI, and properly handles abort signals.
 */

import { logToFile } from "./logger";
import { emitEvent } from "../rpc.js";

// ============================================================================
// Configuration
// ============================================================================

const DEFAULT_RETRIES = 5;
const INITIAL_DELAY_MS = 2000;

// ============================================================================
// Public API
// ============================================================================

/**
 * Retries a function with exponential backoff on rate limit errors.
 *
 * @param fn - Async function to execute
 * @param signal - AbortSignal for cancellation
 * @param retries - Number of retry attempts remaining
 * @param delay - Current delay in milliseconds
 * @param attempt - Current attempt number (for logging)
 */
export async function retry<T>(
  fn: () => Promise<T>,
  signal: AbortSignal,
  retries = DEFAULT_RETRIES,
  delay = INITIAL_DELAY_MS,
  attempt = 1,
): Promise<T> {
  try {
    const result = await fn();

    // Emit success event if this was a retry.
    if (attempt > 1) {
      emitEvent({ type: 'retry_result', success: true, attempt });
    }

    return result;

  } catch (err: unknown) {
    // Check for abort first — don't retry on abort.
    const isAbort = (err as Error)?.name === "AbortError" || signal.aborted;
    if (isAbort) {
      throw err;
    }

    // Check if this is a rate limit (429) error.
    const is429 = isRateLimitError(err);

    // If not a 429, or retries exhausted — fail.
    if (!is429 || retries === 0) {
      if (attempt > 1) {
        emitEvent({ type: 'retry_result', success: false, attempt });
      }
      logToFile(`Error: ${(err as Error)?.message ?? JSON.stringify(err)}`);
      throw err;
    }

    // Rate limited — emit cooldown event and retry.
    logToFile(`Retrying in ${delay}ms (${retries} left)`);
    emitEvent({ type: 'cooldown', wait_ms: delay, retries_left: retries });

    try {
      await abortableSleep(delay, signal);
    } catch {
      // Aborted during cooldown.
      throw new DOMException("Aborted", "AbortError");
    }

    // Emit turn start and recurse with exponential backoff.
    emitEvent({ type: 'turn_start' });
    return retry(fn, signal, retries - 1, delay * 2, attempt + 1);
  }
}

// ============================================================================
// Private Helpers
// ============================================================================

/**
 * Sleeps for the specified duration, respecting abort signals.
 */
function abortableSleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(new DOMException("Aborted", "AbortError"));
      return;
    }

    const timer = setTimeout(resolve, ms);

    signal.addEventListener("abort", () => {
      clearTimeout(timer);
      reject(new DOMException("Aborted", "AbortError"));
    }, { once: true });
  });
}

/**
 * Determines if an error is a rate limit (429) from various sources.
 */
function isRateLimitError(err: unknown): boolean {
  const error = err as Record<string, unknown>;
  return (
    error.statusCode === 429 ||
    error.code === 429 ||
    (typeof error.message === 'string' && error.message.includes('rate'))
  );
}