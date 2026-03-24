/**
 * Format a duration in seconds to a human-readable string.
 * e.g. 3735 → "1h 2m 15s"
 */
export function formatDuration(totalSeconds: number): string {
  if (totalSeconds < 0) totalSeconds = 0;
  const days = Math.floor(totalSeconds / 86400);
  const hours = Math.floor((totalSeconds % 86400) / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = Math.floor(totalSeconds % 60);

  const parts: string[] = [];
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  if (minutes > 0) parts.push(`${minutes}m`);
  if (seconds > 0 || parts.length === 0) parts.push(`${seconds}s`);
  return parts.join(' ');
}

/**
 * Compute a live duration from an ISO timestamp to a reactive `now` value.
 * e.g. formatDurationSince("2026-03-24T10:00:00Z", Date.now()) → "2h 15m 3s"
 */
export function formatDurationSince(isoTimestamp: string, nowMs: number): string {
  try {
    const startMs = new Date(isoTimestamp).getTime();
    const elapsedSeconds = Math.floor((nowMs - startMs) / 1000);
    return formatDuration(elapsedSeconds);
  } catch {
    return '—';
  }
}
