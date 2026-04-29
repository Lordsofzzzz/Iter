/**
 * Conversation history manager.
 *
 * Maintains the message history for LLM context window.
 * Handles adding user/assistant messages and cleanup.
 */

/** A single message in the conversation. */
export type Message = { role: 'user' | 'assistant'; content: string };

/** Manages conversation history for LLM context. */
export class History {
  private messages: Message[] = [];

  /**
   * Returns a copy of all messages (for API calls).
   */
  get(): Message[] {
    return [...this.messages];
  }

  /**
   * Adds a user message to history.
   */
  pushUser(content: string): void {
    this.messages.push({ role: 'user', content });
  }

  /**
   * Adds an assistant message to history.
   */
  pushAssistant(content: string): void {
    this.messages.push({ role: 'assistant', content });
  }

  /**
   * Removes the last message (used on error to allow retry).
   */
  pop(): Message | undefined {
    return this.messages.pop();
  }

  /**
   * Clears all messages.
   */
  clear(): void {
    this.messages.length = 0;
  }
}