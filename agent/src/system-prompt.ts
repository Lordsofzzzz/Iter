/**
 * Pi-style System Prompt Builder
 *
 * Minimal system prompt inspired by pi coding agent.
 * ~200 tokens. No bloat — frontier models already know how to code.
 */

import { execSync } from 'child_process';

/**
 * Builds the system prompt for the LLM.
 */
export function buildSystemPrompt(): string {
  const cwd = process.cwd();
  const os = process.platform;
  const git = getGitBranch();

  const context = [
    `cwd: ${cwd}`,
    `os: ${os}`,
    git ? `git: ${git}` : null,
  ].filter(Boolean).join('\n');

  return `You are Iter, a terminal coding agent.

Rules:
- Use run_command for navigation and search (ls, grep, find).
- read_file before editing — never guess file contents. Supports offset/limit for paging.
- Use edit for precise in-place changes (exact find-and-replace).
- Use write_file for new files or full rewrites.
- search_files before read_file when unsure where something is.
- Summarize actions in plain text. Do NOT use run_command to display what you did.
- Show file paths clearly when working with files.
- Be concise. No preamble, no filler.
- Before destructive ops (git push, delete, overwrite), confirm with the user.

${context}`;
}

/**
 * Gets the current git branch, or null if not in a git repo.
 */
function getGitBranch(): string | null {
  try {
    return execSync('git branch --show-current', {
      stdio: ['ignore', 'pipe', 'ignore'],
      timeout: 2000,
    }).toString().trim() || null;
  } catch {
    return null;
  }
}