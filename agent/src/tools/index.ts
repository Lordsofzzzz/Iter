/**
 * Tool definitions — pi-style AgentTool interface.
 *
 * No dependency on the Vercel AI SDK.
 * Each tool implements AgentTool with an execute() that receives
 * (toolCallId, args, signal, onUpdate).
 *
 * run_command uses spawn() with live stdout streaming via onUpdate,
 * a rolling 1MB buffer, and overflow to a temp log file.
 */

import { readFile, writeFile, mkdir, readdir } from 'fs/promises';
import { dirname, join, relative }             from 'path';
import { spawn }                               from 'child_process';
import { createWriteStream }                   from 'fs';
import { tmpdir }                              from 'os';
import { randomBytes }                         from 'crypto';
import type { AgentTool, ToolResult, ToolSchema } from '../llm/types.js';

// ── Constants ─────────────────────────────────────────────────────────────────

const BLOCKED_COMMANDS  = ['rm -rf /', 'mkfs', 'dd if=', ':(){:|:&};:'];
const CMD_TIMEOUT_MS    = 30_000;
const MAX_OUTPUT_BYTES  = 1024 * 1024;
const MAX_FILE_BYTES    = 512  * 1024;
const TAIL_BYTES        = 200  * 1024;

// ── Schema helpers ────────────────────────────────────────────────────────────

function str(description: string) { return { type: 'string' as const, description }; }
function num(description: string) { return { type: 'number' as const, description }; }
function obj(properties: Record<string, unknown>, required: string[] = []): ToolSchema {
  return { type: 'object', properties: properties as ToolSchema['properties'], required };
}
function ok(text: string): ToolResult  { return { content: [{ type: 'text', text }] }; }
function err(text: string): ToolResult { return { content: [{ type: 'text', text }], isError: true }; }

// ── Tools ─────────────────────────────────────────────────────────────────────

