<script setup lang="ts">
import {
  HomeIcon,
  UsersIcon,
  ShieldExclamationIcon,
  ServerStackIcon,
  PuzzlePieceIcon,
  DocumentTextIcon,
  Cog6ToothIcon,
  KeyIcon,
} from '@heroicons/vue/24/outline';
import type { ProxyStatus } from '~/types/api';

const route = useRoute();
const auth = useAuth();
const { request } = useApi();
const { status: sseStatus, onEvent } = useEventBus();
const { now } = useTick();

const proxyStatus = ref<ProxyStatus | null>(null);
const pinging = ref(false);
const uptimeBaseSeconds = ref(0);
const uptimeBaseTime = ref(Date.now());

const pageMap: Record<string, { label: string; icon: any }> = {
  '/dashboard': { label: 'Dashboard', icon: HomeIcon },
  '/players': { label: 'Players', icon: UsersIcon },
  '/bans': { label: 'Bans', icon: ShieldExclamationIcon },
  '/servers': { label: 'Servers', icon: ServerStackIcon },
  '/plugins': { label: 'Plugins', icon: PuzzlePieceIcon },
  '/logs': { label: 'Logs', icon: DocumentTextIcon },
  '/config': { label: 'Config', icon: Cog6ToothIcon },
};

const currentPage = computed(() => {
  for (const [path, meta] of Object.entries(pageMap)) {
    if (route.path === path || route.path.startsWith(path + '/')) return meta;
  }
  return { label: 'Dashboard', icon: HomeIcon };
});

import { formatDuration } from '~/utils/time';

const liveUptime = computed(() => {
  if (!uptimeBaseSeconds.value) return '-';
  const elapsed = Math.floor((now.value - uptimeBaseTime.value) / 1000);
  return formatDuration(uptimeBaseSeconds.value + elapsed);
});

onMounted(async () => {
  try {
    const res = await request<{ data: ProxyStatus }>('/proxy');
    proxyStatus.value = res.data;
    uptimeBaseSeconds.value = res.data.uptime_seconds;
    uptimeBaseTime.value = Date.now();
  } catch {
    // Will be populated by SSE
  }
});

onEvent('stats.tick', (data) => {
  const d = data as Record<string, unknown>;
  if (proxyStatus.value) {
    if (typeof d.players_online === 'number') proxyStatus.value.players_online = d.players_online;
    if (typeof d.servers_online === 'number') proxyStatus.value.servers_count = d.servers_online;
    if (typeof d.uptime_seconds === 'number') {
      uptimeBaseSeconds.value = d.uptime_seconds;
      uptimeBaseTime.value = Date.now();
    }
  }
  pinging.value = true;
  setTimeout(() => { pinging.value = false; }, 300);
});

function handleSwitchKey() {
  auth.clear();
  navigateTo('/login');
}
</script>

<template>
  <!-- Desktop ticker (hidden below lg) -->
  <header
    class="fixed top-0 z-[var(--ir-z-ticker)] hidden items-center justify-between border-b border-[var(--ir-border)] px-5 lg:flex"
    :style="{
      height: 'var(--ir-ticker-height)',
      left: 'var(--ir-rail-width)',
      right: '0',
      background: 'var(--ir-glass-bg-strong)',
      backdropFilter: 'blur(16px) saturate(1.3)',
      WebkitBackdropFilter: 'blur(16px) saturate(1.3)',
    }"
  >
    <!-- Left: Page breadcrumb -->
    <div class="flex items-center gap-2.5">
      <component :is="currentPage.icon" class="h-4 w-4 text-[var(--ir-accent)]" />
      <span class="text-sm font-semibold tracking-tight">{{ currentPage.label }}</span>
    </div>

    <!-- Center: Live stats -->
    <div class="flex items-center gap-5">
      <div class="flex items-center gap-2 text-xs text-[var(--ir-text-muted)]">
        <span class="font-mono text-[var(--ir-text)]">{{ proxyStatus?.version ?? '-' }}</span>
      </div>
      <span class="h-3 w-px bg-[var(--ir-border)]" />
      <div class="flex items-center gap-1.5 text-xs text-[var(--ir-text-muted)]">
        <span>Uptime</span>
        <span class="font-mono text-[var(--ir-text)]">{{ liveUptime }}</span>
      </div>
      <span class="h-3 w-px bg-[var(--ir-border)]" />
      <div class="flex items-center gap-1.5 text-xs text-[var(--ir-text-muted)]">
        <UsersIcon class="h-3.5 w-3.5" />
        <span class="font-mono text-[var(--ir-text)]">{{ proxyStatus?.players_online ?? 0 }}</span>
      </div>
      <span class="h-3 w-px bg-[var(--ir-border)]" />
      <div class="flex items-center gap-1.5 text-xs text-[var(--ir-text-muted)]">
        <ServerStackIcon class="h-3.5 w-3.5" />
        <span class="font-mono text-[var(--ir-text)]">{{ proxyStatus?.servers_count ?? 0 }}</span>
      </div>
    </div>

    <!-- Right: SSE indicator + actions -->
    <div class="flex items-center gap-3">
      <div class="flex items-center gap-2">
        <span
          class="h-2 w-2 rounded-full transition-transform duration-150"
          :class="[
            sseStatus === 'open' ? 'bg-[var(--ir-success)]' : 'bg-[var(--ir-danger)]',
            pinging ? 'sse-ping' : '',
          ]"
        />
        <span class="text-[10px] font-medium uppercase tracking-wider text-[var(--ir-text-muted)]">
          {{ sseStatus === 'open' ? 'Live' : 'Offline' }}
        </span>
      </div>

      <span class="h-3 w-px bg-[var(--ir-border)]" />

      <button
        class="flex items-center gap-1.5 rounded-md px-2 py-1 text-xs text-[var(--ir-text-muted)] transition-colors hover:bg-white/[0.04] hover:text-white"
        @click="handleSwitchKey"
      >
        <KeyIcon class="h-3.5 w-3.5" />
        Key
      </button>
    </div>
  </header>

  <!-- Mobile top bar (visible below lg) -->
  <header
    class="fixed inset-x-0 top-0 z-[var(--ir-z-ticker)] flex items-center justify-between border-b border-[var(--ir-border)] px-4 lg:hidden"
    :style="{
      height: 'var(--ir-ticker-height)',
      background: 'var(--ir-glass-bg-strong)',
      backdropFilter: 'blur(16px) saturate(1.3)',
      WebkitBackdropFilter: 'blur(16px) saturate(1.3)',
    }"
  >
    <div class="flex items-center gap-2.5">
      <img src="/logo.svg" alt="Infrarust" class="h-6 w-6" />
      <span class="text-sm font-semibold">{{ currentPage.label }}</span>
    </div>

    <div class="flex items-center gap-2">
      <span
        class="h-2 w-2 rounded-full"
        :class="[
          sseStatus === 'open' ? 'bg-[var(--ir-success)]' : 'bg-[var(--ir-danger)]',
          pinging ? 'sse-ping' : '',
        ]"
      />
    </div>
  </header>
</template>
