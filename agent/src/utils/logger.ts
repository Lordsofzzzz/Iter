/**
 * File logging utility.
 *
 * Writes timestamped log messages to a file for debugging.
 * Useful for capturing stderr from child processes without
 * corrupting the TUI's alternate screen.
 */

import * as fs from 'fs';
import * as path from 'path';

// ============================================================================
// Configuration
// ============================================================================

const LOG_DIR = path.join(process.cwd(), 'agent', 'logs');
const LOG_FILE = path.join(LOG_DIR, 'tui.log');

// Ensure log directory exists on module load.
try {
  if (!fs.existsSync(LOG_DIR)) {
    fs.mkdirSync(LOG_DIR, { recursive: true });
  }
} catch {
  // Directory creation failed — logging will be no-op.
}

// ============================================================================
// Public API
// ============================================================================

/**
 * Appends a timestamped message to the log file.
 *
 * Silently fails if file write fails (non-blocking).
 */
export function logToFile(message: string): void {
  try {
    const timestamp = new Date().toISOString();
    const logLine = `[${timestamp}] ${message}\n`;
    fs.appendFileSync(LOG_FILE, logLine, 'utf8');
  } catch {
    // Silently ignore write errors.
  }
}
