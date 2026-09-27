<script setup lang="ts">
import type { PluginDto, ApiEnvelope } from '~/types/api';

const { request } = useApi();
const { push } = useToast();
const { onEvent } = useEventBus();
const { now } = useTick();
const rows = ref<PluginDto[]>([]);
const fetchedAt = ref(Date.now());

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

async function refreshPlugins() {
  try {
    const res = await request<ApiEnvelope<PluginDto[]>>('/plugins');
    rows.value = res.data;
    fetchedAt.value = Date.now();
  } catch {
    return;
  }
}

onMounted(() => {
  fetchedAt.value = Date.now();
});

onEvent('stats.tick', () => {
  if (Date.now() - fetchedAt.value >= PLUGIN_REFRESH_MS) refreshPlugins();
});
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
        <div class="flex items-center justify-between gap-2">
          <h3 class="font-semibold">{{ plugin.name }}</h3>
          <div class="flex items-center gap-1.5">
            <StatusBadge v-if="pluginNeedsAttention(plugin.runtime)" :status="plugin.runtime.health" />
            <StatusBadge :status="plugin.state" />
          </div>
        </div>
        <p class="mt-2 text-sm text-[var(--ir-text-muted)]">{{ plugin.description ?? 'No description' }}</p>
        <div class="mt-3 flex items-center">
          <div class="flex items-center gap-1.5">
            <span class="rounded border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] px-2 py-0.5 font-mono text-[10px] text-[var(--ir-text-muted)]">
              v{{ plugin.version }}
            </span>
            <span
              v-if="pluginNeedsAttention(plugin.runtime) && retryLabel(plugin.runtime, fetchedAt, now)"
              class="rounded border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] px-2 py-0.5 font-mono text-[10px] text-[var(--ir-text-muted)]"
            >
              {{ retryLabel(plugin.runtime, fetchedAt, now) }}
            </span>
          </div>
        </div>
      </NuxtLink>
    </div>
  </div>
</template>
