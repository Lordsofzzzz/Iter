export type Message = { role: 'user' | 'assistant'; content: string };

export class History {
  private messages: Message[] = [];

  get(): Message[] {
    return [...this.messages];
  }

  pushUser(content: string): void {
    this.messages.push({ role: 'user', content });
  }

  pushAssistant(content: string): void {
    this.messages.push({ role: 'assistant', content });
  }

  pop(): Message | undefined {
    return this.messages.pop();
  }

  clear(): void {
    this.messages.length = 0;
  }
}