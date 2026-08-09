<script setup lang="ts">
import { ArrowDownTrayIcon, ArrowPathIcon, PlayIcon } from '@heroicons/vue/24/outline';
import type { ApiEnvelope, BackendState, BackendStatus, MutationResult } from '~/types/api';
import { formatDuration } from '~/utils/time';

const props = defineProps<{
  serverId: string;
  backends: BackendStatus[];
  strategy?: string;
  locked?: boolean;
}>();

const emit = defineEmits<{ changed: [] }>();

const { request } = useApi();
const { push } = useToast();
const { ask } = useConfirm();

const pending = ref<string | null>(null);

const stateStyles: Record<BackendState, { shape: string; label: string; text: string }> = {
  healthy: {
    shape: 'rounded-full bg-[var(--ir-success)]',
    label: 'Healthy',
    text: 'text-[#bce5b6]',
  },
  probing: {
    shape: 'rounded-full border-2 border-[var(--ir-warn)] backend-probe-pulse',
    label: 'Probing',
    text: 'text-[#ffd8ad]',
  },
  unhealthy: {
    shape: 'rounded-[2px] bg-[var(--ir-danger)]',
    label: 'Unhealthy',
    text: 'text-[#ffc0bc]',
  },
  draining: {
    shape: 'rotate-45 rounded-[1px] bg-slate-400',
    label: 'Draining',
    text: 'text-slate-200',
  },
};

const style = (state: BackendState) => stateStyles[state] ?? stateStyles.unhealthy;

const columns = [
  { key: 'address', label: 'Address' },
  { key: 'state', label: 'State' },
  { key: 'weight', label: 'Weight' },
  { key: 'connections', label: 'Conns' },
  { key: 'ejections', label: 'Ejections' },
  { key: 'since', label: 'Healthy since' },
];

function weightLabel(backend: BackendStatus): string {
  return backend.effective_weight === backend.weight
    ? String(backend.weight)
    : `${backend.effective_weight} / ${backend.weight}`;
}

function ramping(backend: BackendStatus): boolean {
  return backend.effective_weight !== backend.weight;
}

function sinceLabel(backend: BackendStatus): string {
  if (backend.healthy_since_secs == null) {
    return backend.last_failure_secs_ago == null
      ? '—'
      : `failed ${formatDuration(backend.last_failure_secs_ago)} ago`;
  }
  return formatDuration(backend.healthy_since_secs);
}

const actionCopy: Record<'drain' | 'enable' | 'reset', { title: string; message: string }> = {
  drain: {
    title: 'Drain backend',
    message: 'Stop sending new connections to this address. Existing ones keep running.',
  },
  enable: {
    title: 'Enable backend',
    message: 'Put this address back into rotation.',
  },
  reset: {
    title: 'Reset backend',
    message: 'Clear the ejection counters and the slow-start ramp for this address.',
  },
};

async function act(backend: BackendStatus, action: 'drain' | 'enable' | 'reset') {
  const copy = actionCopy[action];
  const confirmed = await ask(copy.title, `${backend.address} — ${copy.message}`);
  if (!confirmed) return;

  pending.value = `${backend.address}:${action}`;
  try {
    await request<ApiEnvelope<MutationResult>>(
      `/servers/${encodeURIComponent(props.serverId)}/backends/${encodeURIComponent(backend.address)}/${action}`,
      { method: 'POST', body: {} }
    );
    push({ type: 'success', title: `${copy.title} — ${backend.address}` });
    emit('changed');
  } catch (e: unknown) {
    const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? `Failed to ${action}`;
    push({ type: 'error', title: msg });
  } finally {
    pending.value = null;
  }
}

const busy = (backend: BackendStatus, action: string) => pending.value === `${backend.address}:${action}`;
</script>

