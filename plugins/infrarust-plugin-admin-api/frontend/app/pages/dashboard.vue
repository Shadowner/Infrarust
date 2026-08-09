<script setup lang="ts">
import {
  ArrowPathIcon,
  MegaphoneIcon,
  TrashIcon,
  PowerIcon,
  ServerStackIcon,
} from '@heroicons/vue/24/outline';
import type { ProxyStatus, ServerDto, StatsDto, ApiEnvelope, MutationResult, RecentEventDto } from '~/types/api';
import { formatDuration } from '~/utils/time';
import { resolveServerStatus } from '~/utils/serverStatus';
import type { PlayerJoinPayload, PlayerLeavePayload, ServerStateChangePayload, BanEventPayload } from '~/types/events';

const { request } = useApi();
const { push } = useToast();
const { ask } = useConfirm();
const { onEvent } = useEventBus();
const { now } = useTick();
const proxyStatus = ref<ProxyStatus | null>(null);
const uptimeBaseSeconds = ref(0);
const uptimeBaseTime = ref(Date.now());
const stats = ref<StatsDto | null>(null);
const eventItems = ref<Array<{ type: string; timestamp?: string; summary?: string }>>([]);
const servers = ref<ServerDto[]>([]);
const showBroadcast = ref(false);

await useAsyncData('dashboard:proxy', async () => {
  const [proxyRes, serversRes, statsRes, recentRes] = await Promise.all([
    request<ApiEnvelope<ProxyStatus>>('/proxy'),
    request<ApiEnvelope<ServerDto[]>>('/servers').catch(() => ({ data: [] as ServerDto[] })),
    request<ApiEnvelope<StatsDto>>('/stats').catch(() => null),
    request<ApiEnvelope<RecentEventDto[]>>('/events/recent').catch(() => null),
  ]);
  proxyStatus.value = proxyRes.data;
  uptimeBaseSeconds.value = proxyRes.data.uptime_seconds;
  uptimeBaseTime.value = Date.now();
  servers.value = serversRes.data;
  if (statsRes) stats.value = statsRes.data;
  if (recentRes) {
    eventItems.value = recentRes.data.map((e) => ({
      type: e.type,
      timestamp: e.timestamp,
      summary: e.summary,
    }));
  }
  return proxyRes;
});

// Health checks with auto-refresh
const serverIds = computed(() => servers.value.map((s) => s.id));
const { getHealth } = useServerHealth(serverIds);

const liveUptime = computed(() => {
  if (!uptimeBaseSeconds.value) return '—';
  const elapsed = Math.floor((now.value - uptimeBaseTime.value) / 1000);
  return formatDuration(uptimeBaseSeconds.value + elapsed);
});

const serverStatus = (server: ServerDto) => resolveServerStatus(server, getHealth(server.id));

function addEvent(type: string, summary: string, timestamp?: string) {
  eventItems.value = [
    { type, timestamp: timestamp ?? new Date().toISOString(), summary },
    ...eventItems.value,
  ].slice(0, 20);
}

// Stats tick → update proxy counters
onEvent('stats.tick', (data) => {
  const d = data as Partial<StatsDto>;
  if (proxyStatus.value) {
    if (d.players_online !== undefined) proxyStatus.value = { ...proxyStatus.value, players_online: d.players_online };
  }
  if (stats.value) {
    stats.value = { ...stats.value, ...d };
  }
  if (typeof d.uptime_seconds === 'number') {
    uptimeBaseSeconds.value = d.uptime_seconds;
    uptimeBaseTime.value = Date.now();
  }
});

const fleetBreakdown = computed(() => [
  { label: 'online', count: stats.value?.servers_online ?? 0, dot: 'bg-[var(--ir-success)]' },
  { label: 'sleeping', count: stats.value?.servers_sleeping ?? 0, dot: 'bg-slate-400' },
  { label: 'offline', count: stats.value?.servers_offline ?? 0, dot: 'bg-[var(--ir-danger)]' },
]);

