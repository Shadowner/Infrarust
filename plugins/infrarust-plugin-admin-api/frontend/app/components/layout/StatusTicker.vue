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
  HeartIcon,
  ExclamationTriangleIcon,
  ArrowUpCircleIcon,
  ArrowPathRoundedSquareIcon,
} from '@heroicons/vue/24/outline';
import type { ApiEnvelope, ProxyStatus, UpdateStatusDto } from '~/types/api';

const UPDATE_REFRESH_INTERVAL = 30 * 60 * 1000;

const route = useRoute();
const auth = useAuth();
const { request } = useApi();
const { status: sseStatus, onEvent } = useEventBus();
const { now } = useTick();
const { pending: restartPending } = useRestartRequired();
const { restart: hotRestart } = useHotRestart();

const proxyStatus = ref<ProxyStatus | null>(null);
const pinging = ref(false);
const uptimeBaseSeconds = ref(0);
const uptimeBaseTime = ref(Date.now());
const updateStatus = ref<UpdateStatusDto | null>(null);
let updateTimer: ReturnType<typeof setInterval> | null = null;

const pageMap: Record<string, { label: string; icon: any }> = {
  '/dashboard': { label: 'Dashboard', icon: HomeIcon },
  '/players': { label: 'Players', icon: UsersIcon },
  '/bans': { label: 'Bans', icon: ShieldExclamationIcon },
  '/servers': { label: 'Servers', icon: ServerStackIcon },
  '/plugins': { label: 'Plugins', icon: PuzzlePieceIcon },
  '/logs': { label: 'Logs', icon: DocumentTextIcon },
  '/config/proxy': { label: 'Proxy Config', icon: Cog6ToothIcon },
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

const availableUpdate = computed(() =>
  updateStatus.value?.enabled ? updateStatus.value.available : null,
);
const updateApplied = computed(() => updateStatus.value?.binary_changed ?? false);
const updateNotesUrl = computed(() => {
  const url = availableUpdate.value?.notes_url;
  return url && /^https?:\/\//i.test(url) ? url : undefined;
});
const updateTitle = computed(() => {
  const update = availableUpdate.value;
  if (!update) return '';
  const kind = update.security ? 'Security update' : 'Update';
  const state = updateApplied.value ? 'installed, restart the proxy to apply it' : 'available';
  const notes = updateNotesUrl.value ? ' (opens the release notes)' : '';
  return `${kind} ${update.version} ${state}${notes}`;
});

const restartAvailable = computed(() => proxyStatus.value?.restart_available ?? false);
const restartHintTitle = computed(() =>
  restartAvailable.value
    ? 'A new binary is on disk. Hot restart the proxy to apply it'
    : 'A new binary is on disk. Restart the proxy to apply it',
);

function handleRestartHint() {
  if (restartAvailable.value) hotRestart();
}

async function loadUpdateStatus() {
  const res = await request<ApiEnvelope<UpdateStatusDto>>('/proxy/update').catch(() => null);
  if (res) updateStatus.value = res.data;
}

async function loadProxyStatus() {
  const res = await request<ApiEnvelope<ProxyStatus>>('/proxy').catch(() => null);
  if (!res) return;
  proxyStatus.value = res.data;
  uptimeBaseSeconds.value = res.data.uptime_seconds;
  uptimeBaseTime.value = Date.now();
}

let sseWasOpen = sseStatus.value === 'open';
watch(sseStatus, (status) => {
  if (status !== 'open') return;
  if (sseWasOpen) {
    loadProxyStatus();
    loadUpdateStatus();
  }
  sseWasOpen = true;
});

onMounted(() => {
  loadUpdateStatus();
  updateTimer = setInterval(loadUpdateStatus, UPDATE_REFRESH_INTERVAL);
  loadProxyStatus();
});

onBeforeUnmount(() => {
  if (updateTimer) clearInterval(updateTimer);
  updateTimer = null;
});

onEvent('stats.tick', (data) => {
  const d = data as Record<string, unknown>;
  if (proxyStatus.value) {
    if (typeof d.players_online === 'number') proxyStatus.value.players_online = d.players_online;
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
      <div class="flex items-center gap-1.5 text-xs text-[var(--ir-text-muted)]" title="Players online">
        <UsersIcon class="h-3.5 w-3.5" />
        <span class="font-mono text-[var(--ir-text)]">{{ proxyStatus?.players_online ?? 0 }}</span>
        <span class="sr-only">players online</span>
      </div>
      <span class="h-3 w-px bg-[var(--ir-border)]" />
      <div class="flex items-center gap-1.5 text-xs text-[var(--ir-text-muted)]" title="Configured servers">
        <ServerStackIcon class="h-3.5 w-3.5" />
        <span class="font-mono text-[var(--ir-text)]">{{ proxyStatus?.servers_count ?? 0 }}</span>
        <span class="sr-only">configured servers</span>
      </div>
    </div>

    <!-- Right: SSE indicator + actions -->
    <div class="flex items-center gap-3">
      <NuxtLink
        v-if="restartPending"
        to="/config/proxy"
        class="flex items-center gap-1.5 rounded-full border border-[rgba(233,160,71,0.35)] bg-[rgba(233,160,71,0.14)] px-2.5 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-[#ffd8ad]"
      >
        <ExclamationTriangleIcon class="h-3 w-3" />
        Restart required
      </NuxtLink>

      <component
        v-if="availableUpdate"
        :is="updateNotesUrl ? 'a' : 'span'"
        :href="updateNotesUrl"
        :target="updateNotesUrl ? '_blank' : undefined"
        :rel="updateNotesUrl ? 'noopener' : undefined"
        :title="updateTitle"
        class="flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-[10px] font-semibold uppercase tracking-wider"
        :class="availableUpdate.security
          ? 'border-[rgba(204,62,56,0.4)] bg-[rgba(204,62,56,0.16)] text-[#ffc0bc]'
          : 'border-[var(--ir-border-strong)] bg-[var(--ir-accent-soft)] text-[var(--ir-text)]'"
      >
        <component :is="availableUpdate.security ? ShieldExclamationIcon : ArrowUpCircleIcon" class="h-3 w-3" />
        {{ availableUpdate.security ? 'Security update' : 'Update' }}
        <span class="font-mono normal-case">{{ availableUpdate.version }}</span>
      </component>

      <component
        v-if="updateApplied"
        :is="restartAvailable ? 'button' : 'span'"
        :type="restartAvailable ? 'button' : undefined"
        :title="restartHintTitle"
        class="flex items-center gap-1.5 rounded-full border border-[rgba(233,160,71,0.35)] bg-[rgba(233,160,71,0.14)] px-2.5 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-[#ffd8ad]"
        :class="restartAvailable ? 'transition-colors hover:bg-[rgba(233,160,71,0.24)]' : ''"
        @click="handleRestartHint"
      >
        <ArrowPathRoundedSquareIcon class="h-3 w-3" />
        Restart to apply
      </component>

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

      <span class="h-3 w-px bg-[var(--ir-border)]" />

      <a
        href="https://ko-fi.com/C1C41WOEBB"
        target="_blank"
        rel="noopener"
        class="support-link group flex items-center gap-0 rounded-md px-2 py-1 text-[var(--ir-text-muted)] transition-all hover:gap-1.5"
      >
        <HeartIcon class="h-3.5 w-3.5 shrink-0 transition-colors group-hover:text-[var(--ir-danger)]" />
        <span class="max-w-0 overflow-hidden whitespace-nowrap text-xs opacity-0 transition-all duration-300 group-hover:max-w-[8rem] group-hover:opacity-100 group-hover:text-[var(--ir-danger)]">
          Support us
        </span>
      </a>
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
      <NuxtLink v-if="restartPending" to="/config/proxy" aria-label="Restart required" class="text-[var(--ir-warn)]">
        <ExclamationTriangleIcon class="h-4 w-4" />
      </NuxtLink>
      <component
        v-if="availableUpdate"
        :is="updateNotesUrl ? 'a' : 'span'"
        :href="updateNotesUrl"
        :target="updateNotesUrl ? '_blank' : undefined"
        :rel="updateNotesUrl ? 'noopener' : undefined"
        :aria-label="updateTitle"
        :title="updateTitle"
        :class="availableUpdate.security ? 'text-[var(--ir-danger)]' : 'text-[var(--ir-accent)]'"
      >
        <component :is="availableUpdate.security ? ShieldExclamationIcon : ArrowUpCircleIcon" class="h-4 w-4" />
      </component>
      <component
        v-if="updateApplied"
        :is="restartAvailable ? 'button' : 'span'"
        :type="restartAvailable ? 'button' : undefined"
        :aria-label="restartHintTitle"
        :title="restartHintTitle"
        class="text-[var(--ir-warn)]"
        @click="handleRestartHint"
      >
        <ArrowPathRoundedSquareIcon class="h-4 w-4" />
      </component>
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
