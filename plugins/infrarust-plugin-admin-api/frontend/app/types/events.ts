export type AdminEventType =
  | 'stats.tick'
  | 'player.join'
  | 'player.leave'
  | 'player.switch'
  | 'server.state_change'
  | 'config.reload'
  | 'ban.created'
  | 'ban.removed'
  | string;

export interface AdminEvent<T = unknown> {
  type: AdminEventType;
  timestamp?: string;
  payload: T;
}

// ── SSE Payload Types ──

export interface StatsTickPayload {
  players_online: number;
  servers_online: number;
  bans_active: number;
  uptime_seconds: number;
  memory_rss_bytes: number | null;
  timestamp: string;
}

export interface PlayerJoinPayload {
  player_id: number;
  username: string;
  uuid: string;
  server: string;
  timestamp: string;
}

export interface PlayerLeavePayload {
  player_id: number;
  username: string;
  last_server: string | null;
  timestamp: string;
}

export interface PlayerSwitchPayload {
  player_id: number;
  username: string;
  from_server: string | null;
  to_server: string;
  timestamp: string;
}

export interface ServerStateChangePayload {
  server_id: string;
  old_state: string;
  new_state: string;
  timestamp: string;
}

export interface BanEventPayload {
  target_type: string;
  target_value: string;
  reason?: string | null;
  source?: string;
  timestamp: string;
}
