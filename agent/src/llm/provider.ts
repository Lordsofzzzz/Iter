/**
 * Provider configuration.
 *
 * Mirrors pi-mono's provider/API approach:
 *   - "openai-completions"    → OpenAI, OpenRouter, DeepSeek, Groq, Mistral (compat), etc.
 *   - "anthropic-messages"    → Anthropic direct API
 *   - "google-generative-ai"  → Google Gemini
 *
 * Auto-detect active provider from environment variables if not explicitly set.
 */

export type ApiType =
  | 'openai-completions'
  | 'anthropic-messages'
  | 'google-generative-ai';

export interface ProviderConfig {
  id:      string;
  name:    string;
  baseUrl: string;
  apiType: ApiType;
  /** Environment variable name OR literal key value */
  apiKeyEnv: string;
  /** Model prefix used in models.dev / OpenRouter-style IDs */
  modelPrefix?: string;
}

// ── Built-in providers ────────────────────────────────────────────────────────

export const PROVIDERS: ProviderConfig[] = [
  {
    id:         'openrouter',
    name:       'OpenRouter',
    baseUrl:    'https://openrouter.ai/api/v1',
    apiType:    'openai-completions',
    apiKeyEnv:  'OPENROUTER_API_KEY',
  },
  {
    id:         'anthropic',
    name:       'Anthropic',
    baseUrl:    'https://api.anthropic.com/v1',
    apiType:    'anthropic-messages',
    apiKeyEnv:  'ANTHROPIC_API_KEY',
  },
  {
    id:         'openai',
    name:       'OpenAI',
    baseUrl:    'https://api.openai.com/v1',
    apiType:    'openai-completions',
    apiKeyEnv:  'OPENAI_API_KEY',
  },
  {
    id:         'google',
    name:       'Google Gemini',
    baseUrl:    'https://generativelanguage.googleapis.com/v1beta',
    apiType:    'google-generative-ai',
    apiKeyEnv:  'GOOGLE_API_KEY',
  },
  {
    id:         'deepseek',
    name:       'DeepSeek',
    baseUrl:    'https://api.deepseek.com/v1',
    apiType:    'openai-completions',
    apiKeyEnv:  'DEEPSEEK_API_KEY',
  },
  {
    id:         'groq',
    name:       'Groq',
    baseUrl:    'https://api.groq.com/openai/v1',
    apiType:    'openai-completions',
    apiKeyEnv:  'GROQ_API_KEY',
  },
  {
    id:         'mistral',
    name:       'Mistral',
    baseUrl:    'https://api.mistral.ai/v1',
    apiType:    'openai-completions',
    apiKeyEnv:  'MISTRAL_API_KEY',
  },
  {
    id:         'ollama',
    name:       'Ollama (local)',
    baseUrl:    'http://localhost:11434/v1',
    apiType:    'openai-completions',
    apiKeyEnv:  'OLLAMA_API_KEY',
  },
];

// ── Active provider state ─────────────────────────────────────────────

let _activeProvider: ProviderConfig | null = null;

/** Resolve API key from environment. */
export function resolveApiKey(cfg: ProviderConfig): string {
  const raw = process.env[cfg.apiKeyEnv] ?? '';
  return raw;
}

/**
 * Auto-detect the first provider with a non-empty API key in the environment.
 * Priority: OPENROUTER → ANTHROPIC → OPENAI → GOOGLE → DEEPSEEK → GROQ → MISTRAL → OLLAMA
 */
export function detectProvider(): ProviderConfig {
  const explicit = process.env.ITER_PROVIDER;
  if (explicit) {
    const found = PROVIDERS.find(p => p.id === explicit);
    if (found) {
      console.error(`[provider] using explicit ITER_PROVIDER=${explicit}`);
      return found;
    }
    console.error(`[provider] ITER_PROVIDER=${explicit} not found, falling back to auto-detect`);
  }

  for (const p of PROVIDERS) {
    if (p.id === 'ollama') continue;
    const key = resolveApiKey(p);
    if (key) {
      console.error(`[provider] auto-detected: ${p.id}`);
      return p;
    }
  }

  console.error('[provider] no API key found, falling back to Ollama');
  return PROVIDERS.find(p => p.id === 'ollama')!;
}

export function getActiveProvider(): ProviderConfig {
  if (!_activeProvider) {
    _activeProvider = detectProvider();
  }
  return _activeProvider;
}

export function setActiveProvider(providerId: string): boolean {
  const found = PROVIDERS.find(p => p.id === providerId);
  if (!found) return false;
  _activeProvider = found;
  console.error(`[provider] switched to: ${providerId}`);
  return true;
}

export function listProviders(): ProviderConfig[] {
  return PROVIDERS;
}

export function inferProvider(model: string): ProviderConfig {
  const active = getActiveProvider();
  if (active.id === 'openrouter') return active;

  if (model.startsWith('claude-')) return PROVIDERS.find(p => p.id === 'anthropic')!;
  if (model.startsWith('gpt-') || model.startsWith('o1') || model.startsWith('o3')) {
    return PROVIDERS.find(p => p.id === 'openai')!;
  }
  if (model.startsWith('gemini-')) return PROVIDERS.find(p => p.id === 'google')!;
  if (model.startsWith('deepseek-')) return PROVIDERS.find(p => p.id === 'deepseek')!;
  if (model.startsWith('llama') || model.startsWith('mixtral') || model.startsWith('gemma')) {
    return PROVIDERS.find(p => p.id === 'groq')!;
  }

  return active;
}

export function stripProviderPrefix(model: string): string {
  const slash = model.indexOf('/');
  if (slash === -1) return model;
  const prefix = model.slice(0, slash);
  if (PROVIDERS.some(p => p.id === prefix)) {
    return model.slice(slash + 1);
  }
  return model;
}