import { createInterface } from 'readline';

const rl = createInterface({ input: process.stdin });

function emit(payload: any) {
  process.stdout.write(JSON.stringify(payload) + '\n');
}

function sleep(ms: number) { return new Promise(r => setTimeout(r, ms)); }

rl.on('line', (line) => {
  try {
    if (!line.trim()) return;
    const cmd = JSON.parse(line);
    handleCommand(cmd);
  } catch (e) {
    emit({ type: 'error', message: 'Invalid JSON' });
  }
});

async function handleCommand(cmd: any) {
  const id = cmd.id;
  switch (cmd.type) {
    case 'get_state':
      emit({ 
        kind: 'response', 
        command: 'get_state', 
        id, 
        success: true, 
        data: { 
          model_name: 'mock-model', 
          model_limit: 100000, 
          is_streaming: false 
        } 
      });
      break;
    case 'prompt':
      emit({ kind: 'response', command: 'prompt', id, success: true });
      
      const content = (cmd.content || '').toLowerCase();
      
      emit({ type: 'turn_start' });
      
      if (content.includes('error')) {
        await sleep(200);
        emit({ type: 'error', message: 'Simulated LLM Error' });
      } else if (content.includes('tool')) {
        emit({ type: 'text_delta', delta: 'Running a tool... ' });
        await sleep(500);
        emit({ type: 'tool_call', name: 'run_command', input: '{"cmd":"ls"}' });
        await sleep(500);
        emit({ type: 'tool_update', tool_call_id: 'tool-1', delta: 'Listing files...\n' });
        await sleep(300);
        emit({ type: 'tool_result', name: 'run_command', output: 'file1.txt\nfile2.txt' });
        await sleep(200);
        emit({ type: 'text_delta', delta: '\nTool finished.' });
      } else {
        // Default Echo
        await sleep(200);
        emit({ type: 'text_delta', delta: `Mock echo: ${cmd.content}` });
        await sleep(100);
      }
      
      emit({ type: 'turn_end' });
      emit({ type: 'agent_end' });
      break;
    default:
      emit({ kind: 'response', command: cmd.type, id, success: false, error: 'Not implemented' });
  }
}
