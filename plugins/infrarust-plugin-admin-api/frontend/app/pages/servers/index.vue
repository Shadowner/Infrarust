<script setup lang="ts">
import {
  LockClosedIcon,
  PencilSquareIcon,
  PlayIcon,
  PlusIcon,
  StopIcon,
  TrashIcon,
} from '@heroicons/vue/24/outline';
import type { ApiEnvelope, MutationResult, ServerDto } from '~/types/api';
import type { ServerStateChangePayload } from '~/types/events';
import { resolveServerStatus } from '~/utils/serverStatus';

const { request } = useApi();
const { push } = useToast();
const { ask } = useConfirm();
const rows = ref<ServerDto[]>([]);

async function fetchServers() {
  try {
    const res = await request<ApiEnvelope<ServerDto[]>>('/servers');
    rows.value = res.data;
  } catch {
    push({ type: 'error', title: 'Failed to load servers' });
  }
  return true;
}

await useAsyncData('servers', fetchServers);

const { onEvent } = useEventBus();
onEvent('server.state_change', (data) => {
  const d = data as ServerStateChangePayload;
  if (d.server_id) {
    const srv = rows.value.find((s) => s.id === d.server_id);
    if (srv) srv.state = d.new_state;
  }
});

const allServerIds = computed(() => rows.value.map((s) => s.id));
const { getHealth, loading: healthLoading } = useServerHealth(allServerIds);

const sources = computed(() => {
  const seen = new Map<string, number>();
  for (const server of rows.value) {
    seen.set(server.source, (seen.get(server.source) ?? 0) + 1);
  }
  return [...seen.entries()].map(([source, count]) => ({ source, count }));
});

const sourceFilter = ref<string>('');
const visibleServers = computed(() =>
  sourceFilter.value ? rows.value.filter((s) => s.source === sourceFilter.value) : rows.value
);

const serverStatus = (server: ServerDto) => resolveServerStatus(server, getHealth(server.id));

function stateAccent(server: ServerDto) {
  const status = serverStatus(server);
  if (status === 'online') return 'border-l-[3px] border-l-[#5daf50]';
  if (['offline', 'crashed', 'unreachable'].includes(status)) return 'border-l-[3px] border-l-[#cc3e38]';
  if (['starting', 'stopping'].includes(status)) return 'border-l-[3px] border-l-[#e9a047]';
  return 'border-l-[3px] border-l-slate-500';
}

async function startServer(id: string) {
  try {
    await request<ApiEnvelope<MutationResult>>(`/servers/${encodeURIComponent(id)}/start`, { method: 'POST', body: {} });
    push({ type: 'success', title: `Start requested for ${id}` });
  } catch {
    push({ type: 'error', title: `Failed to start ${id}` });
  }
}

async function stopServer(id: string) {
  try {
    await request<ApiEnvelope<MutationResult>>(`/servers/${encodeURIComponent(id)}/stop`, { method: 'POST', body: {} });
    push({ type: 'success', title: `Stop requested for ${id}` });
  } catch {
    push({ type: 'error', title: `Failed to stop ${id}` });
  }
}

async function deleteServer(id: string) {
  const confirmed = await ask('Delete server', `Permanently delete '${id}'?`);
  if (!confirmed) return;
  try {
    await request<ApiEnvelope<MutationResult>>(`/servers/${encodeURIComponent(id)}`, { method: 'DELETE' });
    push({ type: 'success', title: `Deleted ${id}` });
    await fetchServers();
  } catch (e: unknown) {
    const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? 'Failed to delete';
    push({ type: 'error', title: msg });
  }
}
</script>

