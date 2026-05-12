/**
 * LLM module exports.
 *
 * Re-exports core LLM functionality for external use.
 */

import { LLMClient } from './client.js';
export { LLMClient, MODEL_NAME, MODEL_TEMP, getModelLimit, fetchModelLimits, setModel } from './client.js';
export { Stats, type SessionStats } from './stats.js';
export { streamLLM } from './stream.js';
export { parseProviderError, isGenericRetryable, isContextOverflow, type ProviderError } from './provider-error.js';

/**
 * Clears conversation history.
 * @param client - The LLM client to clear
 */
export function clearHistory(client: LLMClient): void {
  client.clearHistory();
}