<template>
  <div class="grid gap-3">
    <div v-if="strategy" class="flex items-center gap-2">
      <span class="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Strategy</span>
      <span class="accent-chip">{{ strategy }}</span>
    </div>

    <!-- Desktop table -->
    <div class="glass-pane hidden overflow-auto lg:block">
      <table class="min-w-full text-left text-sm">
        <caption class="sr-only">Backend addresses for {{ serverId }}</caption>
        <thead>
          <tr class="border-b border-[var(--ir-border)]" style="background: var(--ir-glass-bg-strong);">
            <th
              v-for="column in columns"
              :key="column.key"
              scope="col"
              class="sticky top-0 px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.09em] text-[var(--ir-text-muted)]"
              style="background: var(--ir-glass-bg-strong);"
            >
              {{ column.label }}
            </th>
            <th
              v-if="!locked"
              scope="col"
              class="sticky top-0 px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.09em] text-[var(--ir-text-muted)]"
              style="background: var(--ir-glass-bg-strong);"
            >
              Actions
            </th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="backend in backends"
            :key="backend.address"
            class="border-t border-[var(--ir-border)] text-[var(--ir-text)] odd:bg-white/[0.02] transition-colors duration-100 hover:bg-[rgba(232,131,42,0.04)]"
          >
            <td class="px-4 py-2.5 font-mono text-xs">{{ backend.address }}</td>
            <td class="px-4 py-2.5">
              <span class="inline-flex items-center gap-2" :class="style(backend.state).text">
                <span class="h-2.5 w-2.5 shrink-0" :class="style(backend.state).shape" />
                <span class="text-[11px] font-semibold uppercase tracking-[0.08em]">{{ style(backend.state).label }}</span>
              </span>
            </td>
            <td class="px-4 py-2.5 font-mono text-xs">
              {{ weightLabel(backend) }}
              <span v-if="ramping(backend)" class="ml-1 text-[10px] text-[var(--ir-warn)]">ramping</span>
            </td>
            <td class="px-4 py-2.5 font-mono text-xs">{{ backend.active_connections }}</td>
            <td class="px-4 py-2.5 font-mono text-xs">{{ backend.ejections }}</td>
            <td class="px-4 py-2.5 text-xs text-[var(--ir-text-muted)]">{{ sinceLabel(backend) }}</td>
            <td v-if="!locked" class="px-4 py-2.5">
              <div class="flex items-center gap-1">
                <button
                  v-if="backend.state !== 'draining'"
                  class="btn btn-ghost flex items-center gap-1 text-xs"
                  :disabled="busy(backend, 'drain')"
                  @click="act(backend, 'drain')"
                >
                  <ArrowDownTrayIcon class="h-3.5 w-3.5" /> Drain
                </button>
                <button
                  v-else
                  class="btn btn-ghost flex items-center gap-1 text-xs"
                  :disabled="busy(backend, 'enable')"
                  @click="act(backend, 'enable')"
                >
                  <PlayIcon class="h-3.5 w-3.5" /> Enable
                </button>
                <button
                  class="btn btn-ghost flex items-center gap-1 text-xs"
                  :disabled="busy(backend, 'reset')"
                  @click="act(backend, 'reset')"
                >
                  <ArrowPathIcon class="h-3.5 w-3.5" :class="busy(backend, 'reset') ? 'animate-spin' : ''" /> Reset
                </button>
              </div>
            </td>
          </tr>
          <tr v-if="backends.length === 0">
            <td :colspan="locked ? columns.length : columns.length + 1" class="px-4 py-10 text-center text-sm text-[var(--ir-text-muted)]">
              No backend addresses reported.
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Mobile card list -->
    <div class="flex flex-col gap-3 lg:hidden">
      <div v-if="backends.length === 0" class="glass-pane p-6 text-center text-sm text-[var(--ir-text-muted)]">
        No backend addresses reported.
      </div>
      <div v-for="backend in backends" :key="backend.address" class="glass-pane p-4">
        <div class="flex items-center justify-between gap-3">
          <span class="font-mono text-xs">{{ backend.address }}</span>
          <span class="inline-flex items-center gap-2" :class="style(backend.state).text">
            <span class="h-2.5 w-2.5 shrink-0" :class="style(backend.state).shape" />
            <span class="text-[11px] font-semibold uppercase tracking-[0.08em]">{{ style(backend.state).label }}</span>
          </span>
        </div>
        <dl class="mt-3 space-y-2">
          <div class="flex items-start justify-between gap-3">
            <dt class="text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--ir-text-muted)]">Weight</dt>
            <dd class="font-mono text-xs">{{ weightLabel(backend) }}</dd>
          </div>
          <div class="flex items-start justify-between gap-3">
            <dt class="text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--ir-text-muted)]">Conns</dt>
            <dd class="font-mono text-xs">{{ backend.active_connections }}</dd>
          </div>
          <div class="flex items-start justify-between gap-3">
            <dt class="text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--ir-text-muted)]">Ejections</dt>
            <dd class="font-mono text-xs">{{ backend.ejections }}</dd>
          </div>
          <div class="flex items-start justify-between gap-3">
            <dt class="text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--ir-text-muted)]">Healthy since</dt>
            <dd class="text-xs text-[var(--ir-text-muted)]">{{ sinceLabel(backend) }}</dd>
          </div>
        </dl>
        <div v-if="!locked" class="mt-3 flex items-center gap-2 border-t border-[var(--ir-border)] pt-3">
          <button
            v-if="backend.state !== 'draining'"
            class="btn btn-ghost flex items-center gap-1 text-xs"
            :disabled="busy(backend, 'drain')"
            @click="act(backend, 'drain')"
          >
            <ArrowDownTrayIcon class="h-3.5 w-3.5" /> Drain
          </button>
          <button
            v-else
            class="btn btn-ghost flex items-center gap-1 text-xs"
            :disabled="busy(backend, 'enable')"
            @click="act(backend, 'enable')"
          >
            <PlayIcon class="h-3.5 w-3.5" /> Enable
          </button>
          <button
            class="btn btn-ghost flex items-center gap-1 text-xs"
            :disabled="busy(backend, 'reset')"
            @click="act(backend, 'reset')"
          >
            <ArrowPathIcon class="h-3.5 w-3.5" /> Reset
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
