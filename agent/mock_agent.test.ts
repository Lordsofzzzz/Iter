import { spawnSync } from 'child_process';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { describe, expect, test } from 'bun:test';

function sendCommand(cmd: object): string[] {
  const agentDir = dirname(fileURLToPath(import.meta.url));
  const result = spawnSync('bun', ['run', join(agentDir, 'mock_agent.ts')], {
    cwd: agentDir,
    input: JSON.stringify(cmd) + '\n',
    encoding: 'utf8',
  });
  if (result.status !== 0) {
    throw new Error(result.stderr || `mock agent exited ${result.status}`);
  }
  return result.stdout.trim().split('\n').filter(Boolean);
}

describe('mock_agent', () => {
  test('get_state returns mock model', () => {
    const lines = sendCommand({ type: 'get_state', id: '1' });
    const response = JSON.parse(lines[0]);
    expect(response.success).toBe(true);
    expect(response.data.model_name).toBe('mock-model');
  });

  test('echo returns text delta', () => {
    const lines = sendCommand({ type: 'prompt', content: 'hello', id: '2' });
    const events = lines.map((l) => JSON.parse(l));
    const textDelta = events.find((e) => e.type === 'text_delta');
    const agentEnd = events.find((e) => e.type === 'agent_end');
    expect(textDelta.delta).toContain('Mock echo: hello');
    expect(agentEnd).toMatchObject({ id: '2', success: true });
  });

  test('error trigger emits error', () => {
    const lines = sendCommand({ type: 'prompt', content: 'trigger error', id: '3' });
    const events = lines.map((l) => JSON.parse(l));
    const errorEvent = events.find((e) => e.type === 'error');
    const agentEnd = events.find((e) => e.type === 'agent_end');
    expect(errorEvent.message).toBe('Simulated LLM Error');
    expect(agentEnd).toMatchObject({ id: '3', success: false });
  });

  test('tool trigger emits tool events', () => {
    const lines = sendCommand({ type: 'prompt', content: 'run tool', id: '4' });
    const events = lines.map((l) => JSON.parse(l));
    const toolCall = events.find((e) => e.type === 'tool_call');
    const toolResult = events.find((e) => e.type === 'tool_result');
    expect(toolCall.name).toBe('run_command');
    expect(toolResult.name).toBe('run_command');
  });
});
