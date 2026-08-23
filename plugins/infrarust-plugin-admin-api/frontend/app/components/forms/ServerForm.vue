<script setup lang="ts">
import {
  CheckCircleIcon,
  ExclamationTriangleIcon,
  LockClosedIcon,
  PlusIcon,
  TrashIcon,
  XCircleIcon,
} from '@heroicons/vue/24/outline';
import type {
  ActiveHealthConfig,
  ApiEnvelope,
  BalanceStrategy,
  DomainRewrite,
  ForwardingMode,
  MotdConfig,
  MotdEntry,
  MotdState,
  ProbeKind,
  LocalServerManager,
  ProxyConfigDto,
  ProxyMode,
  RemoteServerManager,
  ServerAddressEntry,
  ServerConfig,
  ServerManagerConfig,
  ValidationResultDto,
} from '~/types/api';
import { toToml } from '~/utils/toml';

const props = withDefaults(defineProps<{
  isEdit: boolean;
  serverId?: string;
  config?: ServerConfig;
  rawToml?: string;
  submitting?: boolean;
}>(), {
  serverId: '',
  config: undefined,
  rawToml: undefined,
  submitting: false,
});

const emit = defineEmits<{
  submit: [payload: { kind: 'config'; config: ServerConfig } | { kind: 'raw'; toml: string }];
  cancel: [];
}>();

const { request } = useApi();
const { push } = useToast();
const { ask } = useConfirm();

const MOTD_STATES: Array<{ key: MotdState; label: string; hint: string }> = [
  { key: 'online', label: 'Online', hint: 'Backend is reachable.' },
  { key: 'sleeping', label: 'Sleeping', hint: 'Stopped, will wake on join.' },
  { key: 'starting', label: 'Starting', hint: 'Boot in progress.' },
  { key: 'crashed', label: 'Crashed', hint: 'Exited unexpectedly.' },
  { key: 'stopping', label: 'Stopping', hint: 'Shutdown in progress.' },
  { key: 'unreachable', label: 'Unreachable', hint: 'No route to the backend.' },
];

const FORWARDING_MODES: ProxyMode[] = ['passthrough', 'zero_copy', 'server_only'];

interface AddressRow { address: string; weight: number }
interface MotdDraft { enabled: boolean; text: string; favicon: string; version_name: string; max_players: string }
interface HealthDraft {
  enabled: boolean;
  kind: ProbeKind;
  unhealthy_interval: string;
  probe_healthy: boolean;
  interval: string;
  timeout: string;
  max_concurrent: number;
}

const emptyMotdDraft = (): MotdDraft => ({ enabled: false, text: '', favicon: '', version_name: '', max_players: '' });

const serverIdDraft = ref(props.serverId);
const name = ref('');
const network = ref('');
const domains = ref<string[]>([]);
const addressRows = ref<AddressRow[]>([{ address: '', weight: 1 }]);
const balance = ref<BalanceStrategy>('first_available');
const slowStartEnabled = ref(false);
const slowStart = ref('30s');
const slowStartAggression = ref(1);
const proxyMode = ref<ProxyMode>('passthrough');
const forwardingMode = ref<'' | ForwardingMode>('');
const sendProxyProtocol = ref(false);
const domainRewriteKind = ref<'none' | 'explicit' | 'from_backend'>('none');
const domainRewriteValue = ref('');
const maxPlayers = ref(0);
const disconnectMessage = ref('');
const handlers = ref<string[]>([]);

/** What the proxy fills in for keys missing from a server's own `[active_health]` block. */
const HEALTH_KEY_DEFAULTS: HealthDraft = {
  enabled: true,
  kind: 'tcp',
  unhealthy_interval: '10s',
  probe_healthy: false,
  interval: '30s',
  timeout: '3s',
  max_concurrent: 8,
};

const healthOverride = ref(false);
const healthSource = ref<'server' | 'proxy' | null>(null);
const health = reactive<HealthDraft>({ ...HEALTH_KEY_DEFAULTS });
const inheritedHealth = ref<ActiveHealthConfig | null>(null);
const inheritedHealthPending = ref(true);

function fillHealth(source: ActiveHealthConfig) {
  health.enabled = source.enabled ?? HEALTH_KEY_DEFAULTS.enabled;
  health.kind = source.kind ?? HEALTH_KEY_DEFAULTS.kind;
  health.unhealthy_interval = source.unhealthy_interval ?? HEALTH_KEY_DEFAULTS.unhealthy_interval;
  health.probe_healthy = source.probe_healthy ?? HEALTH_KEY_DEFAULTS.probe_healthy;
  health.interval = source.interval ?? HEALTH_KEY_DEFAULTS.interval;
  health.timeout = source.timeout ?? HEALTH_KEY_DEFAULTS.timeout;
  health.max_concurrent = source.max_concurrent ?? HEALTH_KEY_DEFAULTS.max_concurrent;
}

const motd = reactive<Record<MotdState, MotdDraft>>({
  online: emptyMotdDraft(),
  sleeping: emptyMotdDraft(),
  starting: emptyMotdDraft(),
  crashed: emptyMotdDraft(),
  stopping: emptyMotdDraft(),
  unreachable: emptyMotdDraft(),
});

const timeoutsOverride = ref(false);
const timeouts = reactive({ connect: '5s', read: '30s', write: '30s' });
const ipWhitelist = ref<string[]>([]);
const ipBlacklist = ref<string[]>([]);
const managerKind = ref<'none' | 'local' | 'pterodactyl' | 'crafty'>('none');
const localManager = reactive({
  command: '',
  working_dir: '',
  args: [] as string[],
  ready_pattern: '',
  shutdown_timeout: '',
  shutdown_after: '',
  start_timeout: '',
});
const remoteManager = reactive({
  api_url: '',
  api_key: '',
  server_id: '',
  shutdown_after: '',
  start_timeout: '',
  poll_interval: '',
});

