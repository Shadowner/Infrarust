const isTable = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const isSet = (value: unknown): boolean => value !== undefined && value !== null;

const BARE_KEY = /^[A-Za-z0-9_-]+$/;

function quote(value: string): string {
  const escaped = value
    .replace(/\\/g, '\\\\')
    .replace(/"/g, '\\"')
    .replace(/\n/g, '\\n')
    .replace(/\r/g, '\\r')
    .replace(/\t/g, '\\t');
  return `"${escaped}"`;
}

const renderKey = (key: string): string => (BARE_KEY.test(key) ? key : quote(key));

function renderInline(value: unknown): string {
  if (typeof value === 'string') return quote(value);
  if (typeof value === 'boolean') return String(value);
  if (typeof value === 'number') return Number.isFinite(value) ? String(value) : '0';
  if (Array.isArray(value)) {
    return `[${value.filter(isSet).map(renderInline).join(', ')}]`;
  }
  if (isTable(value)) {
    const pairs = Object.entries(value)
      .filter(([, entry]) => isSet(entry))
      .map(([key, entry]) => `${renderKey(key)} = ${renderInline(entry)}`);
    return `{ ${pairs.join(', ')} }`;
  }
  return '""';
}

function renderSection(table: Record<string, unknown>, path: string[], out: string[]): void {
  const entries = Object.entries(table).filter(([, value]) => isSet(value));
  const scalars = entries.filter(([, value]) => !isTable(value));
  const children = entries.filter((entry): entry is [string, Record<string, unknown>] =>
    isTable(entry[1])
  );

  if (scalars.length > 0) {
    if (path.length > 0) {
      if (out.length > 0) out.push('');
      out.push(`[${path.map(renderKey).join('.')}]`);
    }
    for (const [key, value] of scalars) {
      out.push(`${renderKey(key)} = ${renderInline(value)}`);
    }
  }

  for (const [key, value] of children) {
    renderSection(value, [...path, key], out);
  }
}

/**
 * Renders a config object as TOML. Covers the subset the server and proxy
 * schemas use: scalars, string arrays, arrays of inline tables, nested tables.
 */
export function toToml(value: Record<string, unknown>): string {
  const out: string[] = [];
  renderSection(value, [], out);
  return `${out.join('\n')}\n`;
}
