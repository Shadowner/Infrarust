<script setup lang="ts">
import { ArrowLeftIcon } from '@heroicons/vue/24/outline';
import type { PluginDto, ApiEnvelope, MutationResult } from '~/types/api';

const route = useRoute();
const { request } = useApi();
const { push } = useToast();
const pluginId = route.params.id as string;
const plugin = ref<PluginDto | null>(null);

const { data: pluginData } = await useAsyncData(`plugin:${pluginId}`, async () => {
  try {
    const res = await request<ApiEnvelope<PluginDto>>(`/plugins/${encodeURIComponent(pluginId)}`);
    return res.data;
  } catch {
    push({ type: 'error', title: 'Plugin not found' });
    return null;
  }
});
plugin.value = pluginData.value ?? null;

async function togglePlugin(action: 'enable' | 'disable') {
  try {
    await request<ApiEnvelope<MutationResult>>(`/plugins/${encodeURIComponent(pluginId)}/${action}`, {
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
  <div class="grid gap-5">
    <NuxtLink to="/plugins" class="inline-flex items-center gap-1.5 text-sm text-[var(--ir-text-muted)] hover:text-white transition-colors">
      <ArrowLeftIcon class="h-4 w-4" />
      Back to plugins
    </NuxtLink>

    <div class="glass-pane p-5">
      <div class="flex items-center justify-between">
        <div>
          <h2 class="text-xl font-bold tracking-tight">{{ plugin?.name ?? pluginId }}</h2>
          <p class="mt-1 font-mono text-xs text-[var(--ir-text-muted)]">v{{ plugin?.version ?? '—' }}</p>
        </div>
        <StatusBadge :status="plugin?.state ?? 'unknown'" />
      </div>
    </div>

    <div class="glass-pane p-5">
      <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Metadata</h3>
      <div class="space-y-2 text-sm">
        <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">ID</span><span class="font-mono text-xs">{{ plugin?.id ?? '—' }}</span></div>
        <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Authors</span><span>{{ plugin?.authors?.join(', ') || '—' }}</span></div>
        <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Description</span><span>{{ plugin?.description ?? '—' }}</span></div>
      </div>
    </div>

    <div class="glass-pane p-5">
      <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Dependencies</h3>
      <div v-if="plugin?.dependencies?.length" class="space-y-2">
        <div v-for="dep in plugin.dependencies" :key="dep.id" class="flex items-center justify-between text-sm">
          <span class="font-mono text-xs">{{ dep.id }}</span>
          <span class="text-[10px] uppercase text-[var(--ir-text-muted)]">{{ dep.optional ? 'optional' : 'required' }}</span>
        </div>
      </div>
      <p v-else class="text-sm text-[var(--ir-text-muted)]">No dependencies declared.</p>
    </div>

    <div class="flex gap-2">
      <button class="btn btn-secondary" @click="togglePlugin('enable')">Enable</button>
      <button class="btn btn-danger" @click="togglePlugin('disable')">Disable</button>
    </div>
  </div>
</template>
