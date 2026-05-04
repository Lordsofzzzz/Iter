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
const CMD_TIMEOUT_MS    = undefined;

// read_file limits
const MAX_FILE_BYTES    = 10 * 1024;
const MAX_FILE_LINES    = 200;
const DEFAULT_READ_LIMIT = 200;

// search_files limits (decoupled from read_file)
const MAX_SEARCH_BYTES  = 20 * 1024;
const MAX_SEARCH_MATCHES = 100;

// run_command limits
const MAX_OUTPUT_BYTES = 20 * 1024;
const MAX_OUTPUT_LINES = 500;

// list_files limits
const MAX_ENTRIES    = 200;
const DEFAULT_DEPTH  = 1;

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
    description: `Read a file. Default: first ${DEFAULT_READ_LIMIT} lines. Use offset+limit to page through large files. Prefer targeted reads over reading whole files.`,
    parameters: obj({
      path:   str('Path to file, relative to cwd'),
      offset: { ...num('Line to start reading from (1-indexed)'), minimum: 1 },
      limit:  { ...num('Max lines to read (default: 200)') },
    }, ['path']),
    async execute(_id, args) {
      const path = args.path as string;
      const offset = ((args.offset as number | undefined) ?? 1) - 1;
      const userLimit = (args.limit as number | undefined) ?? DEFAULT_READ_LIMIT;
      try {
        const text = await readFile(path, 'utf-8');
        const lines = text.split('\n');
        const slice = lines.slice(offset, offset + userLimit);
        let out = '';
        let bytes = 0;
        let lineCount = 0;
        for (const line of slice) {
          const lineBytes = Buffer.byteLength(line + '\n');
          if (lineCount >= userLimit || bytes + lineBytes > MAX_FILE_BYTES) {
            const remaining = lines.length - offset - lineCount;
            if (remaining > 0) {
              const nextOffset = offset + lineCount + 1;
              out += `\n[${remaining} more lines. Use offset=${nextOffset} to continue.]`;
            }
            break;
          }
          out += line + '\n';
          bytes += lineBytes;
          lineCount++;
        }
        return ok(out);
      } catch (e) { return err(`ERROR: ${(e as Error)?.message ?? e}`); }
    },
  },

  {
    name: 'write_file', label: 'Write File',
    description: 'Write or overwrite a file. Creates parent directories if needed. Use write_file for new files or full rewrites. Use edit for precise in-place changes.',
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
    name: 'edit', label: 'Edit File',
    description:
      'Replace an exact block of text in a file. ' +
      'old_text must match the file content exactly (including whitespace and indentation). ' +
      'Reads the file first if unsure of exact content. ' +
      'Fails if old_text is not found or matches more than once.',
    parameters: obj({
      path:     str('Path to file, relative to cwd'),
      old_text: str('Exact text to replace. Must match file content exactly.'),
      new_text: str('Replacement text.'),
    }, ['path', 'old_text', 'new_text']),
    async execute(_id, args) {
      const path    = args.path     as string;
      const oldText = args.old_text as string;
      const newText = args.new_text as string;

      try {
        const content = await readFile(path, 'utf-8');

        const occurrences = content.split(oldText).length - 1;
        if (occurrences === 0) {
          return err(
            `ERROR: old_text not found in ${path}.\n` +
            `Use read_file to verify the exact content before editing.`
          );
        }
        if (occurrences > 1) {
          return err(
            `ERROR: old_text found ${occurrences} times in ${path}. ` +
            `Make old_text more specific so it matches exactly once.`
          );
        }

        const updated = content.replace(oldText, newText);
        await writeFile(path, updated, 'utf-8');
        return ok(`OK: edited ${path}`);
      } catch (e) { return err(`ERROR: ${(e as Error)?.message ?? e}`); }
    },
  },

  {
    name: 'run_command', label: 'Run Command',
    description: `Run a shell command. Output capped at ${MAX_OUTPUT_LINES} lines / ${MAX_OUTPUT_BYTES/1024}KB. Large output truncated to log file.`,
    parameters: obj({ cmd: str('Shell command'), cwd: str('Working directory (default: process cwd)') }, ['cmd']),
    async execute(_id, args, signal, onUpdate) {
      const cmd = args.cmd as string;
      const cwd = (args.cwd as string | undefined) ?? process.cwd();
      if (BLOCKED_COMMANDS.some(b => cmd.includes(b))) return err('BLOCKED: command not allowed');

      return new Promise<ToolResult>((resolve) => {
        const child = spawn(cmd, { cwd, shell: true, timeout: CMD_TIMEOUT_MS });
        const chunks: Buffer[] = [];
        let totalBytes = 0;
        let totalLines = 0;
        let logPath: string | undefined;
        let logStream: ReturnType<typeof createWriteStream> | undefined;

        const handleData = (data: Buffer) => {
          const text = data.toString('utf-8');
          onUpdate?.({ content: [{ type: 'text', text }] });
          if (logStream) logStream.write(data);
          chunks.push(data);
          totalBytes += data.length;
          totalLines += text.split('\n').length - 1;
          // Start logging to file if either limit exceeded
          if ((totalBytes > MAX_OUTPUT_BYTES || totalLines > MAX_OUTPUT_LINES) && !logPath) {
            logPath   = join(tmpdir(), `iter-cmd-${randomBytes(8).toString('hex')}.log`);
            logStream = createWriteStream(logPath, { flags: 'w' });
            for (const c of chunks) logStream.write(c);
          }
          // Trim from middle to keep rolling buffer small
          while (totalBytes > MAX_OUTPUT_BYTES && chunks.length > 1) {
            totalBytes -= chunks.shift()!.length;
          }
          while (totalLines > MAX_OUTPUT_LINES && chunks.length > 1) {
            const removed = chunks.shift()!;
            totalBytes -= removed.length;
            totalLines -= removed.toString('utf-8').split('\n').length - 1;
          }
        };

        child.stdout?.on('data', handleData);
        child.stderr?.on('data', handleData);
        child.on('close', (code) => {
          if (logStream) logStream.end();
          let output = Buffer.concat(chunks).toString('utf-8').trimEnd() || '(no output)';
          // Trim from middle if still over limits (keep start + tail for errors)
          const lines = output.split('\n');
          if (lines.length > MAX_OUTPUT_LINES) {
            const keep = Math.floor(MAX_OUTPUT_LINES / 2);
            output = lines.slice(0, keep).join('\n') + '\n... [truncated] ...\n' + lines.slice(-keep).join('\n');
          }
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
    description: `List files in a directory. Default depth ${DEFAULT_DEPTH}, max depth 3. Cap at ${MAX_ENTRIES} entries.`,
    parameters: obj({ path: str('Directory to list (default: cwd)'), depth: { ...num(`Max depth (default ${DEFAULT_DEPTH})`), minimum: 0, maximum: 3 } }, []),
    async execute(_id, args) {
      try {
        const dirPath = (args.path as string | undefined) ?? process.cwd();
        const lines: string[] = [];
        await walk(dirPath, dirPath, (args.depth as number | undefined) ?? DEFAULT_DEPTH, lines, MAX_ENTRIES);
        if (lines.length >= MAX_ENTRIES) lines.push(`[Truncated at ${MAX_ENTRIES} entries]`);
        return ok(lines.join('\n') || '(empty directory)');
      } catch (e) { return err(`ERROR: ${(e as Error)?.message ?? e}`); }
    },
  },

  {
    name: 'search_files', label: 'Search Files',
    description: `Search for a text pattern using grep. Max ${MAX_SEARCH_MATCHES} matches. Truncate at ${MAX_SEARCH_BYTES/1024}KB.`,
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
          if (!text || text === '') return resolve(ok('(no matches)'));
          let result = text;
          const lines = result.split('\n');
          if (lines.length > MAX_SEARCH_MATCHES) {
            result = lines.slice(0, MAX_SEARCH_MATCHES).join('\n');
            result += `\n[${lines.length - MAX_SEARCH_MATCHES} more matches. Refine pattern.]`;
          } else if (Buffer.byteLength(text) > MAX_SEARCH_BYTES) {
            result = Buffer.from(text).slice(0, MAX_SEARCH_BYTES).toString('utf-8');
            const lastNewline = result.lastIndexOf('\n');
            result = result.slice(0, lastNewline) + `\n[Truncated at ${MAX_SEARCH_BYTES/1024}KB. Refine pattern.]`;
          }
          resolve(ok(result));
        });
        child.on('error', () => resolve(ok('(no matches)')));
        signal?.addEventListener('abort', () => { child.kill(); resolve(err('ABORTED')); }, { once: true });
      });
    },
  },
];

// ── Helpers ───────────────────────────────────────────────────────────────────

const IGNORE_DIRS = new Set(['node_modules', '.git', 'target', 'dist', '.next', '.turbo']);

async function walk(root: string, dir: string, depth: number, out: string[], maxEntries: number): Promise<void> {
  if (depth < 0 || out.length >= maxEntries) return;
  const entries = await readdir(dir, { withFileTypes: true });
  for (const e of entries) {
    if (out.length >= maxEntries) break;
    if (IGNORE_DIRS.has(e.name)) continue;
    const rel = relative(root, join(dir, e.name));
    out.push(e.isDirectory() ? `${rel}/` : rel);
    if (e.isDirectory()) await walk(root, join(dir, e.name), depth - 1, out, maxEntries);
  }
}