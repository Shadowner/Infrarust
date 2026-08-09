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
  source: string;
  editable: boolean;
  has_server_manager: boolean;
}

export interface ServerDetailDto extends ServerDto {
  limbo_handlers: string[];
  players: Array<{ username: string; uuid: string }>;
}

// ── Server configuration (JSON projection of the server TOML schema) ──

export type ProxyMode = 'passthrough' | 'zero_copy' | 'client_only' | 'offline' | 'server_only';

export type BalanceStrategy = 'first_available' | 'round_robin' | 'least_conn';

export type ForwardingMode = 'none' | 'bungee_cord' | 'bungee_guard' | 'velocity';

export type DomainRewrite = 'none' | 'from_backend' | { explicit: string };

export type ProbeKind = 'tcp' | 'status_ping';

export interface WeightedAddress {
  address: string;
  weight: number;
}

export type ServerAddressEntry = string | WeightedAddress;

export interface ActiveHealthConfig {
  enabled?: boolean;
  kind?: ProbeKind;
  unhealthy_interval?: string;
  probe_healthy?: boolean;
  interval?: string;
  timeout?: string;
  max_concurrent?: number;
}

export interface MotdEntry {
  text: string;
  favicon?: string;
  version_name?: string;
  max_players?: number;
}

export type MotdState = 'online' | 'sleeping' | 'starting' | 'crashed' | 'stopping' | 'unreachable';

export type MotdConfig = Partial<Record<MotdState, MotdEntry>>;

export interface LocalServerManager {
  type: 'local';
  command: string;
  working_dir: string;
  args?: string[];
  ready_pattern?: string;
  shutdown_timeout?: string;
  shutdown_after?: string;
  start_timeout?: string;
}

export interface RemoteServerManager {
  type: 'pterodactyl' | 'crafty';
  api_url: string;
  api_key: string;
  server_id: string;
  shutdown_after?: string;
  start_timeout?: string;
  poll_interval?: string;
}

export type ServerManagerConfig = LocalServerManager | RemoteServerManager;

export interface TimeoutConfig {
  connect?: string;
  read?: string;
  write?: string;
}

export interface IpFilterConfig {
  whitelist?: string[];
  blacklist?: string[];
}

export interface ServerConfig {
  id?: string;
  name?: string | null;
  network?: string | null;
  domains: string[];
  addresses: ServerAddressEntry[];
  balance?: BalanceStrategy;
  slow_start?: string | null;
  slow_start_aggression?: number;
  active_health?: ActiveHealthConfig | null;
  proxy_mode?: ProxyMode;
  forwarding_mode?: ForwardingMode | null;
  send_proxy_protocol?: boolean;
  domain_rewrite?: DomainRewrite;
  motd?: MotdConfig;
  server_manager?: ServerManagerConfig | null;
  timeouts?: TimeoutConfig | null;
  max_players?: number;
  ip_filter?: IpFilterConfig | null;
  disconnect_message?: string | null;
  limbo_handlers?: string[];
}

export interface ValidationResultDto {
  valid: boolean;
  errors: string[];
  warnings: string[];
}

// ── Load balancer ──

export type BackendState = 'healthy' | 'probing' | 'unhealthy' | 'draining';

export interface BackendStatus {
  address: string;
  weight: number;
  effective_weight: number;
  state: BackendState;
  active_connections: number;
  healthy_since_secs: number | null;
  ejections: number;
  last_failure_secs_ago: number | null;
}

export interface ServerBackendsDto {
  strategy: string;
  backends: BackendStatus[];
}

export type ProxyBackendsDto = Record<string, ServerBackendsDto>;

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

/**
 * `GET /config/proxy`. Secrets are omitted server-side, and the schema keeps
 * growing, so unknown keys stay addressable instead of being dropped.
 */
export interface ProxyConfigDto {
  bind?: string;
  max_connections?: number;
  connect_timeout?: string;
  connect_max_attempts?: number;
  receive_proxy_protocol?: boolean;
  servers_dir?: string;
  plugins_dir?: string;
  worker_threads?: number;
  so_reuseport?: boolean;
  unknown_domain_behavior?: 'default_motd' | 'drop';
  announce_proxy_commands?: boolean;
  keepalive?: { time?: string; interval?: string; retries?: number };
  status_cache?: { ttl?: string; max_entries?: number };
  rate_limit?: Record<string, unknown>;
  ban?: Record<string, unknown>;
  ip_filter?: IpFilterConfig | null;
  forwarding?: Record<string, unknown> | null;
  telemetry?: Record<string, unknown> | null;
  docker?: Record<string, unknown> | null;
  web?: Record<string, unknown> | null;
  permissions?: Record<string, unknown>;
  active_health?: ActiveHealthConfig;
  default_motd?: MotdConfig | null;
  plugins?: Record<string, { path?: string | null; permissions?: string[]; enabled?: boolean }>;
  [key: string]: unknown;
}

export interface RestartAwareResult extends MutationResult {
  requires_restart?: boolean;
}
