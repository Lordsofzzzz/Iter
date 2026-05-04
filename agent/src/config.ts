/**
 * Configuration for Iter Coding Agent.
 *
 * Mirrors pi-mono's settings approach:
 *   - Environment variables override defaults
 *   - Optional config file (.iter-config.json) for project-specific settings
 *   - Deep merge for nested objects
 */

import { readFile, writeFile, mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { join, dirname } from 'path';
import { logToFile } from './utils/logger.js';
import { emitEvent } from './rpc.js';

// ============================================================================
// Types
// ============================================================================

export interface RetryConfig {
  enabled: boolean;
  maxRetries: number;
  baseDelayMs: number;
  maxDelayMs: number;
}

export interface CompactionConfig {
  enabled: boolean;
  reserveTokens: number;
  keepRecentTokens: number;
}

export interface Config {
  retry: RetryConfig;
  compaction: CompactionConfig;
  model: string;
  temperature: number;
  provider: string;
  timeoutMs: number;
}

// ============================================================================
// Defaults (pi-mono pattern)
// ============================================================================

const DEFAULT_CONFIG: Config = {
  retry: {
    enabled: true,
    maxRetries: 3,
    baseDelayMs: 2000,
    maxDelayMs: 60000,
  },
  compaction: {
    enabled: true,
    reserveTokens: 8000,
    keepRecentTokens: 12000,
  },
  model: 'minimax/minimax-m2.5:free',
  temperature: 0.3,
  provider: '', // auto-detect
  timeoutMs: 3600000,
};

// ============================================================================
// Active config (runtime)
// ============================================================================

let _config: Config | null = null;

// ============================================================================
// Public API
// ============================================================================

/**
 * Get the active configuration.
 * Loads from envvars and merges with defaults.
 */
export function getConfig(): Config {
  if (_config) return _config;

  const cfg = { ...DEFAULT_CONFIG };

  // Override from environment variables
  if (process.env.ITER_MODEL) cfg.model = process.env.ITER_MODEL;
  if (process.env.ITER_TEMPERATURE) {
    const temp = parseFloat(process.env.ITER_TEMPERATURE);
    if (!isNaN(temp)) cfg.temperature = temp;
  }
  if (process.env.ITER_PROVIDER) cfg.provider = process.env.ITER_PROVIDER;
  if (process.env.ITER_TIMEOUT_MS) {
    const timeout = parseInt(process.env.ITER_TIMEOUT_MS, 10);
    if (!isNaN(timeout)) cfg.timeoutMs = timeout;
  }

  // Retry overrides
  if (process.env.ITER_MAX_RETRIES) {
    const retries = parseInt(process.env.ITER_MAX_RETRIES, 10);
    if (!isNaN(retries)) cfg.retry.maxRetries = retries;
  }
  if (process.env.ITER_BASE_DELAY_MS) {
    const delay = parseInt(process.env.ITER_BASE_DELAY_MS, 10);
    if (!isNaN(delay)) cfg.retry.baseDelayMs = delay;
  }
  if (process.env.ITER_MAX_DELAY_MS) {
    const delay = parseInt(process.env.ITER_MAX_DELAY_MS, 10);
    if (!isNaN(delay)) cfg.retry.maxDelayMs = delay;
  }
  if (process.env.ITER_RETRY_DISABLED === '1' || process.env.ITER_RETRY_DISABLED === 'true') {
    cfg.retry.enabled = false;
  }

  // Compaction overrides
  if (process.env.ITER_COMPACTION_DISABLED === '1' || process.env.ITER_COMPACTION_DISABLED === 'true') {
    cfg.compaction.enabled = false;
  }
  if (process.env.ITER_RESERVE_TOKENS) {
    const tokens = parseInt(process.env.ITER_RESERVE_TOKENS, 10);
    if (!isNaN(tokens)) cfg.compaction.reserveTokens = tokens;
  }

  _config = cfg;
  return cfg;
}

/**
 * Reload configuration from disk.
 */
export function reloadConfig(): Config {
  _config = null;
  return getConfig();
}

/**
 * Get a specific setting.
 */
export function getRetryConfig(): RetryConfig {
  return getConfig().retry;
}

export function getCompactionConfig(): CompactionConfig {
  return getConfig().compaction;
}

export function getModel(): string {
  return getConfig().model;
}

export function getTemperature(): number {
  return getConfig().temperature;
}

export function getProvider(): string {
  return getConfig().provider;
}

export function getTimeoutMs(): number {
  return getConfig().timeoutMs;
}

/**
 * Check if retry is enabled.
 */
export function isRetryEnabled(): boolean {
  return getConfig().retry.enabled;
}

/**
 * Check if compaction is enabled.
 */
export function isCompactionEnabled(): boolean {
  return getConfig().compaction.enabled;
}

// ============================================================================
// Config file handling (optional)
// ============================================================================

const CONFIG_FILE_NAME = '.iter-config.json';

/**
 * Load config from optional project file.
 * File settings override defaults and envvars.
 */
export async function loadConfigFile(cwd: string): Promise<Config> {
  const configPath = join(cwd, CONFIG_FILE_NAME);
  
  if (!existsSync(configPath)) {
    return getConfig();
  }

  try {
    const content = await readFile(configPath, 'utf-8');
    const fileConfig = JSON.parse(content);
    
    // Deep merge: file overrides defaults
    const merged = deepMerge(DEFAULT_CONFIG, fileConfig);
    _config = merged;
    
    console.error(`[config] loaded from ${CONFIG_FILE_NAME}`);
    return merged;
  } catch (err) {
    console.error(`[config] failed to load ${CONFIG_FILE_NAME}: ${err}`);
    return getConfig();
  }
}

/**
 * Save current config to file.
 */
export async function saveConfigFile(cwd: string): Promise<void> {
  const configPath = join(cwd, CONFIG_FILE_NAME);
  const dir = dirname(configPath);
  
  try {
    await mkdir(dir, { recursive: true });
    await writeFile(configPath, JSON.stringify(getConfig(), null, 2) + '\n');
    console.error(`[config] saved to ${CONFIG_FILE_NAME}`);
  } catch (err) {
    console.error(`[config] failed to save: ${err}`);
  }
}

// ============================================================================
// Helpers
// ============================================================================

function deepMerge<T>(target: T, source: Partial<T>): T {
  const result = { ...target };
  
  for (const key of Object.keys(source)) {
    const sourceVal = (source as any)[key];
    const targetVal = (result as any)[key];
    
    if (sourceVal && typeof sourceVal === 'object' && !Array.isArray(sourceVal) &&
        targetVal && typeof targetVal === 'object' && !Array.isArray(targetVal)) {
      (result as any)[key] = deepMerge(targetVal, sourceVal);
    } else {
      (result as any)[key] = sourceVal;
    }
  }
  
  return result;
}

// Re-export retry for convenience
export { retry } from './utils/retry.js';
export { isRetryableError } from './utils/retry.js';