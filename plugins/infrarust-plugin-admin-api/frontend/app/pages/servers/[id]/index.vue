<script setup lang="ts">
import {
  PlayIcon,
  StopIcon,
  PencilSquareIcon,
  TrashIcon,
  ArrowLeftIcon,
  HeartIcon,
  ArrowPathIcon,
} from '@heroicons/vue/24/outline';
import type { ServerDetailDto, HealthCheckDto, ApiEnvelope, MutationResult } from '~/types/api';

const route = useRoute();
const { request } = useApi();
const { push } = useToast();
const { ask } = useConfirm();
const serverId = route.params.id as string;
const server = ref<ServerDetailDto | null>(null);
const health = ref<HealthCheckDto | null>(null);
const healthLoading = ref(false);

const { data } = await useAsyncData(`server:${serverId}`, async () => {
  try {
    const res = await request<ApiEnvelope<ServerDetailDto>>(`/servers/${encodeURIComponent(serverId)}`);
    return res.data;
  } catch {
    push({ type: 'error', title: 'Server not found' });
    return null;
  }
});
server.value = data.value ?? null;

// Try to load cached health check
onMounted(async () => {
  try {
    const res = await request<ApiEnvelope<HealthCheckDto>>(`/servers/${encodeURIComponent(serverId)}/health/cached`);
    health.value = res.data;
  } catch {
    // No cached health — that's fine
  }
});

async function checkHealth() {
  healthLoading.value = true;
  try {
    const res = await request<ApiEnvelope<HealthCheckDto>>(`/servers/${encodeURIComponent(serverId)}/health`);
    health.value = res.data;
    if (res.data.online) {
      push({ type: 'success', title: 'Server is online', message: `${res.data.latency_ms}ms latency` });
    } else {
      push({ type: 'error', title: 'Server is offline', message: res.data.error ?? undefined });
    }
  } catch {
    push({ type: 'error', title: 'Health check failed' });
  } finally {
    healthLoading.value = false;
  }
}

async function startServer() {
  try {
    await request<ApiEnvelope<MutationResult>>(`/servers/${encodeURIComponent(serverId)}/start`, { method: 'POST', body: {} });
    push({ type: 'success', title: 'Start requested' });
  } catch {
    push({ type: 'error', title: 'Failed to start server' });
  }
}

async function stopServer() {
  try {
    await request<ApiEnvelope<MutationResult>>(`/servers/${encodeURIComponent(serverId)}/stop`, { method: 'POST', body: {} });
    push({ type: 'success', title: 'Stop requested' });
  } catch {
    push({ type: 'error', title: 'Failed to stop server' });
  }
}

async function deleteServer() {
  const confirmed = await ask('Delete Server', `Permanently delete server '${serverId}'? This cannot be undone.`);
  if (!confirmed) return;
  try {
    await request<ApiEnvelope<MutationResult>>(`/servers/${encodeURIComponent(serverId)}`, { method: 'DELETE' });
    push({ type: 'success', title: `Server '${serverId}' deleted` });
    navigateTo('/servers');
  } catch (e: unknown) {
    const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? 'Failed to delete server';
    push({ type: 'error', title: msg });
  }
}

function relativeTime(ts: string) {
  try {
    const diff = Date.now() - new Date(ts).getTime();
    if (diff < 60000) return `${Math.floor(diff / 1000)}s ago`;
    if (diff < 3600000) return `${Math.floor(diff / 60000)}m ago`;
    return `${Math.floor(diff / 3600000)}h ago`;
  } catch {
    return ts;
  }
}
</script>

