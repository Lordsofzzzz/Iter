export { LLMClient, MODEL_NAME, MODEL_LIMIT, MODEL_TEMP } from './client.js';
export { History } from './history.js';
export { Stats, type SessionStats } from './stats.js';

export function clearHistory(client: LLMClient): void {
  // Reset stats, history is already managed by client
}