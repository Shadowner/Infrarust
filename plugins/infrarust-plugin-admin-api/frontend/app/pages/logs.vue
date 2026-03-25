<script setup lang="ts">
import {
  PauseIcon,
  PlayIcon,
  TrashIcon,
  ClipboardDocumentIcon,
  ArrowDownTrayIcon,
  FunnelIcon,
} from '@heroicons/vue/24/outline';
import type { LogEntryDto, ApiEnvelope } from '~/types/api';

const { request } = useApi();
const { push } = useToast();

const level = ref('info');
const targetFilter = ref('');
const searchQuery = ref('');
const showFilters = ref(false);

// Log stream via dedicated SSE connection
const {
  logs,
  status: sseStatus,
  paused,
  connect: connectStream,
  close: closeStream,
  pause: pauseStream,
  resume: resumeStream,
  clear: clearStreamLogs,
} = useLogStream();

// Fetch initial log history
async function fetchHistory() {
  try {
    const res = await request<ApiEnvelope<LogEntryDto[]>>('/logs/history', {
      query: { n: 500, level: level.value, target: targetFilter.value || undefined },
    });
    logs.value = res.data;
  } catch {
    // Log history may not be available
  }
  return true;
}

await useAsyncData('logs:history', fetchHistory);

// Re-fetch history + reconnect stream when filters change
watch([level, targetFilter], () => {
  clearStreamLogs();
  fetchHistory();
  closeStream();
  connectStream(level.value, targetFilter.value || undefined);
});

function togglePause() {
  if (paused.value) {
    resumeStream(level.value, targetFilter.value || undefined);
  } else {
    pauseStream();
  }
}

function clearLogs() {
  clearStreamLogs();
}

function copyLogs() {
  const text = filteredLogs.value.map((l) => `${l.timestamp} [${l.level.toUpperCase().padEnd(5)}] ${l.target ? `${l.target}: ` : ''}${l.message}`).join('\n');
  navigator.clipboard.writeText(text);
  push({ type: 'success', title: 'Copied to clipboard', message: `${filteredLogs.value.length} entries` });
}

