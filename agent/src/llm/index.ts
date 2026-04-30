/**
 * LLM module exports.
 *
 * Re-exports core LLM functionality for external use.
 */

export { LLMClient, MODEL_NAME, MODEL_LIMIT, MODEL_TEMP, setModel } from './client.js';
export { History } from './history.js';
export { Stats, type SessionStats } from './stats.js';

/**
 * Clears conversation history.
 * @param client - The LLM client to clear
 */
export function clearHistory(client: LLMClient): void {
  client.clearHistory();
}