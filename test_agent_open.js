import { spawn } from 'child_process';

const child = spawn('bun', ['run', 'src/index.ts'], {
  cwd: 'agent',
  stdio: ['pipe', 'inherit', 'inherit']
});

child.stdin.write('{"type":"prompt","content":"Hello!"}\n');

// don't close stdin so process stays alive
