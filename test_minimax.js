import { streamOpenRouter } from './agent/src/llm/stream.js';
import { tools } from './agent/src/tools/index.js';

async function run() {
  const context = {
    systemPrompt: "You are a helpful assistant.",
    messages: [{ role: 'user', content: 'What is 1+1?' }],
    tools: tools,
  };
  
  const stream = streamOpenRouter('minimax/minimax-m2y5:free', context, {
    temperature: 0.3,
    apiKey: ''
  });
  
  for await (const event of stream) {
    if (event.type === 'error') {
      console.error("ERROR:", event.error.errorMessage);
    } else if (event.type === 'text_delta') {
      process.stdout.write(event.delta);
    }
  }
}

run();
