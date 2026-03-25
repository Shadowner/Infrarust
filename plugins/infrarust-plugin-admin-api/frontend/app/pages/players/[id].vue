<script setup lang="ts">
import {
  XMarkIcon,
  PaperAirplaneIcon,
  ChatBubbleLeftIcon,
  ArrowLeftIcon,
} from '@heroicons/vue/24/outline';
import type { PlayerDetailDto, ApiEnvelope } from '~/types/api';
import { formatDurationSince } from '~/utils/time';

const route = useRoute();
const { request } = useApi();
const { now } = useTick();
const { push } = useToast();
const playerId = route.params.id as string;
const player = ref<PlayerDetailDto | null>(null);

const showKick = ref(false);
const showSend = ref(false);
const showMessage = ref(false);

const { data: playerData } = await useAsyncData(`player:${playerId}`, async () => {
  try {
    const res = await request<ApiEnvelope<PlayerDetailDto>>(`/players/${encodeURIComponent(playerId)}`);
    return res.data;
  } catch {
    push({ type: 'error', title: 'Player not found' });
    return null;
  }
});
player.value = playerData.value ?? null;

const username = computed(() => player.value?.username ?? playerId);

function onKicked() {
  navigateTo('/players');
}
</script>

<template>
  <div class="grid gap-5">
    <NuxtLink to="/players" class="inline-flex items-center gap-1.5 text-sm text-[var(--ir-text-muted)] hover:text-white transition-colors">
      <ArrowLeftIcon class="h-4 w-4" />
      Back to players
    </NuxtLink>

    <!-- Player header card -->
    <div class="glass-pane p-5">
      <div class="flex items-center gap-4">
        <img
          :src="`https://minotar.net/helm/${username}/48`"
          :alt="username"
          class="h-12 w-12 rounded-lg"
          loading="lazy"
        />
        <div>
          <h2 class="text-xl font-bold tracking-tight">{{ username }}</h2>
          <p class="text-xs font-mono text-[var(--ir-text-muted)]">{{ player?.uuid ?? '—' }}</p>
        </div>
        <div class="ml-auto">
          <StatusBadge :status="player?.is_active ? 'Intercepted' : 'Passthrough'" />
        </div>
      </div>
    </div>

    <!-- Info grid -->
    <div class="grid gap-4 md:grid-cols-2">
      <div class="glass-pane p-5">
        <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Identity</h3>
        <div class="space-y-2 text-sm">
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Username</span><span>{{ player?.username ?? '—' }}</span></div>
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">UUID</span><span class="font-mono text-xs">{{ player?.uuid ?? '—' }}</span></div>
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">IP</span><span class="font-mono text-xs">{{ player?.ip ?? '—' }}</span></div>
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Protocol</span><span>{{ player?.protocol_version ?? '—' }}</span></div>
        </div>
      </div>
      <div class="glass-pane p-5">
        <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Session</h3>
        <div class="space-y-2 text-sm">
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Server</span><span>{{ player?.server ?? '—' }}</span></div>
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Duration</span><span class="font-mono">{{ player?.connected_since ? formatDurationSince(player.connected_since, now) : '—' }}</span></div>
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Connected Since</span><span class="text-xs">{{ player?.connected_since ?? '—' }}</span></div>
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Mode</span><StatusBadge :status="player?.is_active ? 'Intercepted' : 'Passthrough'" /></div>
        </div>
      </div>
    </div>

    <!-- Actions -->
    <div class="glass-pane p-5">
      <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Actions</h3>
      <div class="flex flex-wrap gap-2">
        <button class="btn btn-danger flex items-center gap-2" @click="showKick = true">
          <XMarkIcon class="h-4 w-4" /> Kick
        </button>
        <button class="btn btn-secondary flex items-center gap-2" :disabled="!player?.is_active" @click="showSend = true">
          <PaperAirplaneIcon class="h-4 w-4" /> Send to Server
        </button>
        <button class="btn btn-secondary flex items-center gap-2" :disabled="!player?.is_active" @click="showMessage = true">
          <ChatBubbleLeftIcon class="h-4 w-4" /> Message
        </button>
        <p v-if="!player?.is_active" class="text-xs text-[var(--ir-text-muted)]">
          Passthrough mode — send and message are unavailable.
        </p>
      </div>
    </div>

    <!-- Modals -->
    <KickPlayerModal v-model="showKick" :username="username" @done="onKicked" />
    <SendPlayerModal v-model="showSend" :username="username" />
    <MessagePlayerModal v-model="showMessage" :username="username" />
  </div>
</template>
