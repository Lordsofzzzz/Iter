/**
 * LLM module exports.
 *
 * Re-exports core LLM functionality for external use.
 */

import { LLMClient } from './client.js';
export { LLMClient } from './client.js';
export { getModelLimit, setModelLimit } from './client.js';
export { Stats, type SessionStats } from './stats.js';
export { streamLLM } from './stream.js';
export { parseProviderError, isGenericRetryable, isContextOverflow, type ProviderError } from './provider-error.js';
export { listProviders, setActiveProvider, getActiveProvider, PROVIDERS } from './provider.js';
export { createModel, listModels, clearModelCache } from './model-factory.js';

/**
 * Clears conversation history.
 * @param client - The LLM client to clear
 */
export function clearHistory(client: LLMClient): void {
  client.clearHistory();
}