const playersOn = (id: string) => stats.value?.players_by_server?.[id] ?? 0;

const busiestServers = computed(() =>
  Object.entries(stats.value?.players_by_server ?? {})
    .filter(([, count]) => count > 0)
    .sort((a, b) => b[1] - a[1])
);

// Player events → event feed
onEvent('player.join', (data) => {
  const d = data as PlayerJoinPayload;
  addEvent('player.join', `${d.username} joined ${d.server}`, d.timestamp);
});
onEvent('player.leave', (data) => {
  const d = data as PlayerLeavePayload;
  addEvent('player.leave', `${d.username} disconnected`, d.timestamp);
});

// Server state changes → update dots + feed
onEvent('server.state_change', (data) => {
  const d = data as ServerStateChangePayload;
  const srv = servers.value.find((s) => s.id === d.server_id);
  if (srv) srv.state = d.new_state;
  addEvent('server.state_change', `${d.server_id}: ${d.old_state} → ${d.new_state}`, d.timestamp);
});

// Ban events → feed
onEvent('ban.created', (data) => {
  const d = data as BanEventPayload;
  addEvent('ban.created', `Banned ${d.target_value} (${d.target_type})`, d.timestamp);
});
onEvent('ban.removed', (data) => {
  const d = data as BanEventPayload;
  addEvent('ban.removed', `Unbanned ${d.target_value}`, d.timestamp);
});

async function handleAction(label: string) {
  try {
    if (label === 'Reload Config') {
      await request<ApiEnvelope<MutationResult>>('/config/reload', { method: 'POST', body: {} });
      push({ type: 'success', title: 'Config reload requested' });
    } else if (label === 'Broadcast') {
      showBroadcast.value = true;
      return;
    } else if (label === 'Run GC') {
      await request<ApiEnvelope<MutationResult>>('/proxy/gc', { method: 'POST', body: {} });
      push({ type: 'success', title: 'GC completed' });
    } else if (label === 'Shutdown') {
      const first = await ask('Shutdown Proxy', 'This will disconnect all players. Continue?');
      if (!first) return;
      const second = await ask('Final Confirmation', 'This cannot be undone. Proceed?');
      if (!second) return;
      await request<ApiEnvelope<MutationResult>>('/proxy/shutdown', { method: 'POST', body: {} });
      push({ type: 'success', title: 'Shutdown initiated' });
    }
  } catch (e: unknown) {
    const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? 'Operation failed';
    push({ type: 'error', title: msg });
  }
}

const quickActions = [
  { label: 'Reload Config', icon: ArrowPathIcon },
  { label: 'Broadcast', icon: MegaphoneIcon },
  { label: 'Run GC', icon: TrashIcon },
  { label: 'Shutdown', icon: PowerIcon },
];
</script>

