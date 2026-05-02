import { createInterface } from 'readline';

const rl = createInterface({ input: process.stdin });

function emit(payload: any) {
  process.stdout.write(JSON.stringify(payload) + '\n');
}

rl.on('line', (line) => {
  try {
    if (!line.trim()) return;
    const cmd = JSON.parse(line);
    handleCommand(cmd);
  } catch (e) {
    emit({ type: 'error', message: 'Invalid JSON' });
  }
});

function handleCommand(cmd: any) {
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
      // Logic for triggers goes here in Task 2
      break;
    default:
      emit({ kind: 'response', command: cmd.type, id, success: false, error: 'Not implemented' });
  }
}
