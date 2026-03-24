<script setup lang="ts">
import type { PluginDto, ApiEnvelope, MutationResult } from '~/types/api';

const { request } = useApi();
const { push } = useToast();
const rows = ref<PluginDto[]>([]);

const { data: pluginsData } = await useAsyncData('plugins', async () => {
  try {
    const res = await request<ApiEnvelope<PluginDto[]>>('/plugins');
    return res.data;
  } catch {
    push({ type: 'error', title: 'Failed to load plugins' });
    return [];
  }
});
rows.value = pluginsData.value ?? [];

async function togglePlugin(plugin: PluginDto) {
  const action = plugin.state === 'enabled' ? 'disable' : 'enable';
  try {
    await request<ApiEnvelope<MutationResult>>(`/plugins/${encodeURIComponent(plugin.id)}/${action}`, {
      method: 'POST', body: {},
    });
    push({ type: 'success', title: `Plugin ${action}d` });
  } catch (e: unknown) {
    const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? `Failed to ${action} plugin`;
    push({ type: 'error', title: msg });
  }
}
</script>

<template>
  <div class="grid gap-4">
    <div class="flex items-center gap-3">
      <h2 class="text-xl font-bold tracking-tight">Plugins</h2>
      <span class="rounded-full border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] px-2.5 py-0.5 text-xs font-mono text-[var(--ir-text-muted)]">
        {{ rows.length }} loaded
      </span>
    </div>

    <div class="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
      <NuxtLink
        v-for="plugin in rows"
        :key="plugin.id"
        :to="`/plugins/${plugin.id}`"
        class="glass-pane glass-pane-interactive p-4"
      >
        <div class="flex items-center justify-between">
          <h3 class="font-semibold">{{ plugin.name }}</h3>
          <StatusBadge :status="plugin.state" />
        </div>
        <p class="mt-2 text-sm text-[var(--ir-text-muted)]">{{ plugin.description ?? 'No description' }}</p>
        <div class="mt-3 flex items-center justify-between">
          <span class="rounded border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] px-2 py-0.5 font-mono text-[10px] text-[var(--ir-text-muted)]">
            v{{ plugin.version }}
          </span>
          <button
            class="relative h-6 w-11 rounded-full transition-colors"
            :class="plugin.state === 'enabled' ? 'bg-[#5daf50]' : 'bg-slate-600'"
            @click.prevent="togglePlugin(plugin)"
          >
            <span
              class="absolute top-0.5 h-5 w-5 rounded-full bg-white shadow-sm transition-transform"
              :class="plugin.state === 'enabled' ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>
      </NuxtLink>
    </div>
  </div>
</template>
