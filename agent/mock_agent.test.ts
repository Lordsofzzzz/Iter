import { spawn } from 'child_process';
import { describe, expect, test } from 'bun:test';

function sendCommand(cmd: object): Promise<string[]> {
  return new Promise((resolve) => {
    const lines: string[] = [];
    const child = spawn('bun', ['run', 'agent/mock_agent.ts'], {
      stdio: ['pipe', 'pipe', 'pipe'],
    });

    child.stdout.on('data', (data) => {
      lines.push(...data.toString().trim().split('\n').filter(Boolean));
    });

    child.on('close', () => resolve(lines));

    child.stdin.write(JSON.stringify(cmd) + '\n');
    child.stdin.end();
  });
}

describe('mock_agent', () => {
  test('get_state returns mock model', async () => {
    const lines = await sendCommand({ type: 'get_state', id: '1' });
    const response = JSON.parse(lines[0]);
    expect(response.success).toBe(true);
    expect(response.data.model_name).toBe('mock-model');
  });

  test('echo returns text delta', async () => {
    const lines = await sendCommand({ type: 'prompt', content: 'hello', id: '2' });
    const events = lines.map((l) => JSON.parse(l));
    const textDelta = events.find((e) => e.type === 'text_delta');
    expect(textDelta.delta).toContain('Mock echo: hello');
  });

  test('error trigger emits error', async () => {
    const lines = await sendCommand({ type: 'prompt', content: 'trigger error', id: '3' });
    const events = lines.map((l) => JSON.parse(l));
    const errorEvent = events.find((e) => e.type === 'error');
    expect(errorEvent.message).toBe('Simulated LLM Error');
  });

  test('tool trigger emits tool events', async () => {
    const lines = await sendCommand({ type: 'prompt', content: 'run tool', id: '4' });
    const events = lines.map((l) => JSON.parse(l));
    const toolCall = events.find((e) => e.type === 'tool_call');
    const toolResult = events.find((e) => e.type === 'tool_result');
    expect(toolCall.name).toBe('run_command');
    expect(toolResult.name).toBe('run_command');
  });
});