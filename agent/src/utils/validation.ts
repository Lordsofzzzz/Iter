/**
 * Tool argument validation — port of pi-agent's validateToolArguments logic.
 *
 * Validates and coerces tool call arguments against the tool's JSON schema.
 * Coercion rules match pi:
 *   - "42" → 42 for number/integer fields
 *   - "true"/"false" → boolean for boolean fields
 *   - numbers/booleans → string for string fields
 *   - null → type default (0, false, "")
 *
 * Throws a descriptive Error if required fields are missing or types are wrong
 * after coercion. The error text is returned to the model as the tool result.
 */

import type { AgentTool, ToolParameter, ToolSchema } from '../llm/types.js';

// ── Public API ────────────────────────────────────────────────────────────────

/**
 * Validate and coerce args against tool.parameters schema.
 * Returns coerced args on success, throws on validation failure.
 */
export function validateToolArguments(
  tool:    AgentTool,
  rawArgs: Record<string, unknown>,
): Record<string, unknown> {
  const schema = tool.parameters;
  const args   = structuredCloneArgs(rawArgs);

  coerceObject(args, schema);

  const errors = validateObject(args, schema, '');
  if (errors.length > 0) {
    const lines = errors.map(e => `  - ${e}`).join('\n');
    throw new Error(
      `Validation failed for tool "${tool.name}":\n${lines}\n\n` +
      `Received arguments:\n${JSON.stringify(rawArgs, null, 2)}`,
    );
  }

  return args;
}

// ── Deep clone (structuredClone may not be available in older Node) ────────────

function structuredCloneArgs(v: unknown): Record<string, unknown> {
  return JSON.parse(JSON.stringify(v ?? {})) as Record<string, unknown>;
}

// ── Coercion ──────────────────────────────────────────────────────────────────

function coerceValue(value: unknown, schema: ToolParameter): unknown {
  const t = schema.type;

  // Recurse into object.
  if (t === 'object' && typeof value === 'object' && value !== null && !Array.isArray(value)) {
    coerceObject(value as Record<string, unknown>, schema as unknown as ToolSchema);
    return value;
  }

  // Recurse into array items.
  if (t === 'array' && Array.isArray(value) && schema.items) {
    for (let i = 0; i < value.length; i++) {
      value[i] = coerceValue(value[i], schema.items);
    }
    return value;
  }

  // Primitive coercion.
  switch (t) {
    case 'number':
    case 'integer': {
      if (value === null) return 0;
      if (typeof value === 'string' && value.trim() !== '') {
        const n = Number(value);
        if (Number.isFinite(n)) return t === 'integer' ? Math.trunc(n) : n;
      }
      if (typeof value === 'boolean') return value ? 1 : 0;
      return value;
    }
    case 'boolean': {
      if (value === null)      return false;
      if (value === 'true')    return true;
      if (value === 'false')   return false;
      if (value === 1)         return true;
      if (value === 0)         return false;
      return value;
    }
    case 'string': {
      if (value === null)                                    return '';
      if (typeof value === 'number' || typeof value === 'boolean') return String(value);
      return value;
    }
    default:
      return value;
  }
}

function coerceObject(obj: Record<string, unknown>, schema: ToolSchema | ToolParameter): void {
  const props = (schema as ToolSchema).properties ?? (schema as ToolParameter).properties;
  if (!props) return;
  for (const [key, propSchema] of Object.entries(props)) {
    if (key in obj) {
      obj[key] = coerceValue(obj[key], propSchema);
    }
  }
}

// ── Validation ────────────────────────────────────────────────────────────────

function validateObject(
  obj:    Record<string, unknown>,
  schema: ToolSchema | ToolParameter,
  path:   string,
): string[] {
  const errors: string[] = [];
  const props    = (schema as ToolSchema).properties ?? (schema as ToolParameter).properties ?? {};
  const required = (schema as ToolSchema).required   ?? (schema as ToolParameter).required   ?? [];

  // Check required fields present.
  for (const key of required) {
    if (!(key in obj) || obj[key] === undefined || obj[key] === null) {
      const fieldPath = path ? `${path}.${key}` : key;
      errors.push(`${fieldPath}: required field missing`);
    }
  }

  // Type-check present fields.
  for (const [key, propSchema] of Object.entries(props)) {
    if (!(key in obj)) continue;
    const val      = obj[key];
    const fieldPath = path ? `${path}.${key}` : key;
    errors.push(...validateValue(val, propSchema, fieldPath));
  }

  return errors;
}

function validateValue(value: unknown, schema: ToolParameter, path: string): string[] {
  const errors: string[] = [];
  const t = schema.type;

  // enum check
  if (schema.enum && !schema.enum.includes(value)) {
    errors.push(`${path}: expected one of [${schema.enum.join(', ')}], got ${JSON.stringify(value)}`);
    return errors;
  }

  switch (t) {
    case 'string':
      if (typeof value !== 'string')
        errors.push(`${path}: expected string, got ${typeof value}`);
      break;

    case 'number':
      if (typeof value !== 'number' || !Number.isFinite(value))
        errors.push(`${path}: expected number, got ${JSON.stringify(value)}`);
      break;

    case 'integer':
      if (typeof value !== 'number' || !Number.isInteger(value))
        errors.push(`${path}: expected integer, got ${JSON.stringify(value)}`);
      break;

    case 'boolean':
      if (typeof value !== 'boolean')
        errors.push(`${path}: expected boolean, got ${typeof value}`);
      break;

    case 'array':
      if (!Array.isArray(value)) {
        errors.push(`${path}: expected array, got ${typeof value}`);
      } else if (schema.items) {
        for (let i = 0; i < value.length; i++) {
          errors.push(...validateValue(value[i], schema.items, `${path}[${i}]`));
        }
      }
      break;

    case 'object':
      if (typeof value !== 'object' || value === null || Array.isArray(value)) {
        errors.push(`${path}: expected object, got ${Array.isArray(value) ? 'array' : typeof value}`);
      } else if (schema.properties) {
        errors.push(...validateObject(
          value as Record<string, unknown>,
          schema as unknown as ToolSchema,
          path,
        ));
      }
      break;
  }

  return errors;
}
