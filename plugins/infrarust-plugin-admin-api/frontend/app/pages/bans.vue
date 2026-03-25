<script setup lang="ts">
import {
  GlobeAltIcon,
  UserIcon,
  FingerPrintIcon,
  PlusIcon,
} from '@heroicons/vue/24/outline';
import type { BanDto, PaginatedEnvelope, ApiEnvelope, MutationResult } from '~/types/api';

const { request } = useApi();
const { push } = useToast();
const { ask } = useConfirm();
const search = ref('');
const showBanModal = ref(false);
const rows = ref<BanDto[]>([]);
const totalPages = ref(1);
const totalBans = ref(0);
const { page, perPage, next, prev } = usePagination();

const columns = [
  { key: 'target_type', label: 'Type' },
  { key: 'target_value', label: 'Target' },
  { key: 'reason', label: 'Reason' },
  { key: 'source', label: 'Source' },
  { key: 'permanent', label: 'Duration' },
];

const typeIcons: Record<string, unknown> = {
  ip: GlobeAltIcon,
  username: UserIcon,
  uuid: FingerPrintIcon,
};

async function fetchBans() {
  try {
    const res = await request<PaginatedEnvelope<BanDto>>('/bans', {
      query: { page: page.value, per_page: perPage.value },
    });
    rows.value = res.data;
    totalPages.value = res.meta.total_pages;
    totalBans.value = res.meta.total;
  } catch {
    push({ type: 'error', title: 'Failed to load bans' });
  }
  return true;
}

await useAsyncData('bans', fetchBans);
watch([page], fetchBans);

// SSE for live updates
const { onEvent } = useEventBus();
onEvent(['ban.created', 'ban.removed'], () => { fetchBans(); });

const filteredRows = computed(() => {
  if (!search.value) return rows.value;
  const q = search.value.toLowerCase();
  return rows.value.filter((r) => r.target_value.toLowerCase().includes(q));
});

async function unban(ban: BanDto) {
  const confirmed = await ask('Remove Ban', `Unban ${ban.target_value} (${ban.target_type})?`);
  if (!confirmed) return;
  try {
    await request<ApiEnvelope<MutationResult>>(`/bans/${encodeURIComponent(ban.target_type)}/${encodeURIComponent(ban.target_value)}`, {
      method: 'DELETE',
    });
    push({ type: 'success', title: `Unbanned ${ban.target_value}` });
    await fetchBans();
  } catch {
    push({ type: 'error', title: 'Failed to remove ban' });
  }
}

function onBanCreated() {
  showBanModal.value = false;
  fetchBans();
}
</script>

<template>
  <div class="grid gap-4">
    <div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
      <div class="flex items-center gap-3">
        <h2 class="text-xl font-bold tracking-tight">Bans</h2>
        <span class="rounded-full border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] px-2.5 py-0.5 text-xs font-mono text-[var(--ir-text-muted)]">
          {{ totalBans }} active
        </span>
      </div>
      <div class="flex items-center gap-2">
        <SearchInput v-model="search" placeholder="Search ban target..." />
        <button class="btn btn-primary flex shrink-0 items-center gap-1.5" @click="showBanModal = true">
          <PlusIcon class="h-4 w-4" />
          <span class="hidden sm:inline">New Ban</span>
        </button>
      </div>
    </div>

    <DataTable :columns="columns" :rows="filteredRows">
      <template #cell-target_type="{ value }">
        <div class="flex items-center gap-2">
          <component :is="(typeIcons[value as string] ?? UserIcon) as any" class="h-4 w-4 text-[var(--ir-text-muted)]" />
          <span class="text-xs font-semibold uppercase">{{ value }}</span>
        </div>
      </template>
      <template #cell-target_value="{ value }">
        <span class="font-mono text-xs">{{ value }}</span>
      </template>
      <template #cell-permanent="{ row }">
        <span v-if="(row as BanDto).permanent" class="text-xs text-[var(--ir-danger)]">Permanent</span>
        <span v-else class="text-xs text-[var(--ir-text-muted)]">{{ (row as BanDto).expires_in ?? '—' }}</span>
      </template>
      <template #actions="{ row }">
        <button class="btn btn-ghost text-xs text-[var(--ir-danger)]" @click="unban(row as BanDto)">Unban</button>
      </template>
    </DataTable>

    <PaginationBar :page="page" :total-pages="totalPages" @next="next" @prev="prev" />
    <BanFormModal v-model="showBanModal" @created="onBanCreated" />
  </div>
</template>