<template>
  <div class="grid gap-5">
    <!-- Metric tiles -->
    <section class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
      <StatCard label="Version" :value="proxyStatus?.version ?? '—'" hint="Proxy build" />
      <StatCard label="Uptime" :value="liveUptime" hint="Since last restart" />
      <StatCard label="Players Online" :value="proxyStatus?.players_online ?? 0" hint="Active connections" />
      <StatCard
        label="Servers"
        :value="stats?.servers_total ?? proxyStatus?.servers_count ?? 0"
        :hint="`${stats?.servers_online ?? 0} online · ${stats?.servers_sleeping ?? 0} sleeping · ${stats?.servers_offline ?? 0} offline`"
      />
      <StatCard label="Active Bans" :value="stats?.bans_active ?? 0" hint="Currently enforced" />
    </section>

    <!-- Main grid: Fleet + Activity + Actions -->
    <section class="grid gap-4 lg:grid-cols-[2fr_1fr] xl:grid-cols-[3fr_2fr_auto]">
      <!-- Server Fleet Mini -->
      <div class="glass-pane p-5">
        <div class="mb-4 flex flex-wrap items-center gap-3">
          <div class="flex items-center gap-2">
            <ServerStackIcon class="h-4 w-4 text-[var(--ir-accent)]" />
            <h2 class="text-sm font-semibold uppercase tracking-[0.08em]">Server Fleet</h2>
          </div>
          <div v-if="stats" class="flex items-center gap-3">
            <span v-for="entry in fleetBreakdown" :key="entry.label" class="flex items-center gap-1.5 text-[10px] text-[var(--ir-text-muted)]">
              <span class="h-1.5 w-1.5 rounded-full" :class="entry.dot" />
              {{ entry.count }} {{ entry.label }}
            </span>
          </div>
        </div>
        <div class="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
          <NuxtLink
            v-for="server in servers"
            :key="server.id"
            :to="`/servers/${server.id}`"
            class="flex items-center gap-3 rounded-lg border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-3 transition-all duration-150 hover:border-[var(--ir-border-strong)] hover:bg-[var(--ir-surface)]"
          >
            <ServerStateDot :status="serverStatus(server)" />
            <div class="min-w-0 flex-1">
              <p class="truncate text-sm font-medium">{{ server.id }}</p>
              <p class="truncate text-[10px] text-[var(--ir-text-muted)]">
                <span class="uppercase tracking-[0.08em]">{{ serverStatus(server) }}</span>
                <template v-if="server.domains?.length"> · {{ server.domains.join(', ') }}</template>
              </p>
            </div>
            <span v-if="playersOn(server.id) > 0" class="shrink-0 rounded-full bg-[var(--ir-accent-soft)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--ir-accent)]">
              {{ playersOn(server.id) }}p
            </span>
            <span v-if="getHealth(server.id)?.latency_ms != null" class="shrink-0 font-mono text-[10px] text-[var(--ir-text-muted)]">
              {{ getHealth(server.id)?.latency_ms }}ms
            </span>
          </NuxtLink>

          <div v-if="servers.length === 0" class="col-span-full py-6 text-center text-sm text-[var(--ir-text-muted)]">
            No servers loaded yet.
          </div>
        </div>
      </div>

      <!-- Activity Feed -->
      <div class="glass-pane p-5">
        <h2 class="mb-4 text-sm font-semibold uppercase tracking-[0.08em]">Activity</h2>
        <EventFeed :items="eventItems" />

        <div v-if="busiestServers.length" class="mt-5 border-t border-[var(--ir-border)] pt-4">
          <h3 class="mb-2.5 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Players by server</h3>
          <div class="space-y-1.5">
            <NuxtLink
              v-for="[id, count] in busiestServers"
              :key="id"
              :to="`/servers/${id}`"
              class="flex items-center justify-between gap-3 text-xs text-[var(--ir-text-muted)] transition-colors hover:text-white"
            >
              <span class="truncate font-mono">{{ id }}</span>
              <span class="font-mono text-[var(--ir-text)]">{{ count }}</span>
            </NuxtLink>
          </div>
        </div>
      </div>

      <!-- Quick Actions -->
      <div class="glass-pane flex flex-col gap-1.5 p-4">
        <h2 class="mb-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--ir-text-muted)]">Actions</h2>
        <button
          v-for="action in quickActions"
          :key="action.label"
          class="flex items-center gap-2.5 rounded-md px-2.5 py-2 text-left transition-colors duration-100 hover:bg-white/[0.04]"
          :class="action.label === 'Shutdown' ? 'hover:bg-[rgba(204,62,56,0.08)]' : ''"
          @click="handleAction(action.label)"
        >
          <component
            :is="action.icon"
            class="h-4 w-4 shrink-0"
            :class="action.label === 'Shutdown' ? 'text-[#cc3e38]' : 'text-[var(--ir-accent)]'"
          />
          <span class="text-sm" :class="action.label === 'Shutdown' ? 'text-[#ffc0bc]' : ''">{{ action.label }}</span>
        </button>
      </div>
    </section>

    <BroadcastModal v-model="showBroadcast" />
  </div>
</template>
