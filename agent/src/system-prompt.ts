/**
 * Pi-style System Prompt Builder
 *
 * Minimal system prompt: one sentence role + cwd + os + git branch.
 * ~80 tokens flat, no bloat.
 */

import { execSync } from 'child_process';

/**
 * Builds the system prompt for the LLM.
 * Returns a minimal prompt with role, cwd, os, and git branch.
 */
export function buildSystemPrompt(): string {
  const cwd = process.cwd();
  const os = process.platform;
  const git = getGitBranch();

  return [
    `You are a coding agent. Read files, write files, execute commands.`,
    `cwd: ${cwd}`,
    `os: ${os}`,
    git ? `git: ${git}` : null,
  ].filter(Boolean).join('\n');
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