/**
 * Model factory - creates Vercel AI SDK LanguageModel instances
 * from provider name + model ID + optional API key.
 *
 * Supports: OpenRouter, Anthropic, OpenAI, Google, DeepSeek, Groq, Mistral, Ollama
 */

import type { LanguageModel } from 'ai';
import { listProviders, resolveApiKey, type ProviderConfig } from './provider.js';

export interface ModelConfig {
  provider: string;
  modelId: string;
  apiKey?: string;
}

const modelCache = new Map<string, any>();

async function createAnthropicModel(modelId: string, apiKey?: string): Promise<any> {
  const { createAnthropic } = await import('@ai-sdk/anthropic');
  const key = apiKey ?? resolveApiKey({ id: 'anthropic', apiKeyEnv: 'ANTHROPIC_API_KEY' } as ProviderConfig);
  const provider = createAnthropic({ apiKey: key });
  return provider(modelId);
}

async function createOpenAIModel(modelId: string, apiKey?: string): Promise<any> {
  const { createOpenAI } = await import('@ai-sdk/openai');
  const key = apiKey ?? resolveApiKey({ id: 'openai', apiKeyEnv: 'OPENAI_API_KEY' } as ProviderConfig);
  const provider = createOpenAI({ apiKey: key });
  return provider.chat(modelId);
}

async function createGoogleModel(modelId: string, apiKey?: string): Promise<any> {
  const { createGoogleGenerativeAI } = await import('@ai-sdk/google');
  const key = apiKey ?? resolveApiKey({ id: 'google', apiKeyEnv: 'GOOGLE_API_KEY' } as ProviderConfig);
  const provider = createGoogleGenerativeAI({ apiKey: key });
  return provider(modelId);
}

async function createDeepSeekModel(modelId: string, apiKey?: string): Promise<any> {
  const { createOpenAI } = await import('@ai-sdk/openai');
  const key = apiKey ?? resolveApiKey({ id: 'deepseek', apiKeyEnv: 'DEEPSEEK_API_KEY' } as ProviderConfig);
  const provider = createOpenAI({
    apiKey: key,
    baseURL: 'https://api.deepseek.com/v1',
  });
  return provider.chat(modelId);
}

async function createGroqModel(modelId: string, apiKey?: string): Promise<any> {
  const { createOpenAI } = await import('@ai-sdk/openai');
  const key = apiKey ?? resolveApiKey({ id: 'groq', apiKeyEnv: 'GROQ_API_KEY' } as ProviderConfig);
  const provider = createOpenAI({
    apiKey: key,
    baseURL: 'https://api.groq.com/openai/v1',
  });
  return provider.chat(modelId);
}

async function createMistralModel(modelId: string, apiKey?: string): Promise<any> {
  const { createOpenAI } = await import('@ai-sdk/openai');
  const key = apiKey ?? resolveApiKey({ id: 'mistral', apiKeyEnv: 'MISTRAL_API_KEY' } as ProviderConfig);
  const provider = createOpenAI({
    apiKey: key,
    baseURL: 'https://api.mistral.ai/v1',
  });
  return provider.chat(modelId);
}

async function createOllamaModel(modelId: string, _apiKey?: string): Promise<any> {
  const { createOpenAI } = await import('@ai-sdk/openai');
  const provider = createOpenAI({
    baseURL: 'http://localhost:11434/v1',
    apiKey: 'ollama',
  });
  return provider.chat(modelId);
}

async function createOpenRouterModel(modelId: string, apiKey?: string): Promise<any> {
  const { createOpenRouter } = await import('@openrouter/ai-sdk-provider');
  const key = apiKey ?? resolveApiKey({ id: 'openrouter', apiKeyEnv: 'OPENROUTER_API_KEY' } as ProviderConfig);
  const provider = createOpenRouter({ apiKey: key });
  return provider.chat(modelId);
}

type ModelCreator = (modelId: string, apiKey?: string) => Promise<any>;

const modelCreators: Record<string, ModelCreator> = {
  anthropic: createAnthropicModel,
  openai: createOpenAIModel,
  google: createGoogleModel,
  deepseek: createDeepSeekModel,
  groq: createGroqModel,
  mistral: createMistralModel,
  ollama: createOllamaModel,
  openrouter: createOpenRouterModel,
};

export function listModels(): Record<string, string[]> {
  return {
    anthropic: ['claude-opus-4-5-20250514', 'claude-sonnet-4-20250514', 'claude-haiku-3-5-20250514'],
    openai: ['gpt-4o', 'gpt-4o-mini', 'o3', 'o4-mini'],
    google: ['gemini-2.0-flash', 'gemini-2.0-flash-lite', 'gemini-1.5-pro'],
    deepseek: ['deepseek-chat', 'deepseek-coder'],
    groq: ['llama-3.3-70b-versatile', 'mixtral-8x7b-32768'],
    mistral: ['mistral-small-latest', 'mistral-large-latest'],
    openrouter: ['anthropic/claude-3.5-sonnet', 'google/gemma-3-27b-it', 'deepseek/deepseek-chat'],
  };
}

export async function createModel(config: ModelConfig): Promise<any> {
  const { provider, modelId, apiKey } = config;
  const cacheKey = `${provider}:${modelId}`;

  if (modelCache.has(cacheKey)) {
    return modelCache.get(cacheKey)!;
  }

  const creator = modelCreators[provider];
  if (!creator) {
    throw new Error(`Unknown provider: ${provider}. Available: ${Object.keys(modelCreators).join(', ')}`);
  }

  const model = await creator(modelId, apiKey);
  modelCache.set(cacheKey, model);
  return model;
}

export function clearModelCache(): void {
  modelCache.clear();
}