const rawOverride = ref(false);
const rawDraft = ref('');
const rawSeededFrom = ref<'fields' | 'file'>('fields');

const isIntercepted = computed(() => ['client_only', 'offline'].includes(proxyMode.value));
const requiresDomain = computed(() => FORWARDING_MODES.includes(proxyMode.value));
const rawEditable = computed(() => props.isEdit);
const fieldsLocked = computed(() => rawOverride.value);

const trimmed = (value: string): string | undefined => {
  const out = value.trim();
  return out.length > 0 ? out : undefined;
};

function hydrate(config: ServerConfig) {
  serverIdDraft.value = config.id ?? props.serverId;
  name.value = config.name ?? '';
  network.value = config.network ?? '';
  domains.value = [...(config.domains ?? [])];
  addressRows.value = (config.addresses ?? []).map((entry) =>
    typeof entry === 'string' ? { address: entry, weight: 1 } : { address: entry.address, weight: entry.weight }
  );
  if (addressRows.value.length === 0) addressRows.value = [{ address: '', weight: 1 }];
  balance.value = config.balance ?? 'first_available';
  slowStartEnabled.value = Boolean(config.slow_start);
  if (config.slow_start) slowStart.value = config.slow_start;
  slowStartAggression.value = config.slow_start_aggression ?? 1;
  proxyMode.value = config.proxy_mode ?? 'passthrough';
  forwardingMode.value = config.forwarding_mode ?? '';
  sendProxyProtocol.value = config.send_proxy_protocol ?? false;

  const rewrite = config.domain_rewrite ?? 'none';
  if (typeof rewrite === 'object') {
    domainRewriteKind.value = 'explicit';
    domainRewriteValue.value = rewrite.explicit;
  } else {
    domainRewriteKind.value = rewrite;
    domainRewriteValue.value = '';
  }

  maxPlayers.value = config.max_players ?? 0;
  disconnectMessage.value = config.disconnect_message ?? '';
  handlers.value = [...(config.limbo_handlers ?? [])];

  healthOverride.value = Boolean(config.active_health);
  if (config.active_health) {
    fillHealth(config.active_health);
    healthSource.value = 'server';
  }

  for (const { key } of MOTD_STATES) {
    const entry = config.motd?.[key];
    motd[key] = entry
      ? {
          enabled: true,
          text: entry.text ?? '',
          favicon: entry.favicon ?? '',
          version_name: entry.version_name ?? '',
          max_players: entry.max_players == null ? '' : String(entry.max_players),
        }
      : emptyMotdDraft();
  }

  timeoutsOverride.value = Boolean(config.timeouts);
  if (config.timeouts) {
    timeouts.connect = config.timeouts.connect ?? '5s';
    timeouts.read = config.timeouts.read ?? '30s';
    timeouts.write = config.timeouts.write ?? '30s';
  }

  ipWhitelist.value = [...(config.ip_filter?.whitelist ?? [])];
  ipBlacklist.value = [...(config.ip_filter?.blacklist ?? [])];

  const manager = config.server_manager;
  managerKind.value = manager?.type ?? 'none';
  if (manager?.type === 'local') {
    localManager.command = manager.command;
    localManager.working_dir = manager.working_dir ?? '';
    localManager.args = [...(manager.args ?? [])];
    localManager.ready_pattern = manager.ready_pattern ?? '';
    localManager.shutdown_timeout = manager.shutdown_timeout ?? '';
    localManager.shutdown_after = manager.shutdown_after ?? '';
    localManager.start_timeout = manager.start_timeout ?? '';
  } else if (manager) {
    remoteManager.api_url = manager.api_url;
    remoteManager.api_key = manager.api_key;
    remoteManager.server_id = manager.server_id;
    remoteManager.shutdown_after = manager.shutdown_after ?? '';
    remoteManager.start_timeout = manager.start_timeout ?? '';
    remoteManager.poll_interval = manager.poll_interval ?? '';
  }
}

function buildAddresses(): ServerAddressEntry[] {
  return addressRows.value
    .filter((row) => row.address.trim().length > 0)
    .map((row) =>
      row.weight === 1 ? row.address.trim() : { address: row.address.trim(), weight: row.weight }
    );
}

function buildActiveHealth(): ActiveHealthConfig | undefined {
  if (!healthOverride.value || !healthSource.value) return undefined;
  return {
    enabled: health.enabled,
    kind: health.kind,
    unhealthy_interval: health.unhealthy_interval,
    probe_healthy: health.probe_healthy,
    interval: health.interval,
    timeout: health.timeout,
    max_concurrent: health.max_concurrent,
  };
}

