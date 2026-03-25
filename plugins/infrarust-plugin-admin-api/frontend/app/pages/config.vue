<script setup lang="ts">
import {
  ArrowPathIcon,
  TrashIcon,
  PowerIcon,
} from '@heroicons/vue/24/outline';
import type { ProxyStatus, ProviderDto, ApiEnvelope, MutationResult } from '~/types/api';

const { request } = useApi();
const { push } = useToast();
const { ask } = useConfirm();

const providers = ref<ProviderDto[]>([]);
const proxyConfig = ref<ProxyStatus | null>(null);

await useAsyncData('config', async () => {
  const [providersRes, proxyRes] = await Promise.all([
    request<ApiEnvelope<ProviderDto[]>>('/config/providers').catch(() => ({ data: [] as ProviderDto[] })),
    request<ApiEnvelope<ProxyStatus>>('/proxy').catch(() => null),
  ]);
  providers.value = providersRes.data;
  if (proxyRes) proxyConfig.value = proxyRes.data;
  return true;
});

const operations = [
  {
    label: 'Reload Config',
    description: 'Re-read all configuration providers and apply changes.',
    icon: ArrowPathIcon,
    variant: 'accent' as const,
  },
  {
    label: 'Run GC',
    description: 'Trigger garbage collection to free unused memory.',
    icon: TrashIcon,
    variant: 'accent' as const,
  },
  {
    label: 'Shutdown Proxy',
    description: 'Gracefully terminate the proxy. This is irreversible.',
    icon: PowerIcon,
    variant: 'danger' as const,
  },
];

async function handleOperation(op: typeof operations[number]) {
  try {
    if (op.label === 'Reload Config') {
      await request<ApiEnvelope<MutationResult>>('/config/reload', { method: 'POST', body: {} });
      push({ type: 'success', title: 'Config reload requested' });
    } else if (op.label === 'Run GC') {
      await request<ApiEnvelope<MutationResult>>('/proxy/gc', { method: 'POST', body: {} });
      push({ type: 'success', title: 'GC completed' });
    } else if (op.label === 'Shutdown Proxy') {
      const first = await ask('Shutdown Proxy', 'Are you sure? This will disconnect all players.');
      if (!first) return;
      const second = await ask('Final Confirmation', 'This action cannot be undone. Proceed?');
      if (!second) return;
      await request<ApiEnvelope<MutationResult>>('/proxy/shutdown', { method: 'POST', body: {} });
      push({ type: 'success', title: 'Shutdown initiated' });
    }
  } catch (e: unknown) {
    const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? 'Operation failed';
    push({ type: 'error', title: msg });
  }
}
</script>

<template>
  <div class="grid gap-5">
    <h2 class="text-xl font-bold tracking-tight">Configuration</h2>

    <div class="grid gap-4 md:grid-cols-2">
      <div class="glass-pane p-5">
        <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Providers</h3>
        <div v-if="providers.length" class="space-y-2">
          <div v-for="p in providers" :key="p.provider_type" class="flex items-center justify-between text-sm">
            <span class="font-mono text-xs">{{ p.provider_type }}</span>
            <span class="rounded border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] px-2 py-0.5 text-[10px] text-[var(--ir-text-muted)]">
              {{ p.configs_count }} configs
            </span>
          </div>
        </div>
        <p v-else class="text-sm text-[var(--ir-text-muted)]">No providers loaded.</p>
      </div>
      <div class="glass-pane p-5">
        <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Proxy Config</h3>
        <div class="space-y-2 text-sm">
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Bind Address</span><span class="font-mono text-xs">{{ proxyConfig?.bind_address ?? '—' }}</span></div>
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Version</span><span class="font-mono text-xs">{{ proxyConfig?.version ?? '—' }}</span></div>
          <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Memory RSS</span><span class="font-mono text-xs">{{ proxyConfig?.memory_rss_bytes ? `${Math.round(proxyConfig.memory_rss_bytes / 1048576)} MB` : '—' }}</span></div>
          <div>
            <span class="text-[var(--ir-text-muted)]">Features</span>
            <div class="mt-1 flex flex-wrap gap-1">
              <span v-for="f in proxyConfig?.features ?? []" :key="f" class="accent-chip">{{ f }}</span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <div>
      <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Operations</h3>
      <div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <button
          v-for="op in operations"
          :key="op.label"
          class="glass-pane glass-pane-interactive flex items-start gap-4 p-5 text-left"
          :class="op.variant === 'danger' ? 'hover:border-[rgba(204,62,56,0.4)]' : ''"
          @click="handleOperation(op)"
        >
          <div
            class="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg"
            :class="op.variant === 'danger' ? 'bg-[rgba(204,62,56,0.14)]' : 'bg-[var(--ir-accent-soft)]'"
          >
            <component
              :is="op.icon"
              class="h-5 w-5"
              :class="op.variant === 'danger' ? 'text-[#cc3e38]' : 'text-[var(--ir-accent)]'"
            />
          </div>
          <div>
            <p class="font-semibold" :class="op.variant === 'danger' ? 'text-[#ffc0bc]' : ''">{{ op.label }}</p>
            <p class="mt-0.5 text-xs text-[var(--ir-text-muted)]">{{ op.description }}</p>
          </div>
        </button>
      </div>
    </div>
  </div>
</template>