export const tools: AgentTool[] = [

  {
    name: 'read_file', label: 'Read File',
    description: 'Read a file from disk. Large files are tail-truncated.',
    parameters: obj({ path: str('Path to file, relative to cwd') }, ['path']),
    async execute(_id, args) {
      const path = args.path as string;
      try {
        const buf = await readFile(path);
        if (buf.byteLength <= MAX_FILE_BYTES) return ok(buf.toString('utf-8'));
        const tail = buf.slice(buf.byteLength - TAIL_BYTES);
        return ok(`[... ${buf.byteLength - TAIL_BYTES} bytes truncated ...]\n${tail.toString('utf-8')}`);
      } catch (e) { return err(`ERROR: ${(e as Error)?.message ?? e}`); }
    },
  },

  {
    name: 'write_file', label: 'Write File',
    description: 'Write or overwrite a file. Creates parent directories if needed.',
    parameters: obj({ path: str('Path to write'), content: str('Full file content') }, ['path', 'content']),
    async execute(_id, args) {
      try {
        await mkdir(dirname(args.path as string), { recursive: true });
        await writeFile(args.path as string, args.content as string, 'utf-8');
        return ok(`OK: wrote ${args.path}`);
      } catch (e) { return err(`ERROR: ${(e as Error)?.message ?? e}`); }
    },
  },

  {
    name: 'run_command', label: 'Run Command',
    description: 'Run a shell command. Returns stdout+stderr. 30s timeout. Live output streamed. Large output truncated to log file.',
    parameters: obj({ cmd: str('Shell command'), cwd: str('Working directory (default: process cwd)') }, ['cmd']),
    async execute(_id, args, signal, onUpdate) {
      const cmd = args.cmd as string;
      const cwd = (args.cwd as string | undefined) ?? process.cwd();
      if (BLOCKED_COMMANDS.some(b => cmd.includes(b))) return err('BLOCKED: command not allowed');

      return new Promise<ToolResult>((resolve) => {
        const child = spawn(cmd, { cwd, shell: true, timeout: CMD_TIMEOUT_MS });
        const chunks: Buffer[] = [];
        let totalBytes = 0;
        let logPath: string | undefined;
        let logStream: ReturnType<typeof createWriteStream> | undefined;

        const handleData = (data: Buffer) => {
          onUpdate?.({ content: [{ type: 'text', text: data.toString('utf-8') }] });
          if (logStream) logStream.write(data);
          chunks.push(data);
          totalBytes += data.length;
          if (totalBytes > MAX_OUTPUT_BYTES && !logPath) {
            logPath   = join(tmpdir(), `iter-cmd-${randomBytes(8).toString('hex')}.log`);
            logStream = createWriteStream(logPath, { flags: 'w' });
            for (const c of chunks) logStream.write(c);
          }
          while (totalBytes > MAX_OUTPUT_BYTES && chunks.length > 1) {
            totalBytes -= chunks.shift()!.length;
          }
        };

        child.stdout?.on('data', handleData);
        child.stderr?.on('data', handleData);
        child.on('close', (code) => {
          if (logStream) logStream.end();
          let output = Buffer.concat(chunks).toString('utf-8').trimEnd() || '(no output)';
          if (logPath) output = `[... truncated, log: ${logPath} ...]\n${output}`;
          if (code !== null && code !== 0) output = `EXIT ${code}:\n${output}`;
          resolve(ok(output));
        });
        child.on('error', (e) => resolve(err(`ERROR: ${e.message}`)));
        signal?.addEventListener('abort', () => { child.kill(); resolve(err('ABORTED')); }, { once: true });
      });
    },
  },

  {
    name: 'list_files', label: 'List Files',
    description: 'List files in a directory recursively. Default depth 2, max 5.',
    parameters: obj({ path: str('Directory to list'), depth: { ...num('Max depth (default 2)'), minimum: 0, maximum: 5 } }, ['path']),
    async execute(_id, args) {
      try {
        const lines: string[] = [];
        await walk(args.path as string, args.path as string, (args.depth as number | undefined) ?? 2, lines);
        return ok(lines.join('\n') || '(empty directory)');
      } catch (e) { return err(`ERROR: ${(e as Error)?.message ?? e}`); }
    },
  },

  {
    name: 'search_files', label: 'Search Files',
    description: 'Search for a text pattern using grep. Returns up to 50 matches.',
    parameters: obj({
      pattern: str('Pattern to search for'),
      path:    str('Directory to search (default: cwd)'),
      glob:    str('File glob e.g. "*.ts"'),
    }, ['pattern']),
    async execute(_id, args, signal) {
      const grepArgs = ['-rn', '--color=never', '-E',
        '--exclude-dir=node_modules', '--exclude-dir=.git',
        '--exclude-dir=logs', '--exclude-dir=target', '--exclude-dir=dist',
        '--exclude=*.log', '--exclude=*.lock',
      ];
      if (args.glob) grepArgs.push(`--include=${args.glob}`);
      grepArgs.push('--', args.pattern as string, (args.path as string | undefined) ?? '.');
      return new Promise<ToolResult>((resolve) => {
        const child = spawn('grep', grepArgs, { timeout: 10_000 });
        const out: Buffer[] = [];
        child.stdout?.on('data', (d: Buffer) => out.push(d));
        child.stderr?.on('data', () => {});
        child.on('close', () => {
          const text = Buffer.concat(out).toString('utf-8').trim();
          resolve(ok(text ? text.split('\n').slice(0, 50).join('\n') : '(no matches)'));
        });
        child.on('error', () => resolve(ok('(no matches)')));
        signal?.addEventListener('abort', () => { child.kill(); resolve(err('ABORTED')); }, { once: true });
      });
    },
  },
];

// ── Helpers ───────────────────────────────────────────────────────────────────

const IGNORE_DIRS = new Set(['node_modules', '.git', 'target', 'dist', '.next', '.turbo']);

async function walk(root: string, dir: string, depth: number, out: string[]): Promise<void> {
  if (depth < 0) return;
  const entries = await readdir(dir, { withFileTypes: true });
  for (const e of entries) {
    if (IGNORE_DIRS.has(e.name)) continue;
    const rel = relative(root, join(dir, e.name));
    out.push(e.isDirectory() ? `${rel}/` : rel);
    if (e.isDirectory()) await walk(root, join(dir, e.name), depth - 1, out);
  }
}