function buildMotd(): MotdConfig | undefined {
  const out: MotdConfig = {};
  for (const { key } of MOTD_STATES) {
    const draft = motd[key];
    if (!draft.enabled || draft.text.trim().length === 0) continue;
    const entry: MotdEntry = { text: draft.text };
    const favicon = trimmed(draft.favicon);
    if (favicon) entry.favicon = favicon;
    const version = trimmed(draft.version_name);
    if (version) entry.version_name = version;
    const max = Number.parseInt(draft.max_players, 10);
    if (Number.isFinite(max)) entry.max_players = max;
    out[key] = entry;
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

function buildManager(): ServerManagerConfig | undefined {
  const kind = managerKind.value;
  if (kind === 'none') return undefined;
  if (kind === 'local') {
    const manager: LocalServerManager = {
      type: 'local',
      command: localManager.command.trim(),
      working_dir: localManager.working_dir.trim(),
    };
    if (localManager.args.length > 0) manager.args = [...localManager.args];
    const readyPattern = trimmed(localManager.ready_pattern);
    if (readyPattern) manager.ready_pattern = readyPattern;
    const shutdownTimeout = trimmed(localManager.shutdown_timeout);
    if (shutdownTimeout) manager.shutdown_timeout = shutdownTimeout;
    const shutdownAfter = trimmed(localManager.shutdown_after);
    if (shutdownAfter) manager.shutdown_after = shutdownAfter;
    const startTimeout = trimmed(localManager.start_timeout);
    if (startTimeout) manager.start_timeout = startTimeout;
    return manager;
  }
  const manager: RemoteServerManager = {
    type: kind,
    api_url: remoteManager.api_url.trim(),
    api_key: remoteManager.api_key,
    server_id: remoteManager.server_id.trim(),
  };
  const shutdownAfter = trimmed(remoteManager.shutdown_after);
  if (shutdownAfter) manager.shutdown_after = shutdownAfter;
  const startTimeout = trimmed(remoteManager.start_timeout);
  if (startTimeout) manager.start_timeout = startTimeout;
  const pollInterval = trimmed(remoteManager.poll_interval);
  if (pollInterval) manager.poll_interval = pollInterval;
  return manager;
}

function buildDomainRewrite(): DomainRewrite | undefined {
  if (domainRewriteKind.value === 'none') return undefined;
  if (domainRewriteKind.value === 'from_backend') return 'from_backend';
  const explicit = trimmed(domainRewriteValue.value);
  return explicit ? { explicit } : undefined;
}

function buildConfig(): ServerConfig {
  const config: ServerConfig = {
    domains: [...domains.value],
    addresses: buildAddresses(),
    proxy_mode: proxyMode.value,
  };

  const id = trimmed(serverIdDraft.value);
  if (id) config.id = id;
  const displayName = trimmed(name.value);
  if (displayName) config.name = displayName;
  const net = trimmed(network.value);
  if (net) config.network = net;

  if (balance.value !== 'first_available') config.balance = balance.value;
  if (slowStartEnabled.value) {
    const window = trimmed(slowStart.value);
    if (window) config.slow_start = window;
  }
  if (slowStartAggression.value !== 1) config.slow_start_aggression = slowStartAggression.value;

  const activeHealth = buildActiveHealth();
  if (activeHealth) config.active_health = activeHealth;

  if (forwardingMode.value) config.forwarding_mode = forwardingMode.value;
  if (sendProxyProtocol.value) config.send_proxy_protocol = true;

  const rewrite = buildDomainRewrite();
  if (rewrite) config.domain_rewrite = rewrite;

  const motdConfig = buildMotd();
  if (motdConfig) config.motd = motdConfig;

  const manager = buildManager();
  if (manager) config.server_manager = manager;

  if (timeoutsOverride.value) {
    config.timeouts = { connect: timeouts.connect, read: timeouts.read, write: timeouts.write };
  }

  if (maxPlayers.value > 0) config.max_players = maxPlayers.value;

  if (ipWhitelist.value.length > 0 || ipBlacklist.value.length > 0) {
    config.ip_filter = {};
    if (ipWhitelist.value.length > 0) config.ip_filter.whitelist = [...ipWhitelist.value];
    if (ipBlacklist.value.length > 0) config.ip_filter.blacklist = [...ipBlacklist.value];
  }

  const message = trimmed(disconnectMessage.value);
  if (message) config.disconnect_message = message;
  if (handlers.value.length > 0) config.limbo_handlers = [...handlers.value];

  return config;
}

const generatedToml = computed(() => toToml(buildConfig() as unknown as Record<string, unknown>));
const effectiveToml = computed(() => (rawOverride.value ? rawDraft.value : generatedToml.value));

const baselineToml = ref('');
watch(() => props.config, (config) => {
  if (config) hydrate(config);
  baselineToml.value = generatedToml.value;
}, { immediate: true });

const typedEdited = computed(() => generatedToml.value !== baselineToml.value);

const canOverrideHealth = computed(() => healthSource.value !== null || inheritedHealth.value !== null);

watch(healthOverride, (on) => {
  if (!on || healthSource.value) return;
  const inherited = inheritedHealth.value;
  if (!inherited) {
    healthOverride.value = false;
    return;
  }
  fillHealth(inherited);
  healthSource.value = 'proxy';
});

onMounted(async () => {
  try {
    const res = await request<ApiEnvelope<ProxyConfigDto>>('/config/proxy');
    inheritedHealth.value = res.data.active_health ?? null;
  } catch {
    inheritedHealth.value = null;
  } finally {
    inheritedHealthPending.value = false;
  }
});

const validation = ref<ValidationResultDto | null>(null);
const validating = ref(false);
const validationFailed = ref(false);
let validateTimer: ReturnType<typeof setTimeout> | null = null;

async function runValidation() {
  const toml = effectiveToml.value;
  validating.value = true;
  validationFailed.value = false;
  try {
    const res = await request<ApiEnvelope<ValidationResultDto>>('/servers/validate', {
      method: 'POST',
      body: toml,
      headers: { 'Content-Type': 'text/plain' },
    });
    validation.value = res.data;
  } catch {
    validation.value = null;
    validationFailed.value = true;
  } finally {
    validating.value = false;
  }
}

watch(effectiveToml, () => {
  if (validateTimer) clearTimeout(validateTimer);
  validateTimer = setTimeout(runValidation, 600);
});

onBeforeUnmount(() => { if (validateTimer) clearTimeout(validateTimer); });

function startRawOverride() {
  const file = props.rawToml;
  if (file && !typedEdited.value) {
    rawDraft.value = file;
    rawSeededFrom.value = 'file';
  } else {
    rawDraft.value = generatedToml.value;
    rawSeededFrom.value = 'fields';
  }
  rawOverride.value = true;
}

async function loadServerFile() {
  const file = props.rawToml;
  if (!file) return;
  const confirmed = await ask(
    'Load the file from the server',
    'The editor is replaced with the TOML currently stored for this server. Everything in the editor now, including the edits carried over from the tabs, is lost.'
  );
  if (!confirmed) return;
  rawDraft.value = file;
  rawSeededFrom.value = 'file';
}

async function discardRawOverride() {
  const confirmed = await ask(
    'Discard raw TOML',
    'The manual TOML edits are dropped and the tabbed fields take over again.'
  );
  if (!confirmed) return;
  rawOverride.value = false;
  rawDraft.value = '';
}

function addAddressRow() {
  addressRows.value = [...addressRows.value, { address: '', weight: 1 }];
}

function removeAddressRow(index: number) {
  addressRows.value = addressRows.value.filter((_, i) => i !== index);
  if (addressRows.value.length === 0) addressRows.value = [{ address: '', weight: 1 }];
}

const tabs = [
  { id: 'general', label: 'General' },
  { id: 'backends', label: 'Backends' },
  { id: 'health', label: 'Health' },
  { id: 'motd', label: 'MOTD' },
  { id: 'advanced', label: 'Advanced' },
] as const;

type TabId = typeof tabs[number]['id'];

const activeTab = ref<TabId>('general');
const tabButtons = ref<HTMLButtonElement[]>([]);

function setTabButton(el: unknown, index: number) {
  if (el) tabButtons.value[index] = el as HTMLButtonElement;
}

function focusTab(index: number) {
  const next = tabs[(index + tabs.length) % tabs.length];
  if (!next) return;
  activeTab.value = next.id;
  nextTick(() => tabButtons.value[(index + tabs.length) % tabs.length]?.focus());
}

function onTabKeydown(event: KeyboardEvent, index: number) {
  if (event.key === 'ArrowRight') { event.preventDefault(); focusTab(index + 1); }
  else if (event.key === 'ArrowLeft') { event.preventDefault(); focusTab(index - 1); }
  else if (event.key === 'Home') { event.preventDefault(); focusTab(0); }
  else if (event.key === 'End') { event.preventDefault(); focusTab(tabs.length - 1); }
}

const errorCount = computed(() => validation.value?.errors.length ?? 0);

function submit() {
  if (rawOverride.value) {
    if (rawDraft.value.trim().length === 0) {
      push({ type: 'error', title: 'The raw TOML is empty' });
      return;
    }
    emit('submit', { kind: 'raw', toml: rawDraft.value });
    return;
  }

  if (!props.isEdit && !serverIdDraft.value.trim()) {
    push({ type: 'error', title: 'Server ID is required' });
    activeTab.value = 'general';
    return;
  }
  if (requiresDomain.value && domains.value.length === 0) {
    push({ type: 'error', title: `${proxyMode.value} routes by domain, so at least one domain is required` });
    activeTab.value = 'general';
    return;
  }
  if (buildAddresses().length === 0) {
    push({ type: 'error', title: 'At least one backend address is required' });
    activeTab.value = 'backends';
    return;
  }
  if (managerKind.value === 'local' && !(trimmed(localManager.command) && trimmed(localManager.working_dir))) {
    push({ type: 'error', title: 'A local server manager needs a command and a working directory' });
    activeTab.value = 'advanced';
    return;
  }

  emit('submit', { kind: 'config', config: buildConfig() });
}
</script>

<template>
  <div class="glass-pane relative overflow-hidden">
    <div class="absolute inset-x-0 top-0 h-[3px] bg-[var(--ir-accent-gradient)]" />

    <!-- Tabs -->
    <div
      role="tablist"
      aria-label="Server configuration sections"
      class="flex flex-wrap gap-1 border-b border-[var(--ir-border)] px-3 pt-5"
    >
      <button
        v-for="(tab, index) in tabs"
        :id="`server-tab-${tab.id}`"
        :key="tab.id"
        :ref="(el) => setTabButton(el, index)"
        role="tab"
        type="button"
        :aria-selected="activeTab === tab.id"
        :aria-controls="`server-panel-${tab.id}`"
        :tabindex="activeTab === tab.id ? 0 : -1"
        class="relative rounded-t-[var(--ir-radius-sm)] px-3.5 py-2 text-sm font-medium transition-colors duration-150"
        :class="activeTab === tab.id
          ? 'bg-[var(--ir-accent-soft)] text-white'
          : 'text-[var(--ir-text-muted)] hover:bg-white/[0.04] hover:text-white'"
        @click="activeTab = tab.id"
        @keydown="onTabKeydown($event, index)"
      >
        {{ tab.label }}
        <span
          v-if="tab.id === 'advanced' && errorCount > 0"
          class="ml-1.5 inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-[var(--ir-danger)] px-1 text-[10px] font-bold text-white"
        >
          {{ errorCount }}
        </span>
        <span
          v-if="activeTab === tab.id"
          class="absolute inset-x-2 -bottom-px h-[2px] rounded-full bg-[var(--ir-accent)]"
        />
      </button>
    </div>

    <!-- Raw override banner -->
    <div
      v-if="rawOverride"
      class="flex flex-wrap items-center gap-3 border-b border-[rgba(233,160,71,0.3)] bg-[rgba(233,160,71,0.1)] px-5 py-3"
    >
      <LockClosedIcon class="h-4 w-4 shrink-0 text-[var(--ir-warn)]" />
      <p class="flex-1 text-xs text-[#ffd8ad]">
        Raw TOML is the source of truth. The tabbed fields are read-only, stop tracking it as you type, and are not sent
        on save.
      </p>
      <button class="btn btn-secondary text-xs" @click="discardRawOverride">Back to fields</button>
    </div>

    <div class="p-6">
      <!-- General -->
      <div
        v-show="activeTab === 'general'"
        id="server-panel-general"
        role="tabpanel"
        aria-labelledby="server-tab-general"
        :inert="fieldsLocked || undefined"
        class="grid gap-5"
      >
        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label for="field-id" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Server ID</label>
            <input id="field-id" v-model="serverIdDraft" class="input font-mono" placeholder="my-server" :disabled="isEdit || fieldsLocked" />
          </div>
          <div>
            <label for="field-name" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Display name</label>
            <input id="field-name" v-model="name" class="input" placeholder="Optional, overrides the ID" :disabled="fieldsLocked" />
          </div>
        </div>

        <div>
          <label for="field-network" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Network</label>
          <input id="field-network" v-model="network" class="input" placeholder="Leave empty to isolate this server" :disabled="fieldsLocked" />
          <p class="mt-1.5 text-[11px] text-[var(--ir-text-muted)]">Players can only be switched between servers sharing a network.</p>
        </div>

        <div>
          <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Domains</span>
          <TagInput v-model="domains" placeholder="mc.example.com or *.example.com" />
          <p class="mt-1.5 text-[11px] text-[var(--ir-text-muted)]">
            <template v-if="requiresDomain">Required: {{ proxyMode }} routes players by the domain they connect to.</template>
            <template v-else>Optional in {{ proxyMode }} mode. With none, the server is only reachable through a server switch.</template>
          </p>
        </div>

        <div>
          <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Proxy mode</span>
          <ProxyModeSelector v-model="proxyMode" />
        </div>

        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label for="field-forwarding" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Player info forwarding</label>
            <select id="field-forwarding" v-model="forwardingMode" class="input" :disabled="fieldsLocked">
              <option value="">Inherit proxy default</option>
              <option value="none">None</option>
              <option value="bungee_cord">BungeeCord</option>
              <option value="bungee_guard">BungeeGuard</option>
              <option value="velocity">Velocity modern</option>
            </select>
          </div>
          <div>
            <label for="field-max-players" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Max players</label>
            <input id="field-max-players" v-model.number="maxPlayers" type="number" min="0" class="input font-mono" :disabled="fieldsLocked" />
            <p class="mt-1.5 text-[11px] text-[var(--ir-text-muted)]">0 means unlimited.</p>
          </div>
        </div>

        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label for="field-rewrite" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Domain rewrite</label>
            <select id="field-rewrite" v-model="domainRewriteKind" class="input" :disabled="fieldsLocked">
              <option value="none">Forward as-is</option>
              <option value="explicit">Explicit domain</option>
              <option value="from_backend">Take from backend address</option>
            </select>
          </div>
          <div v-if="domainRewriteKind === 'explicit'">
            <label for="field-rewrite-value" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Rewrite to</label>
            <input id="field-rewrite-value" v-model="domainRewriteValue" class="input font-mono" placeholder="backend.internal" :disabled="fieldsLocked" />
          </div>
        </div>

        <label class="flex items-center gap-2.5 text-sm">
          <input v-model="sendProxyProtocol" type="checkbox" class="h-4 w-4 accent-[var(--ir-accent)]" :disabled="fieldsLocked" />
          Send the PROXY protocol header to the backend
        </label>

        <div>
          <label for="field-disconnect" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Disconnect message</label>
          <input id="field-disconnect" v-model="disconnectMessage" class="input" placeholder="Shown when the backend cannot be reached" :disabled="fieldsLocked" />
        </div>

        <div v-if="isIntercepted">
          <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Limbo handlers</span>
          <TagInput v-model="handlers" placeholder="plugin-id" />
          <p class="mt-1.5 text-[11px] text-[var(--ir-text-muted)]">Plugin IDs, run in the order listed.</p>
        </div>
      </div>

      <!-- Backends -->
      <div
        v-show="activeTab === 'backends'"
        id="server-panel-backends"
        role="tabpanel"
        aria-labelledby="server-tab-backends"
        :inert="fieldsLocked || undefined"
        class="grid gap-5"
      >
        <div>
          <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Backend addresses</span>
          <div class="rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-3">
            <div class="grid gap-2">
              <div class="hidden grid-cols-[1fr_100px_auto] gap-2 px-1 sm:grid">
                <span class="text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--ir-text-muted)]">host:port</span>
                <span class="text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--ir-text-muted)]">Weight</span>
                <span class="sr-only">Remove</span>
              </div>
              <div
                v-for="(row, index) in addressRows"
                :key="index"
                class="grid grid-cols-[1fr_auto] gap-2 sm:grid-cols-[1fr_100px_auto]"
              >
                <input
                  v-model="row.address"
                  class="input font-mono"
                  placeholder="127.0.0.1:25565"
                  :aria-label="`Backend address ${index + 1}`"
                  :disabled="fieldsLocked"
                />
                <input
                  v-model.number="row.weight"
                  type="number"
                  min="1"
                  class="input font-mono"
                  :aria-label="`Weight for backend ${index + 1}`"
                  :disabled="fieldsLocked"
                />
                <button
                  class="btn btn-ghost text-[var(--ir-danger)]"
                  type="button"
                  :aria-label="`Remove backend ${index + 1}`"
                  :disabled="fieldsLocked"
                  @click="removeAddressRow(index)"
                >
                  <TrashIcon class="h-4 w-4" />
                </button>
              </div>
            </div>
            <button class="btn btn-ghost mt-2 flex items-center gap-1 text-xs" type="button" :disabled="fieldsLocked" @click="addAddressRow">
              <PlusIcon class="h-3.5 w-3.5" /> Add address
            </button>
          </div>
        </div>

        <div>
          <label for="field-balance" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Balance strategy</label>
          <select id="field-balance" v-model="balance" class="input" :disabled="fieldsLocked">
            <option value="first_available">First available — config order</option>
            <option value="round_robin">Round robin — weighted, smooth</option>
            <option value="least_conn">Least connections — weighted</option>
          </select>
          <p class="mt-1.5 text-[11px] text-[var(--ir-text-muted)]">
            Slow start only applies to round robin and least connections.
          </p>
        </div>

        <div class="rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-4">
          <label class="flex items-center gap-2.5 text-sm">
            <input v-model="slowStartEnabled" type="checkbox" class="h-4 w-4 accent-[var(--ir-accent)]" :disabled="fieldsLocked" />
            Ramp traffic to freshly healthy backends
          </label>
          <div v-if="slowStartEnabled" class="mt-3 grid gap-4 sm:grid-cols-2">
            <div>
              <label for="field-slow-start" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Ramp window</label>
              <input id="field-slow-start" v-model="slowStart" class="input font-mono" placeholder="30s" :disabled="fieldsLocked" />
            </div>
            <div>
              <label for="field-aggression" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Aggression</label>
              <input id="field-aggression" v-model.number="slowStartAggression" type="number" min="0.1" step="0.1" class="input font-mono" :disabled="fieldsLocked" />
              <p class="mt-1.5 text-[11px] text-[var(--ir-text-muted)]">1.0 is linear, higher is a softer start.</p>
            </div>
          </div>
        </div>
      </div>

      <!-- Health -->
      <div
        v-show="activeTab === 'health'"
        id="server-panel-health"
        role="tabpanel"
        aria-labelledby="server-tab-health"
        :inert="fieldsLocked || undefined"
        class="grid gap-5"
      >
        <div class="rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-4">
          <label class="flex items-start gap-2.5 text-sm">
            <input
              v-model="healthOverride"
              type="checkbox"
              class="mt-0.5 h-4 w-4 accent-[var(--ir-accent)]"
              :disabled="fieldsLocked || !canOverrideHealth"
            />
            <span>
              Override the proxy-wide active health settings
              <span class="mt-1 block text-[11px] text-[var(--ir-text-muted)]">
                Unchecked, this server writes no <code class="font-mono">active_health</code> block and follows whatever
                <NuxtLink to="/config/proxy" class="text-[var(--ir-accent)] hover:underline">the global config</NuxtLink> says.
              </span>
            </span>
          </label>

          <p v-if="!canOverrideHealth && inheritedHealthPending" class="mt-3 text-[11px] text-[var(--ir-text-muted)]">
            Reading the proxy-wide settings...
          </p>
          <p
            v-else-if="!canOverrideHealth"
            class="mt-3 rounded-lg border border-[rgba(233,160,71,0.25)] bg-[rgba(233,160,71,0.1)] px-3 py-2 text-[11px] text-[#ffd8ad]"
          >
            The proxy-wide settings could not be read, so this override cannot be prefilled with them. Writing one from
            here would replace values this page never saw. Use the raw TOML editor on the Advanced tab instead.
          </p>
        </div>

        <div v-if="healthOverride" class="grid gap-5">
          <p class="text-[11px] text-[var(--ir-text-muted)]">
            <template v-if="healthSource === 'proxy'">Prefilled from the proxy-wide settings.</template>
            <template v-else>Loaded from this server's own block.</template>
            The proxy takes this block instead of the global one, never a field-by-field merge, so all seven values below
            are written.
          </p>

          <label class="flex items-center gap-2.5 text-sm">
            <input v-model="health.enabled" type="checkbox" class="h-4 w-4 accent-[var(--ir-accent)]" :disabled="fieldsLocked" />
            Probe this server's backends
          </label>

          <div class="grid gap-4 sm:grid-cols-2">
            <div>
              <label for="field-probe-kind" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Probe kind</label>
              <select id="field-probe-kind" v-model="health.kind" class="input" :disabled="fieldsLocked">
                <option value="tcp">TCP connect</option>
                <option value="status_ping">Minecraft status ping</option>
              </select>
            </div>
            <div>
              <label for="field-max-concurrent" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Max concurrent probes</label>
              <input id="field-max-concurrent" v-model.number="health.max_concurrent" type="number" min="1" class="input font-mono" :disabled="fieldsLocked" />
              <p class="mt-1.5 text-[11px] text-[var(--ir-text-muted)]">The prober reads this from the global block only.</p>
            </div>
          </div>

          <div class="grid gap-4 sm:grid-cols-3">
            <div>
              <label for="field-unhealthy-interval" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Recovery interval</label>
              <input id="field-unhealthy-interval" v-model="health.unhealthy_interval" class="input font-mono" placeholder="10s" :disabled="fieldsLocked" />
              <p class="mt-1.5 text-[11px] text-[var(--ir-text-muted)]">How often ejected addresses are retried.</p>
            </div>
            <div>
              <label for="field-interval" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Sweep interval</label>
              <input id="field-interval" v-model="health.interval" class="input font-mono" placeholder="30s" :disabled="fieldsLocked" />
            </div>
            <div>
              <label for="field-timeout" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Probe timeout</label>
              <input id="field-timeout" v-model="health.timeout" class="input font-mono" placeholder="3s" :disabled="fieldsLocked" />
            </div>
          </div>

          <label class="flex items-start gap-2.5 text-sm">
            <input v-model="health.probe_healthy" type="checkbox" class="mt-0.5 h-4 w-4 accent-[var(--ir-accent)]" :disabled="fieldsLocked" />
            <span>
              Also probe healthy addresses
              <span class="mt-1 block text-[11px] text-[var(--ir-text-muted)]">
                Uses the sweep interval. Real traffic already detects healthy-to-failing on its own.
              </span>
            </span>
          </label>
        </div>
      </div>

      <!-- MOTD -->
      <div
        v-show="activeTab === 'motd'"
        id="server-panel-motd"
        role="tabpanel"
        aria-labelledby="server-tab-motd"
        :inert="fieldsLocked || undefined"
        class="grid gap-4"
      >
        <p class="text-[11px] text-[var(--ir-text-muted)]">
          Each state falls back to the proxy default while it is off. Use § codes for colours.
        </p>
        <div
          v-for="state in MOTD_STATES"
          :key="state.key"
          class="rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-4"
        >
          <label class="flex items-center gap-2.5 text-sm">
            <input v-model="motd[state.key].enabled" type="checkbox" class="h-4 w-4 accent-[var(--ir-accent)]" :disabled="fieldsLocked" />
            <span class="font-semibold">{{ state.label }}</span>
            <span class="text-[11px] text-[var(--ir-text-muted)]">{{ state.hint }}</span>
          </label>

          <div v-if="motd[state.key].enabled" class="mt-3 grid gap-4">
            <div>
              <label :for="`motd-text-${state.key}`" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Text</label>
              <input :id="`motd-text-${state.key}`" v-model="motd[state.key].text" class="input font-mono" placeholder="§6My server" :disabled="fieldsLocked" />
            </div>
            <div class="grid gap-4 sm:grid-cols-3">
              <div>
                <label :for="`motd-version-${state.key}`" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Version label</label>
                <input :id="`motd-version-${state.key}`" v-model="motd[state.key].version_name" class="input" placeholder="1.21" :disabled="fieldsLocked" />
              </div>
              <div>
                <label :for="`motd-max-${state.key}`" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Shown max players</label>
                <input :id="`motd-max-${state.key}`" v-model="motd[state.key].max_players" type="number" min="0" class="input font-mono" :disabled="fieldsLocked" />
              </div>
              <div>
                <label :for="`motd-favicon-${state.key}`" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Favicon</label>
                <input :id="`motd-favicon-${state.key}`" v-model="motd[state.key].favicon" class="input" placeholder="path, URL or data URI" :disabled="fieldsLocked" />
              </div>
            </div>
            <div class="flex items-start gap-3 rounded-lg border border-[var(--ir-border)] bg-[#080808] p-3">
              <img
                v-if="motd[state.key].favicon.startsWith('data:')"
                :src="motd[state.key].favicon"
                alt=""
                class="h-12 w-12 rounded border border-[var(--ir-border)]"
                style="image-rendering: pixelated;"
              />
              <div class="min-w-0 flex-1">
                <MinecraftMotd :motd="motd[state.key].text" />
                <p class="mt-1 text-[10px] text-[var(--ir-text-muted)]">
                  {{ motd[state.key].version_name || 'version from backend' }} ·
                  {{ motd[state.key].max_players || '—' }} slots
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- Advanced -->
      <div
        v-show="activeTab === 'advanced'"
        id="server-panel-advanced"
        role="tabpanel"
        aria-labelledby="server-tab-advanced"
        class="grid gap-6"
      >
        <div :inert="fieldsLocked || undefined" class="grid gap-5">
          <div class="rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-4">
            <label class="flex items-center gap-2.5 text-sm">
              <input v-model="timeoutsOverride" type="checkbox" class="h-4 w-4 accent-[var(--ir-accent)]" :disabled="fieldsLocked" />
              Override the global timeouts
            </label>
            <div v-if="timeoutsOverride" class="mt-3 grid gap-4 sm:grid-cols-3">
              <div>
                <label for="field-t-connect" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Connect</label>
                <input id="field-t-connect" v-model="timeouts.connect" class="input font-mono" placeholder="5s" :disabled="fieldsLocked" />
              </div>
              <div>
                <label for="field-t-read" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Read</label>
                <input id="field-t-read" v-model="timeouts.read" class="input font-mono" placeholder="30s" :disabled="fieldsLocked" />
              </div>
              <div>
                <label for="field-t-write" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Write</label>
                <input id="field-t-write" v-model="timeouts.write" class="input font-mono" placeholder="30s" :disabled="fieldsLocked" />
              </div>
            </div>
          </div>

          <div class="grid gap-4 sm:grid-cols-2">
            <div>
              <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">IP whitelist (CIDR)</span>
              <TagInput v-model="ipWhitelist" placeholder="10.0.0.0/8" />
            </div>
            <div>
              <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">IP blacklist (CIDR)</span>
              <TagInput v-model="ipBlacklist" placeholder="203.0.113.7/32" />
            </div>
          </div>

          <div>
            <label for="field-manager" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Server manager</label>
            <select id="field-manager" v-model="managerKind" class="input" :disabled="fieldsLocked">
              <option value="none">None — the backend is always up</option>
              <option value="local">Local process</option>
              <option value="pterodactyl">Pterodactyl</option>
              <option value="crafty">Crafty Controller</option>
            </select>
          </div>

          <div v-if="managerKind === 'local'" class="grid gap-4 rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-4">
            <div class="grid gap-4 sm:grid-cols-2">
              <div>
                <label for="field-command" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Command</label>
                <input id="field-command" v-model="localManager.command" class="input font-mono" placeholder="java" :disabled="fieldsLocked" />
              </div>
              <div>
                <label for="field-workdir" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Working directory</label>
                <input id="field-workdir" v-model="localManager.working_dir" class="input font-mono" placeholder="/srv/mc" :disabled="fieldsLocked" />
              </div>
            </div>
            <div>
              <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Arguments</span>
              <TagInput v-model="localManager.args" placeholder="-jar" />
            </div>
            <div class="grid gap-4 sm:grid-cols-2">
              <div>
                <label for="field-ready" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Ready pattern</label>
                <input id="field-ready" v-model="localManager.ready_pattern" class="input font-mono" placeholder="Done (" :disabled="fieldsLocked" />
              </div>
              <div>
                <label for="field-shutdown-timeout" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Shutdown timeout</label>
                <input id="field-shutdown-timeout" v-model="localManager.shutdown_timeout" class="input font-mono" placeholder="30s" :disabled="fieldsLocked" />
              </div>
              <div>
                <label for="field-shutdown-after" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Idle shutdown after</label>
                <input id="field-shutdown-after" v-model="localManager.shutdown_after" class="input font-mono" placeholder="10m" :disabled="fieldsLocked" />
              </div>
              <div>
                <label for="field-start-timeout" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Start timeout</label>
                <input id="field-start-timeout" v-model="localManager.start_timeout" class="input font-mono" placeholder="2m" :disabled="fieldsLocked" />
              </div>
            </div>
          </div>

          <div v-else-if="managerKind !== 'none'" class="grid gap-4 rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-4">
            <div class="grid gap-4 sm:grid-cols-3">
              <div>
                <label for="field-api-url" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">API URL</label>
                <input id="field-api-url" v-model="remoteManager.api_url" class="input font-mono" placeholder="https://panel.example.com" :disabled="fieldsLocked" />
              </div>
              <div>
                <label for="field-api-key" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">API key</label>
                <input id="field-api-key" v-model="remoteManager.api_key" type="password" class="input font-mono" :disabled="fieldsLocked" />
              </div>
              <div>
                <label for="field-remote-id" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Remote server ID</label>
                <input id="field-remote-id" v-model="remoteManager.server_id" class="input font-mono" :disabled="fieldsLocked" />
              </div>
            </div>
            <div class="grid gap-4 sm:grid-cols-3">
              <div>
                <label for="field-r-shutdown-after" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Idle shutdown after</label>
                <input id="field-r-shutdown-after" v-model="remoteManager.shutdown_after" class="input font-mono" placeholder="10m" :disabled="fieldsLocked" />
              </div>
              <div>
                <label for="field-r-start-timeout" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Start timeout</label>
                <input id="field-r-start-timeout" v-model="remoteManager.start_timeout" class="input font-mono" placeholder="2m" :disabled="fieldsLocked" />
              </div>
              <div>
                <label for="field-r-poll" class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Poll interval</label>
                <input id="field-r-poll" v-model="remoteManager.poll_interval" class="input font-mono" placeholder="5s" :disabled="fieldsLocked" />
              </div>
            </div>
          </div>
        </div>

        <!-- Raw TOML -->
        <div class="grid gap-3 border-t border-[var(--ir-border)] pt-6">
          <div class="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h3 class="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Raw TOML</h3>
              <p class="mt-1 text-[11px] text-[var(--ir-text-muted)]">
                <template v-if="rawOverride && rawSeededFrom === 'fields'">
                  Started from the tabbed fields as they stood, edits included. Sent verbatim on save.
                </template>
                <template v-else-if="rawOverride">
                  Started from the file stored for this server, comments and all. Sent verbatim on save.
                </template>
                <template v-else-if="rawEditable && typedEdited">
                  Generated from the tabbed fields. Taking over carries your edits into the editor.
                </template>
                <template v-else-if="rawEditable">
                  Generated from the tabbed fields. Taking over loads the file stored for this server.
                </template>
                <template v-else>Generated from the tabbed fields. It follows every edit you make.</template>
              </p>
            </div>
            <div class="flex items-center gap-2">
              <button class="btn btn-secondary text-xs" type="button" :disabled="validating" @click="runValidation">
                {{ validating ? 'Validating...' : 'Validate now' }}
              </button>
              <button v-if="!rawOverride && rawEditable" class="btn btn-secondary text-xs" type="button" @click="startRawOverride">
                Take over manually
              </button>
              <template v-else-if="rawOverride">
                <button
                  v-if="rawToml && rawDraft !== rawToml"
                  class="btn btn-secondary text-xs"
                  type="button"
                  @click="loadServerFile"
                >
                  Load the stored file
                </button>
                <button class="btn btn-secondary text-xs" type="button" @click="discardRawOverride">
                  Back to fields
                </button>
              </template>
            </div>
          </div>

          <p v-if="!rawEditable" class="text-[11px] text-[var(--ir-text-muted)]">
            Manual TOML editing needs an existing server. Create this one first, then edit it here.
          </p>

          <textarea
            v-if="rawOverride"
            v-model="rawDraft"
            aria-label="Raw server TOML"
            spellcheck="false"
            rows="18"
            class="input font-mono text-xs leading-relaxed"
          />
          <textarea
            v-else
            :value="generatedToml"
            aria-label="Generated server TOML"
            readonly
            spellcheck="false"
            rows="18"
            class="input font-mono text-xs leading-relaxed opacity-80"
          />

          <!-- Validation panel -->
          <div class="rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-4">
            <p v-if="validationFailed" class="text-xs text-[var(--ir-text-muted)]">
              The proxy did not answer the validation request.
            </p>
            <p v-else-if="!validation" class="text-xs text-[var(--ir-text-muted)]">
              {{ validating ? 'Validating...' : 'Edit anything above to validate.' }}
            </p>
            <template v-else>
              <p
                class="flex items-center gap-2 text-sm font-medium"
                :class="validation.valid ? 'text-[#bce5b6]' : 'text-[#ffc0bc]'"
              >
                <CheckCircleIcon v-if="validation.valid" class="h-4 w-4" />
                <XCircleIcon v-else class="h-4 w-4" />
                {{ validation.valid ? 'Valid configuration' : `${validation.errors.length} error(s)` }}
              </p>

              <ul v-if="validation.errors.length" class="mt-3 grid gap-1.5">
                <li
                  v-for="(error, index) in validation.errors"
                  :key="`e-${index}`"
                  class="flex items-start gap-2 rounded-lg border border-[rgba(204,62,56,0.25)] bg-[rgba(204,62,56,0.1)] px-3 py-2 text-xs text-[#ffc0bc]"
                >
                  <XCircleIcon class="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  <span class="font-mono">{{ error }}</span>
                </li>
              </ul>

              <ul v-if="validation.warnings.length" class="mt-3 grid gap-1.5">
                <li
                  v-for="(warning, index) in validation.warnings"
                  :key="`w-${index}`"
                  class="flex items-start gap-2 rounded-lg border border-[rgba(233,160,71,0.25)] bg-[rgba(233,160,71,0.1)] px-3 py-2 text-xs text-[#ffd8ad]"
                >
                  <ExclamationTriangleIcon class="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  <span class="font-mono">{{ warning }}</span>
                </li>
              </ul>
            </template>
          </div>
        </div>
      </div>

      <div class="mt-6 flex justify-end gap-2 border-t border-[var(--ir-border)] pt-5">
        <button class="btn btn-secondary" type="button" @click="emit('cancel')">Cancel</button>
        <button class="btn btn-primary" type="button" :disabled="submitting" @click="submit">
          {{ submitting ? 'Saving...' : isEdit ? 'Update server' : 'Create server' }}
        </button>
      </div>
    </div>
  </div>
</template>
