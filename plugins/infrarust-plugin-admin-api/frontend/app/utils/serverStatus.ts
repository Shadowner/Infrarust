import type { HealthCheckDto, ServerDto } from '~/types/api';

/**
 * States reported by a server manager that a TCP ping cannot refine: an idle-stopped
 * or booting backend answers nothing, which is not the same as being down.
 */
const MANAGER_OWNED = ['sleeping', 'starting', 'stopping', 'crashed', 'offline'];

/** `unreachable` rather than `offline`: nothing here knows whether the backend was meant to be up. */
export function resolveServerStatus(server: ServerDto, health?: HealthCheckDto): string {
  const state = server.state ?? '';
  if (MANAGER_OWNED.includes(state)) return state;
  if (health) return health.online ? 'online' : 'unreachable';
  return state || 'unknown';
}
