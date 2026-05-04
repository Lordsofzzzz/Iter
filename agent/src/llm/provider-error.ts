/**
 * Provider-specific error handling utilities.
 *
 * Each provider has different error formats and rate limit headers.
 * This module provides unified error parsing per provider.
 */

import type { ApiType } from './provider.js';

// ============================================================================
// Types
// ============================================================================

export interface ProviderError {
  message: string;
  statusCode: number;
  isRetryable: boolean;
  retryAfterMs?: number;
}

/**
 * Parse an HTTP error response into a structured ProviderError.
 * Different providers have different error formats.
 */
export function parseProviderError(
  response: Response,
  body: string,
  apiType: ApiType,
): ProviderError {
  const status = response.status;
  
  // Try to parse JSON error body
  let json: { error?: { message?: string; type?: string; code?: string | number; status?: string } } | null = null;
  try {
    json = JSON.parse(body) as typeof json;
  } catch {
    // Not JSON
  }

  // Provider-specific parsing
  switch (apiType) {
    case 'anthropic-messages':
      return parseAnthropicError(status, json);
    case 'google-generative-ai':
      return parseGoogleError(status, json);
    case 'openai-completions':
    default:
      return parseOpenAIError(status, json, response);
  }
}

// ============================================================================
// Anthropic
// ============================================================================

function parseAnthropicError(status: number, json: { error?: { message?: string; type?: string } } | null): ProviderError {
  // Anthropic error format: { "error": { "type": "rate_limit_error", "message": "..." } }
  const errorObj = json?.error;
  const message = errorObj?.message 
    ?? errorObj?.type 
    ?? `HTTP ${status}`;
  
  const isRetryable = isAnthropicRetryable(status, errorObj?.type);
  
  return {
    message: String(message),
    statusCode: status,
    isRetryable,
  };
}

function isAnthropicRetryable(status: number, errorType: string | undefined): boolean {
  if (status === 429) return true;
  if (status >= 500) return true;
  
  if (errorType) {
    const retryableTypes = [
      'rate_limit_error',
      'overloaded_error',
      'internal_error',
      'server_error',
      'service_unavailable',
    ];
    return retryableTypes.some(t => errorType.toLowerCase().includes(t));
  }
  
  return false;
}

// ============================================================================
// Google Gemini
// ============================================================================

function parseGoogleError(status: number, json: { error?: { message?: string; status?: string; code?: string | number } } | null): ProviderError {
  // Google error format: { "error": { "code": 429, "message": "...", "status": "RESOURCE_EXHAUSTED" } }
  const errorObj = json?.error;
  const message = errorObj?.message 
    ?? errorObj?.status 
    ?? errorObj?.code 
    ?? `HTTP ${status}`;
  
  const isRetryable = isGoogleRetryable(status, errorObj?.status);
  
  return {
    message: String(message),
    statusCode: status,
    isRetryable,
  };
}

function isGoogleRetryable(status: number, errorStatus: string | undefined): boolean {
  if (status === 429) return true;
  if (status >= 500) return true;
  
  const retryableStatuses = [
    'RESOURCE_EXHAUSTED',
    'UNAVAILABLE',
    'INTERNAL',
    'DEADLINE_EXCEEDED',
  ];
  
  if (errorStatus && retryableStatuses.includes(errorStatus)) {
    return true;
  }
  
  return false;
}

// ============================================================================
// OpenAI / OpenRouter / Compatible
// ============================================================================

function parseOpenAIError(
  status: number, 
  json: { error?: { message?: string; type?: string; code?: string | number } } | null,
  response: Response,
): ProviderError {
  // OpenAI/OpenRouter error format: { "error": { "message": "...", "type": "server_error", "code": "rate_limit_error" } }
  const errorObj = json?.error;
  const message = errorObj?.message 
    ?? errorObj?.type 
    ?? errorObj?.code 
    ?? `HTTP ${status}`;
  
  const isRetryable = isOpenAIRetryable(status, errorObj?.type, errorObj?.code);
  
  // Check for Retry-After header (OpenRouter supports this)
  let retryAfterMs: number | undefined;
  const retryAfter = response.headers.get('retry-after');
  if (retryAfter) {
    const seconds = parseInt(retryAfter, 10);
    if (!isNaN(seconds)) {
      retryAfterMs = seconds * 1000;
    }
  }
  
  return {
    message: String(message),
    statusCode: status,
    isRetryable,
    retryAfterMs,
  };
}

function isOpenAIRetryable(status: number, errorType: string | undefined, errorCode: string | number | undefined): boolean {
  if (status === 429) return true;
  if (status >= 500) return true;
  
  const retryableTypes = [
    'rate_limit_error',
    'overloaded_error', 
    'server_error',
    'internal_error',
    'service_unavailable',
    'insufficient_quota',
  ];
  
  const retryableCodes = [
    'rate_limit_exceeded',
    'model_quota_exceeded',
    'insufficient_quota',
  ];
  
  if (errorType && retryableTypes.some(t => errorType.toLowerCase().includes(t))) {
    return true;
  }
  if (errorCode && typeof errorCode === 'string' && retryableCodes.some(c => errorCode.toLowerCase().includes(c))) {
    return true;
  }
  
  return false;
}

// ============================================================================
// Generic retryable check (fallback)
// ============================================================================

/**
 * Generic check if an error is retryable.
 * Matches pi-mono patterns from agent-session.ts _isRetryableError().
 */
export function isGenericRetryable(status: number, message: string): boolean {
  // Explicit status codes
  if (status === 429 || status === 500 || status === 502 || status === 503 || status === 504) {
    return true;
  }

  // Match pi-mono patterns (overloaded, rate limit, network/connection errors, etc.)
  const msg = message.toLowerCase();
  const patterns = [
    'overloaded',
    'provider returned error',
    'rate limit',
    'too many requests',
    '429',
    '500',
    '502',
    '503',
    '504',
    'service unavailable',
    'server error',
    'internal error',
    'network error',
    'connection error',
    'connection refused',
    'connection lost',
    'websocket closed',
    'websocket error',
    'other side closed',
    'fetch failed',
    'upstream connect',
    'reset before headers',
    'socket hang up',
    'ended without',
    'http2 request did not get a response',
    'timed out',
    'timeout',
    'terminated',
    'retry delay',
  ];

  return patterns.some(p => msg.includes(p));
}

// ============================================================================
// Context overflow detection (NOT retryable)
// ============================================================================

/**
 * Check if error is a context overflow error.
 * Context overflow is handled by compaction, NOT retry.
 * Matches pi-mono's isContextOverflow() logic.
 */
export function isContextOverflow(message: string, contextWindow: number): boolean {
  const msg = message.toLowerCase();
  const patterns = [
    'context length',
    'context window',
    'max tokens',
    'too many tokens',
    'exceeds context',
    'context limit',
    'token limit',
    'token limit exceeded',
    'input too long',
    'request too long',
    'maximum context',
    'context overflow',
  ];

  return patterns.some(p => msg.includes(p));
}
