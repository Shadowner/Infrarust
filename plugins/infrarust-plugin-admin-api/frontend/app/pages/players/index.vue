<script setup lang="ts">
import { MegaphoneIcon } from '@heroicons/vue/24/outline';
import type { PlayerDto, PaginatedEnvelope } from '~/types/api';
import { formatDurationSince } from '~/utils/time';

const { request } = useApi();
const { now } = useTick();
const { push } = useToast();
const search = ref('');
const rows = ref<PlayerDto[]>([]);
const totalPages = ref(1);
const playerCount = ref(0);
const { page, perPage, next, prev } = usePagination();
const showBroadcast = ref(false);

// Modal state
const modalTarget = ref('');
const showKick = ref(false);
const showSend = ref(false);
const showMessage = ref(false);

function openKick(username: string) { modalTarget.value = username; showKick.value = true; }
function openSend(username: string) { modalTarget.value = username; showSend.value = true; }
function openMessage(username: string) { modalTarget.value = username; showMessage.value = true; }

const columns = [
  { key: 'username', label: 'Username' },
  { key: 'uuid', label: 'UUID' },
  { key: 'server', label: 'Server' },
  { key: 'connected_since', label: 'Duration' },
  { key: 'is_active', label: 'Mode' },
];

async function fetchPlayers() {
  try {
    const res = await request<PaginatedEnvelope<PlayerDto>>('/players', {
      query: { page: page.value, per_page: perPage.value },
    });
    rows.value = res.data;
    totalPages.value = res.meta.total_pages;
    playerCount.value = res.meta.total;
  } catch {
    push({ type: 'error', title: 'Failed to load players' });
  }
  return true;
}

await useAsyncData('players', fetchPlayers);
watch([page], fetchPlayers);

const { onEvent } = useEventBus();
onEvent('player.leave', () => { fetchPlayers(); });
onEvent('player.join', () => { setTimeout(fetchPlayers, 500); });

const filteredRows = computed(() => {
  if (!search.value) return rows.value;
  const q = search.value.toLowerCase();
  return rows.value.filter((r) => r.username.toLowerCase().includes(q) || r.uuid.toLowerCase().includes(q));
});
</script>

<template>
  <div class="grid gap-4">
    <div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
      <div class="flex items-center gap-3">
        <h2 class="text-xl font-bold tracking-tight">Players</h2>
        <span class="rounded-full border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] px-2.5 py-0.5 text-xs font-mono text-[var(--ir-text-muted)]">
          {{ playerCount }} online
        </span>
      </div>
      <div class="flex items-center gap-2">
        <SearchInput v-model="search" placeholder="Search player..." />
        <button class="btn btn-secondary flex shrink-0 items-center gap-1.5" @click="showBroadcast = true">
          <MegaphoneIcon class="h-4 w-4" />
          <span class="hidden sm:inline">Broadcast</span>
        </button>
      </div>
    </div>

    <DataTable :columns="columns" :rows="filteredRows">
      <template #cell-username="{ row }">
        <NuxtLink :to="`/players/${(row as PlayerDto).username}`" class="flex items-center gap-2 hover:text-[var(--ir-accent)]">
          <img
            :src="`https://minotar.net/helm/${(row as PlayerDto).username}/24`"
            :alt="(row as PlayerDto).username"
            class="h-6 w-6 rounded"
            loading="lazy"
          />
          <span class="font-medium">{{ (row as PlayerDto).username }}</span>
        </NuxtLink>
      </template>
      <template #cell-uuid="{ value }">
        <span class="font-mono text-xs text-[var(--ir-text-muted)]">{{ value }}</span>
      </template>
      <template #cell-connected_since="{ value }">
        <span class="font-mono text-xs text-[var(--ir-text-muted)]">{{ formatDurationSince(value as string, now) }}</span>
      </template>
      <template #cell-is_active="{ value, row }">
        <StatusBadge :status="value ? 'Intercepted' : 'Passthrough'" />
      </template>
      <template #actions="{ row }">
        <div class="flex gap-1">
          <button class="btn btn-ghost text-xs" @click="openKick((row as PlayerDto).username)">Kick</button>
          <button class="btn btn-ghost text-xs" :disabled="!(row as PlayerDto).is_active" @click="openSend((row as PlayerDto).username)">Send</button>
          <button class="btn btn-ghost text-xs" :disabled="!(row as PlayerDto).is_active" @click="openMessage((row as PlayerDto).username)">Msg</button>
        </div>
      </template>
    </DataTable>

    <PaginationBar :page="page" :total-pages="totalPages" @next="next" @prev="prev" />

    <!-- Modals -->
    <BroadcastModal v-model="showBroadcast" />
    <KickPlayerModal v-model="showKick" :username="modalTarget" @done="fetchPlayers" />
    <SendPlayerModal v-model="showSend" :username="modalTarget" @done="fetchPlayers" />
    <MessagePlayerModal v-model="showMessage" :username="modalTarget" />
  </div>
</template>