<template>
  <div class="grid gap-6">
    <div class="flex items-center justify-between">
      <div class="flex items-center gap-3">
        <h2 class="text-xl font-bold tracking-tight">Servers</h2>
        <span class="rounded-full border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] px-2.5 py-0.5 text-xs font-mono text-[var(--ir-text-muted)]">
          {{ rows.length }} nodes
        </span>
        <span v-if="healthLoading" class="text-[10px] text-[var(--ir-text-muted)] animate-pulse">pinging...</span>
      </div>
      <NuxtLink to="/servers/create" class="btn btn-primary flex items-center gap-1.5">
        <PlusIcon class="h-4 w-4" />
        <span class="hidden sm:inline">New server</span>
      </NuxtLink>
    </div>

    <div v-if="sources.length > 1" class="flex flex-wrap items-center gap-2">
      <span class="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Source</span>
      <button
        class="rounded-full border px-2.5 py-0.5 text-[11px] font-medium transition-colors"
        :class="sourceFilter === ''
          ? 'border-[var(--ir-border-strong)] bg-[var(--ir-accent-soft)] text-[var(--ir-accent)]'
          : 'border-[var(--ir-border)] text-[var(--ir-text-muted)] hover:text-white'"
        @click="sourceFilter = ''"
      >
        All
      </button>
      <button
        v-for="entry in sources"
        :key="entry.source"
        class="rounded-full border px-2.5 py-0.5 text-[11px] font-medium transition-colors"
        :class="sourceFilter === entry.source
          ? 'border-[var(--ir-border-strong)] bg-[var(--ir-accent-soft)] text-[var(--ir-accent)]'
          : 'border-[var(--ir-border)] text-[var(--ir-text-muted)] hover:text-white'"
        @click="sourceFilter = entry.source"
      >
        {{ entry.source }} ({{ entry.count }})
      </button>
    </div>

    <div v-if="visibleServers.length === 0" class="glass-pane p-6 text-center">
      <p class="text-sm text-[var(--ir-text-muted)]">No servers to show.</p>
      <NuxtLink to="/servers/create" class="mt-2 inline-flex items-center gap-1.5 text-sm text-[var(--ir-accent)] hover:underline">
        <PlusIcon class="h-4 w-4" /> Create one
      </NuxtLink>
    </div>

    <TransitionGroup v-else name="list" tag="div" class="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
      <NuxtLink
        v-for="server in visibleServers"
        :key="server.id"
        :to="`/servers/${server.id}`"
        class="glass-pane glass-pane-interactive overflow-hidden p-4"
        :class="stateAccent(server)"
      >
        <div class="flex items-center justify-between gap-2">
          <div class="flex min-w-0 items-center gap-2">
            <h3 class="truncate font-semibold">{{ server.id }}</h3>
            <span class="shrink-0 rounded bg-[var(--ir-surface-soft)] px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-[var(--ir-text-muted)]">
              {{ server.source }}
            </span>
            <span
              v-if="!server.editable"
              class="inline-flex shrink-0 items-center gap-1 text-[10px] text-[var(--ir-text-muted)]"
              title="Read-only: this server is owned by its config provider"
            >
              <LockClosedIcon class="h-3 w-3" />
              read-only
            </span>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <span v-if="getHealth(server.id)?.latency_ms != null" class="font-mono text-[10px] text-[var(--ir-text-muted)]">
              {{ getHealth(server.id)?.latency_ms }}ms
            </span>
            <StatusBadge :status="serverStatus(server)" />
          </div>
        </div>

        <div class="mt-2.5 space-y-1">
          <p class="truncate text-xs font-mono text-[var(--ir-text-muted)]">{{ server.addresses.join(', ') }}</p>
          <div class="flex items-center gap-2">
            <span class="text-[10px] text-[var(--ir-text-muted)]">{{ server.proxy_mode }}</span>
            <span class="h-1 w-1 rounded-full bg-[var(--ir-border)]" />
            <span class="text-[10px] text-[var(--ir-text-muted)]">{{ server.player_count }} players</span>
            <template v-if="getHealth(server.id)?.version_name">
              <span class="h-1 w-1 rounded-full bg-[var(--ir-border)]" />
              <span class="text-[10px] text-[var(--ir-text-muted)]">{{ getHealth(server.id)?.version_name }}</span>
            </template>
          </div>
        </div>

        <div class="mt-3 flex items-center gap-1.5" @click.prevent>
          <button v-if="server.has_server_manager" class="btn btn-ghost flex items-center gap-1 text-xs" @click="startServer(server.id)">
            <PlayIcon class="h-3.5 w-3.5" /> Start
          </button>
          <button v-if="server.has_server_manager" class="btn btn-ghost flex items-center gap-1 text-xs" @click="stopServer(server.id)">
            <StopIcon class="h-3.5 w-3.5" /> Stop
          </button>
          <NuxtLink v-if="server.editable" :to="`/servers/${server.id}/edit`" class="btn btn-ghost flex items-center gap-1 text-xs" @click.stop>
            <PencilSquareIcon class="h-3.5 w-3.5" /> Edit
          </NuxtLink>
          <button
            v-if="server.editable"
            class="btn btn-ghost ml-auto flex items-center gap-1 text-xs text-[var(--ir-danger)]"
            @click="deleteServer(server.id)"
          >
            <TrashIcon class="h-3.5 w-3.5" /> Delete
          </button>
        </div>
      </NuxtLink>
    </TransitionGroup>
  </div>
</template>