function exportLogs() {
  const text = filteredLogs.value.map((l) => JSON.stringify(l)).join('\n');
  const blob = new Blob([text], { type: 'application/x-ndjson' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `infrarust-logs-${new Date().toISOString().slice(0, 19).replace(/:/g, '-')}.ndjson`;
  a.click();
  URL.revokeObjectURL(url);
  push({ type: 'success', title: 'Logs exported' });
}

const filteredLogs = computed(() => {
  if (!searchQuery.value) return logs.value;
  const q = searchQuery.value.toLowerCase();
  return logs.value.filter((l) =>
    l.message.toLowerCase().includes(q) ||
    (l.target?.toLowerCase().includes(q) ?? false)
  );
});

const errorCount = computed(() => logs.value.filter((l) => l.level.toLowerCase() === 'error').length);
const warnCount = computed(() => logs.value.filter((l) => l.level.toLowerCase() === 'warn').length);
</script>

<template>
  <div class="flex h-[calc(100vh-var(--ir-ticker-height)-3rem)] flex-col gap-3 lg:h-[calc(100vh-var(--ir-ticker-height)-3.5rem)]">
    <!-- Top bar -->
    <div class="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
      <div class="flex items-center gap-3">
        <h2 class="text-xl font-bold tracking-tight">Logs</h2>

        <!-- SSE status -->
        <div class="flex items-center gap-1.5 rounded-full border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] px-2.5 py-1">
          <span
            class="h-1.5 w-1.5 rounded-full"
            :class="sseStatus === 'open' && !paused ? 'bg-[var(--ir-success)] status-pulse' : paused ? 'bg-[var(--ir-warn)]' : 'bg-slate-500'"
          />
          <span class="text-[10px] font-medium uppercase tracking-wider text-[var(--ir-text-muted)]">
            {{ paused ? 'Paused' : sseStatus === 'open' ? 'Streaming' : 'Connecting' }}
          </span>
        </div>

        <!-- Level quick counts -->
        <div class="hidden items-center gap-1.5 md:flex">
          <span v-if="errorCount > 0" class="rounded bg-red-500/20 px-1.5 py-0.5 font-mono text-[10px] font-bold text-red-300">
            {{ errorCount }} ERR
          </span>
          <span v-if="warnCount > 0" class="rounded bg-orange-500/20 px-1.5 py-0.5 font-mono text-[10px] font-bold text-orange-300">
            {{ warnCount }} WRN
          </span>
          <span class="font-mono text-[10px] text-[var(--ir-text-muted)]">
            {{ filteredLogs.length }} lines
          </span>
        </div>
      </div>

      <!-- Controls -->
      <div class="flex items-center gap-1.5">
        <!-- Filter toggle -->
        <button
          class="btn btn-ghost flex items-center gap-1.5 text-xs"
          :class="showFilters ? 'text-[var(--ir-accent)]' : ''"
          @click="showFilters = !showFilters"
        >
          <FunnelIcon class="h-4 w-4" />
          <span class="hidden sm:inline">Filters</span>
        </button>

        <span class="h-4 w-px bg-[var(--ir-border)]" />

        <!-- Pause/Resume -->
        <button
          class="btn btn-ghost flex items-center gap-1.5 text-xs"
          :class="paused ? 'text-[var(--ir-warn)]' : ''"
          @click="togglePause"
        >
          <component :is="paused ? PlayIcon : PauseIcon" class="h-4 w-4" />
          <span class="hidden sm:inline">{{ paused ? 'Resume' : 'Pause' }}</span>
        </button>

        <!-- Clear -->
        <button class="btn btn-ghost flex items-center gap-1.5 text-xs" @click="clearLogs">
          <TrashIcon class="h-4 w-4" />
          <span class="hidden sm:inline">Clear</span>
        </button>

        <!-- Copy -->
        <button class="btn btn-ghost flex items-center gap-1.5 text-xs" @click="copyLogs">
          <ClipboardDocumentIcon class="h-4 w-4" />
          <span class="hidden sm:inline">Copy</span>
        </button>

        <!-- Export -->
        <button class="btn btn-ghost flex items-center gap-1.5 text-xs" @click="exportLogs">
          <ArrowDownTrayIcon class="h-4 w-4" />
          <span class="hidden sm:inline">Export</span>
        </button>
      </div>
    </div>

    <!-- Filter bar (collapsible) -->
    <Transition name="slide">
      <div v-if="showFilters" class="glass-pane flex flex-wrap items-center gap-3 p-3">
        <div class="flex items-center gap-2">
          <label class="text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--ir-text-muted)]">Level</label>
          <div class="flex gap-1">
            <button
              v-for="l in ['trace', 'debug', 'info', 'warn', 'error']"
              :key="l"
              class="rounded px-2 py-1 font-mono text-[10px] font-bold uppercase transition-colors"
              :class="level === l
                ? l === 'error' ? 'bg-red-500/25 text-red-300'
                  : l === 'warn' ? 'bg-orange-500/25 text-orange-300'
                  : l === 'info' ? 'bg-[var(--ir-accent-soft)] text-[var(--ir-accent)]'
                  : 'bg-slate-500/20 text-slate-300'
                : 'text-[var(--ir-text-muted)] hover:bg-white/[0.04]'
              "
              @click="level = l"
            >
              {{ l }}
            </button>
          </div>
        </div>

        <span class="h-5 w-px bg-[var(--ir-border)]" />

        <div class="flex items-center gap-2">
          <label class="text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--ir-text-muted)]">Target</label>
          <input
            v-model="targetFilter"
            class="input max-w-40 py-1 text-xs"
            placeholder="e.g. infrarust_core"
          />
        </div>

        <span class="h-5 w-px bg-[var(--ir-border)]" />

        <div class="flex items-center gap-2">
          <label class="text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--ir-text-muted)]">Search</label>
          <input
            v-model="searchQuery"
            class="input max-w-48 py-1 text-xs"
            placeholder="Filter messages..."
          />
        </div>

        <span v-if="searchQuery || targetFilter" class="ml-auto">
          <button
            class="text-[10px] text-[var(--ir-text-muted)] hover:text-white transition-colors"
            @click="searchQuery = ''; targetFilter = ''"
          >
            Clear filters
          </button>
        </span>
      </div>
    </Transition>

    <!-- Log console (fills remaining space) -->
    <div class="min-h-0 flex-1">
      <LogConsole :logs="filteredLogs" full-height />
    </div>
  </div>
</template>

<style scoped>
.slide-enter-active { transition: all 0.2s var(--ir-ease-spring); }
.slide-leave-active { transition: all 0.15s ease; }
.slide-enter-from, .slide-leave-to { opacity: 0; transform: translateY(-8px); max-height: 0; margin-bottom: 0; padding: 0; overflow: hidden; }
.slide-enter-to, .slide-leave-from { max-height: 100px; }
</style>
