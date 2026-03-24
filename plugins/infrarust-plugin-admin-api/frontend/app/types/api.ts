// ── Response Wrappers ──

export interface ApiEnvelope<T> {
  data: T;
}

export interface PaginatedMeta {
  total: number;
  page: number;
  per_page: number;
  total_pages: number;
}

export interface PaginatedEnvelope<T> {
  data: T[];
  meta: PaginatedMeta;
}

export interface ApiError {
  error: { code: string; message: string };
}

export interface MutationResult {
  success: boolean;
  message: string;
  details?: unknown;
}

// ── Proxy ──

export interface ProxyStatus {
  version: string;
  uptime_seconds: number;
  uptime_human: string;
  players_online: number;
  servers_count: number;
  bind_address: string;
  features: string[];
  memory_rss_bytes: number | null;
}

// ── Stats ──

export interface StatsDto {
  players_online: number;
  servers_total: number;
  servers_online: number;
  servers_sleeping: number;
  servers_offline: number;
  bans_active: number;
  uptime_seconds: number;
  memory_rss_bytes: number | null;
  players_by_server: Record<string, number>;
  servers_by_state: Record<string, number>;
}

// ── Players ──

export interface PlayerDto {
  id: number;
  username: string;
  uuid: string;
  ip: string;
  server: string | null;
  is_active: boolean;
  connected_since: string;
  connected_duration: string;
}

export interface PlayerDetailDto extends PlayerDto {
  protocol_version: number;
  remote_addr_full: string;
}

export interface PlayerCountDto {
  total: number;
  by_server: Record<string, number>;
  by_mode: Record<string, number>;
}

// ── Bans ──

export interface BanDto {
  target_type: 'ip' | 'username' | 'uuid';
  target_value: string;
  reason: string | null;
  expires_at: string | null;
  expires_in: string | null;
  created_at: string;
  source: string;
  permanent: boolean;
}

export interface BanCheckDto {
  banned: boolean;
  ban: BanDto | null;
}

export interface CreateBanRequest {
  target: { type: 'ip' | 'username' | 'uuid'; value: string };
  reason?: string | null;
  duration_seconds?: number | null;
}

// ── Servers ──

export interface ServerDto {
  id: string;
  addresses: string[];
  domains: string[];
  proxy_mode: string;
  state: string | null;
  player_count: number;
  is_api_managed: boolean;
  has_server_manager: boolean;
}

export interface ServerDetailDto extends ServerDto {
  limbo_handlers: string[];
  players: Array<{ username: string; uuid: string }>;
  is_api_managed: boolean;
  has_server_manager: boolean;
}

export interface CreateServerRequest {
  id: string;
  domains: string[];
  addresses: string[];
  proxy_mode?: string;
  limbo_handlers?: string[];
}

export interface UpdateServerRequest {
  domains?: string[];
  addresses?: string[];
  proxy_mode?: string;
  limbo_handlers?: string[];
}

// ── Plugins ──

export interface PluginDto {
  id: string;
  name: string;
  version: string;
  authors: string[];
  description: string | null;
  state: string;
  dependencies: Array<{ id: string; optional: boolean }>;
}

// ── Activity ──

export interface RecentEventDto {
  type: string;
  summary: string;
  timestamp: string;
}

// ── Health Check ──

export interface HealthCheckDto {
  online: boolean;
  latency_ms: number | null;
  motd?: unknown;
  motd_plain?: string | null;
  version_name?: string | null;
  version_protocol?: number | null;
  players_online?: number | null;
  players_max?: number | null;
  player_sample?: Array<{ name: string; id: string }>;
  favicon?: string | null;
  error?: string | null;
  checked_at: string;
}

// ── Logs ──

export interface LogEntryDto {
  timestamp: string;
  level: string;
  target?: string;
  message: string;
  fields?: Record<string, unknown>;
}

// ── Config ──

export interface ProviderDto {
  provider_type: string;
  configs_count: number;
}
