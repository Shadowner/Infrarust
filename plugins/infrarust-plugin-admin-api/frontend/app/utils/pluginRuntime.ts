import type { PluginRuntimeDto } from '~/types/api';
import { formatDuration } from './time';

export const PLUGIN_REFRESH_MS = 9_500;

export function formatWait(micros: number): string {
  if (micros < 1000) return `${micros}µs`;
  if (micros < 1_000_000) return `${(micros / 1000).toFixed(1)}ms`;
  return `${(micros / 1_000_000).toFixed(1)}s`;
}

export function pluginNeedsAttention(runtime: PluginRuntimeDto | null | undefined): runtime is PluginRuntimeDto {
  return !!runtime && runtime.health !== 'healthy';
}

export function secondsUntilRetry(runtime: PluginRuntimeDto, fetchedAt: number, nowMs: number): number | null {
  if (runtime.retry_in_ms == null) return null;
  return Math.max(0, Math.ceil((fetchedAt + runtime.retry_in_ms - nowMs) / 1000));
}

export function retryLabel(runtime: PluginRuntimeDto, fetchedAt: number, nowMs: number): string | null {
  const seconds = secondsUntilRetry(runtime, fetchedAt, nowMs);
  if (seconds == null) return null;
  return seconds === 0 ? 'retrying now' : `retry in ${formatDuration(seconds)}`;
}
