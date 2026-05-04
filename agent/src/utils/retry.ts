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

    // Check if this is a retryable error (rate limit or server error).
    const isRetryable = isRetryableError(err);

    // If not retryable, or retries exhausted — fail.
    if (!isRetryable || retries === 0) {
      if (attempt > 1) {
        emitEvent({ type: 'retry_result', success: false, attempt });
      }
      logToFile(`Error: ${(err as Error)?.message ?? JSON.stringify(err)}`);
      throw err;
    }

    // Retryable error — emit cooldown event and retry.
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
 * Determines if an error is a rate limit (429) or retryable server error.
 * Mirrors pi-mono's comprehensive retry detection pattern.
 */
function isRetryableError(err: unknown): boolean {
  const error = err as Record<string, unknown>;
  
  // Check for explicit 429 status code
  if (error.statusCode === 429 || error.code === 429) {
    return true;
  }

  // Extract error message for pattern matching
  const msg = (error.message ?? error.error ?? String(err)).toString().toLowerCase();
  
  // pi-mono's comprehensive patterns
  const retryablePatterns = [
    /rate\s?limit/i,
    /rate\s?increased/i,
    /too\s?many\s?requests/i,
    /overloaded/i,
    /provider\s?returned\s?error/i,
    /429/i,
    /500/i,
    /502/i,
    /503/i,
    /504/i,
    /service\s?unavailable/i,
    /server\s?error/i,
    /internal\s?error/i,
    /network\s?error/i,
    /connection\s?error/i,
    /connection\s?refused/i,
    /temporarily\s?unavailable/i,
    /upstream\s?error/i,
    /bad\s?gateway/i,
    /gateway\s?timeout/i,
  ];

  return retryablePatterns.some(pattern => pattern.test(msg));
}

// Legacy alias for backward compatibility
function isRateLimitError(err: unknown): boolean {
  return isRetryableError(err);
}