<template>
  <div class="grid gap-5">
    <NuxtLink to="/servers" class="inline-flex items-center gap-1.5 text-sm text-[var(--ir-text-muted)] hover:text-white transition-colors">
      <ArrowLeftIcon class="h-4 w-4" />
      Back to servers
    </NuxtLink>

    <!-- Header -->
    <div class="glass-pane flex items-center justify-between p-5">
      <div class="flex items-center gap-3">
        <div>
          <div class="flex items-center gap-2">
            <h2 class="text-xl font-bold tracking-tight">{{ serverId }}</h2>
            <span v-if="server?.is_api_managed" class="rounded bg-[var(--ir-accent-soft)] px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-[var(--ir-accent)]">
              API
            </span>
          </div>
          <p class="mt-1 text-xs text-[var(--ir-text-muted)]">{{ server?.proxy_mode ?? '—' }} mode</p>
        </div>
      </div>
      <StatusBadge :status="server?.state ?? 'unknown'" />
    </div>

    <!-- Health Check -->
    <div class="glass-pane overflow-hidden p-5">
      <div class="flex items-center justify-between mb-4">
        <div class="flex items-center gap-2">
          <HeartIcon class="h-4 w-4 text-[var(--ir-accent)]" />
          <h3 class="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Health Check</h3>
        </div>
        <button
          class="btn btn-ghost flex items-center gap-1.5 text-xs"
          :disabled="healthLoading"
          @click="checkHealth"
        >
          <ArrowPathIcon class="h-3.5 w-3.5" :class="healthLoading ? 'animate-spin' : ''" />
          {{ healthLoading ? 'Checking...' : 'Check Now' }}
        </button>
      </div>

      <!-- Health result -->
      <div v-if="health" class="space-y-4">
        <!-- Status bar -->
        <div class="flex items-center gap-3">
          <span
            class="h-3 w-3 rounded-full"
            :class="health.online ? 'bg-[var(--ir-success)] status-pulse' : 'bg-[var(--ir-danger)]'"
          />
          <span class="text-sm font-medium">{{ health.online ? 'Online' : 'Offline' }}</span>
          <span v-if="health.latency_ms != null" class="rounded bg-[var(--ir-surface-soft)] px-2 py-0.5 font-mono text-[10px] text-[var(--ir-text-muted)]">
            {{ health.latency_ms }}ms
          </span>
          <span v-if="health.version_name" class="rounded bg-[var(--ir-surface-soft)] px-2 py-0.5 font-mono text-[10px] text-[var(--ir-text-muted)]">
            {{ health.version_name }}
          </span>
          <span class="ml-auto text-[10px] text-[var(--ir-text-muted)]">{{ relativeTime(health.checked_at) }}</span>
        </div>

        <!-- Error -->
        <div v-if="health.error" class="rounded-lg bg-[rgba(204,62,56,0.1)] border border-[rgba(204,62,56,0.25)] p-3 text-xs text-[#ffc0bc]">
          {{ health.error }}
        </div>

        <!-- MOTD + Favicon -->
        <div v-if="health.online && health.motd" class="flex items-start gap-4">
          <img
            v-if="health.favicon"
            :src="health.favicon"
            alt="Server icon"
            class="h-16 w-16 rounded-lg border border-[var(--ir-border)]"
            style="image-rendering: pixelated;"
          />
          <div class="min-w-0 flex-1 rounded-lg bg-[#080808] border border-[var(--ir-border)] p-3">
            <MinecraftMotd :motd="health.motd" />
          </div>
        </div>

        <!-- Players -->
        <div v-if="health.online && health.players_online != null" class="flex items-center gap-3 text-sm">
          <span class="text-[var(--ir-text-muted)]">Players</span>
          <span class="font-mono">{{ health.players_online }} / {{ health.players_max }}</span>
          <div v-if="health.player_sample?.length" class="flex items-center gap-1.5 ml-2">
            <span
              v-for="p in health.player_sample.slice(0, 5)"
              :key="p.id"
              class="rounded bg-[var(--ir-surface-soft)] px-1.5 py-0.5 text-[10px] font-mono text-[var(--ir-text-muted)]"
            >
              {{ p.name }}
            </span>
            <span v-if="(health.player_sample?.length ?? 0) > 5" class="text-[10px] text-[var(--ir-text-muted)]">
              +{{ (health.player_sample?.length ?? 0) - 5 }} more
            </span>
          </div>
        </div>
      </div>

      <!-- Empty state -->
      <div v-else class="py-4 text-center text-sm text-[var(--ir-text-muted)]">
        Click "Check Now" to ping this server and retrieve its status.
      </div>
    </div>

    <!-- Config + Players -->
    <div class="grid gap-4 md:grid-cols-2">
      <div class="glass-pane p-5">
        <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Configuration</h3>
        <div class="space-y-2 text-sm">
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Addresses</span><span class="font-mono text-xs">{{ server?.addresses?.join(', ') ?? '—' }}</span></div>
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Domains</span><span class="font-mono text-xs">{{ server?.domains?.join(', ') ?? '—' }}</span></div>
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Mode</span><span>{{ server?.proxy_mode ?? '—' }}</span></div>
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Handlers</span><span class="text-xs">{{ server?.limbo_handlers?.join(', ') || 'none' }}</span></div>
          <div class="flex justify-between">
            <span class="text-[var(--ir-text-muted)]">Server Manager</span>
            <StatusBadge :status="server?.has_server_manager ? 'Enabled' : 'Disabled'" />
          </div>
        </div>
      </div>
      <div class="glass-pane p-5">
        <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Connected Players ({{ server?.player_count ?? 0 }})</h3>
        <div v-if="server?.players?.length" class="space-y-2">
          <div v-for="p in server.players" :key="p.uuid" class="flex items-center gap-2 text-sm">
            <img :src="`https://minotar.net/helm/${p.username}/20`" class="h-5 w-5 rounded" loading="lazy" />
            <NuxtLink :to="`/players/${p.username}`" class="hover:text-[var(--ir-accent)]">{{ p.username }}</NuxtLink>
          </div>
        </div>
        <p v-else class="text-sm text-[var(--ir-text-muted)]">No players connected.</p>
      </div>
    </div>

    <!-- Actions -->
    <div class="flex flex-wrap items-center gap-2">
      <template v-if="server?.has_server_manager">
        <button class="btn btn-secondary flex items-center gap-2" @click="startServer">
          <PlayIcon class="h-4 w-4" /> Start
        </button>
        <button class="btn btn-secondary flex items-center gap-2" @click="stopServer">
          <StopIcon class="h-4 w-4" /> Stop
        </button>
      </template>

      <template v-if="server?.is_api_managed">
        <NuxtLink :to="`/servers/${serverId}/edit`" class="btn btn-secondary flex items-center gap-2">
          <PencilSquareIcon class="h-4 w-4" /> Edit
        </NuxtLink>
        <button class="btn btn-danger flex items-center gap-2 ml-auto" @click="deleteServer">
          <TrashIcon class="h-4 w-4" /> Delete
        </button>
      </template>

      <p v-if="!server?.has_server_manager && !server?.is_api_managed" class="text-xs text-[var(--ir-text-muted)]">
        This server is managed by a config provider and cannot be edited or controlled from here.
      </p>
    </div>
  </div>
</template>
