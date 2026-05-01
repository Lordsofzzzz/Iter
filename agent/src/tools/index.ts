/**
 * Tool system for the agent.
 *
 * Provides file system operations, shell command execution,
 * and file search capabilities.
 */

import { tool } from 'ai';
import { z } from 'zod';
import { readFile, writeFile, mkdir, readdir, stat } from 'fs/promises';
import { dirname, join, relative } from 'path';
import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

const BLOCKED_COMMANDS = ['rm -rf /', 'mkfs', 'dd if=', ':(){:|:&};:'];
const CMD_TIMEOUT_MS = 30_000;

export const tools = {
  read_file: tool({
    description: 'Read a file from disk. Returns file contents as string.',
    parameters: z.object({
      path: z.string().describe('Path to file, relative to cwd'),
    }),
    execute: async ({ path }) => {
      try {
        return await readFile(path, 'utf-8');
      } catch (e) {
        return `ERROR: ${e instanceof Error ? e.message : String(e)}`;
      }
    },
  }),

  write_file: tool({
    description: 'Write or overwrite a file. Creates parent dirs if needed.',
    parameters: z.object({
      path:    z.string().describe('Path to write, relative to cwd'),
      content: z.string().describe('Full file content'),
    }),
    execute: async ({ path, content }) => {
      try {
        await mkdir(dirname(path), { recursive: true });
        await writeFile(path, content, 'utf-8');
        return `OK: wrote ${path}`;
      } catch (e) {
        return `ERROR: ${e instanceof Error ? e.message : String(e)}`;
      }
    },
  }),

  run_command: tool({
    description: 'Run a shell command. Returns stdout+stderr. 30s timeout.',
    parameters: z.object({
      cmd: z.string().describe('Shell command to execute'),
      cwd: z.string().optional().describe('Working directory (default: process.cwd())'),
    }),
    execute: async ({ cmd, cwd }) => {
      // Safety: block destructive patterns
      const blocked = BLOCKED_COMMANDS.some(b => cmd.includes(b));
      if (blocked) return `BLOCKED: command not allowed`;

      try {
        const { stdout, stderr } = await execAsync(cmd, {
          cwd:     cwd ?? process.cwd(),
          timeout: CMD_TIMEOUT_MS,
          maxBuffer: 1024 * 1024, // 1MB
        });
        const out = [stdout, stderr].filter(Boolean).join('\n').trim();
        return out || '(no output)';
      } catch (e: any) {
        // exec throws on non-zero exit — still return output
        const out = [e.stdout, e.stderr].filter(Boolean).join('\n').trim();
        return out ? `EXIT ${e.code}:\n${out}` : `ERROR: ${e.message}`;
      }
    },
  }),

  list_files: tool({
    description: 'List files in a directory recursively (max depth 3).',
    parameters: z.object({
      path:  z.string().describe('Directory path'),
      depth: z.number().optional().describe('Max depth (default 2)'),
    }),
    execute: async ({ path, depth = 2 }) => {
      try {
        const lines: string[] = [];
        await walk(path, path, depth, lines);
        return lines.join('\n') || '(empty)';
      } catch (e) {
        return `ERROR: ${e instanceof Error ? e.message : String(e)}`;
      }
    },
  }),

  search_files: tool({
    description: 'Search for a pattern in files using grep.',
    parameters: z.object({
      pattern: z.string().describe('Regex pattern to search'),
      path:    z.string().optional().describe('Directory to search (default: cwd)'),
      glob:    z.string().optional().describe('File glob e.g. "*.ts"'),
    }),
    execute: async ({ pattern, path = '.', glob }) => {
      const include = glob ? `--include="${glob}"` : '';
      const cmd = `grep -rn ${include} --color=never -E "${pattern}" "${path}" 2>/dev/null | head -50`;
      try {
        const { stdout } = await execAsync(cmd, { timeout: 10_000 });
        return stdout.trim() || '(no matches)';
      } catch {
        return '(no matches)';
      }
    },
  }),
};

// ── helpers ──────────────────────────────────────────────────────────────────

async function walk(
  root:  string,
  dir:   string,
  depth: number,
  out:   string[],
): Promise<void> {
  if (depth < 0) return;
  const entries = await readdir(dir, {withFileTypes: true});
  const IGNORE = new Set(['node_modules', '.git', 'target', 'dist', '.next']);

  for (const e of entries) {
    if (IGNORE.has(e.name)) continue;
    const rel = relative(root, join(dir, e.name));
    out.push(e.isDirectory() ? `${rel}/` : rel);
    if (e.isDirectory()) await walk(root, join(dir, e.name), depth - 1, out);
  }
}
