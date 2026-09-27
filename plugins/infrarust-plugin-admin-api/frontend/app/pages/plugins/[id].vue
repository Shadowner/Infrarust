<script setup lang="ts">
import { ArrowLeftIcon, CpuChipIcon } from '@heroicons/vue/24/outline';
import type { PluginDto, ApiEnvelope } from '~/types/api';

const route = useRoute();
const { request } = useApi();
const { push } = useToast();
const { onEvent } = useEventBus();
const { now } = useTick();
const pluginId = route.params.id as string;
const plugin = ref<PluginDto | null>(null);
const fetchedAt = ref(Date.now());

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

async function refreshPlugin() {
  try {
    const res = await request<ApiEnvelope<PluginDto>>(`/plugins/${encodeURIComponent(pluginId)}`);
    plugin.value = res.data;
    fetchedAt.value = Date.now();
  } catch {
    return;
  }
}

onMounted(() => {
  fetchedAt.value = Date.now();
});

onEvent('stats.tick', () => {
  if (Date.now() - fetchedAt.value >= PLUGIN_REFRESH_MS) refreshPlugin();
});

const runtime = computed(() => plugin.value?.runtime ?? null);

const nextAttempt = computed(() => {
  if (!runtime.value) return null;
  const seconds = secondsUntilRetry(runtime.value, fetchedAt.value, now.value);
  if (seconds == null) return null;
  return seconds === 0 ? 'now' : `in ${formatDuration(seconds)}`;
});

const faultAge = computed(() => {
  const fault = runtime.value?.last_fault;
  if (!fault) return null;
  return formatDuration(fault.secs_ago + Math.floor((now.value - fetchedAt.value) / 1000));
});
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
        <div class="flex items-center gap-1.5">
          <StatusBadge v-if="pluginNeedsAttention(runtime)" :status="runtime.health" />
          <StatusBadge :status="plugin?.state ?? 'unknown'" />
        </div>
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

    <div v-if="runtime" class="glass-pane p-5">
      <div class="mb-3 flex items-center gap-2">
        <CpuChipIcon class="h-4 w-4 text-[var(--ir-accent)]" />
        <h3 class="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Runtime</h3>
      </div>
      <div class="space-y-2 text-sm">
        <div class="flex items-center justify-between">
          <span class="text-[var(--ir-text-muted)]">Health</span>
          <span class="flex items-center gap-2">
            <span v-if="nextAttempt" class="font-mono text-xs text-[var(--ir-text-muted)]">next attempt {{ nextAttempt }}</span>
            <StatusBadge :status="runtime.health" />
          </span>
        </div>
        <div class="flex justify-between"><span class="text-[var(--ir-text-muted)]">Generation</span><span class="font-mono text-xs">{{ runtime.generation }}</span></div>
        <div class="flex justify-between">
          <span class="text-[var(--ir-text-muted)]">Restarts</span>
          <span class="font-mono text-xs">{{ runtime.restarts_in_window }} of {{ runtime.max_restarts }} in the last {{ formatDuration(runtime.restart_window_secs) }}</span>
        </div>
        <div class="flex justify-between">
          <span class="text-[var(--ir-text-muted)]">Queue</span>
          <span class="font-mono text-xs">{{ runtime.queue.depth }} of {{ runtime.queue.capacity }} waiting, at most {{ runtime.queue.peak_depth }} in the last {{ formatDuration(runtime.queue.window_secs) }}</span>
        </div>
        <div class="flex justify-between">
          <span class="text-[var(--ir-text-muted)]">Queue wait</span>
          <span v-if="runtime.queue.taken > 0" class="font-mono text-xs">
            p50 {{ formatWait(runtime.queue.wait_p50_us) }} · p99 {{ formatWait(runtime.queue.wait_p99_us) }} · max {{ formatWait(runtime.queue.wait_max_us) }} over {{ runtime.queue.taken }} calls
          </span>
          <span v-else class="text-xs text-[var(--ir-text-muted)]">no calls in the last {{ formatDuration(runtime.queue.window_secs) }}</span>
        </div>
        <div class="flex justify-between">
          <span class="text-[var(--ir-text-muted)]">Last fault</span>
          <span v-if="runtime.last_fault" class="font-mono text-xs">{{ faultAge }} ago, generation {{ runtime.last_fault.generation }}</span>
          <span v-else class="text-xs text-[var(--ir-text-muted)]">none since it was loaded</span>
        </div>
        <div v-if="runtime.last_fault" class="rounded-lg border border-[rgba(204,62,56,0.25)] bg-[rgba(204,62,56,0.1)] p-3 font-mono text-xs text-[#ffc0bc] break-words">
          {{ runtime.last_fault.cause }}
        </div>
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

  </div>
</template>
