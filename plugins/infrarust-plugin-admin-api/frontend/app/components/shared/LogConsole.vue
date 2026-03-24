<script setup lang="ts">
import { ArrowDownIcon } from '@heroicons/vue/24/outline';
import type { LogEntryDto } from '~/types/api';

const props = defineProps<{
  logs: LogEntryDto[];
  fullHeight?: boolean;
}>();

const follow = ref(true);
const container = ref<HTMLElement | null>(null);

function scrollToBottom() {
  if (follow.value && container.value) {
    container.value.scrollTop = container.value.scrollHeight;
  }
}

watch(() => props.logs.length, () => {
  nextTick(scrollToBottom);
});

function handleScroll() {
  if (!container.value) return;
  const { scrollTop, scrollHeight, clientHeight } = container.value;
  follow.value = scrollHeight - scrollTop - clientHeight < 40;
}

function levelColor(level: string) {
  const l = level.toLowerCase();
  if (l === 'error') return { border: 'border-l-red-500', badge: 'bg-red-500/20 text-red-300', text: 'text-red-200/90' };
  if (l === 'warn') return { border: 'border-l-orange-500', badge: 'bg-orange-500/20 text-orange-300', text: 'text-orange-100/80' };
  if (l === 'info') return { border: 'border-l-[var(--ir-accent)]', badge: 'bg-[var(--ir-accent-soft)] text-[var(--ir-accent)]', text: 'text-[var(--ir-text)]' };
  if (l === 'debug') return { border: 'border-l-slate-600', badge: 'bg-slate-500/15 text-slate-400', text: 'text-slate-400' };
  return { border: 'border-l-slate-700', badge: 'bg-slate-600/15 text-slate-500', text: 'text-slate-500' };
}

function formatTimestamp(ts: string) {
  try {
    const d = new Date(ts);
    return d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', second: '2-digit', fractionalSecondDigits: 3 });
  } catch {
    return ts;
  }
}
</script>

<template>
  <div class="flex h-full flex-col">
    <!-- Terminal header -->
    <div class="terminal-chrome flex items-center justify-between">
      <div class="flex items-center gap-2">
        <span class="terminal-dot terminal-dot--close" />
        <span class="terminal-dot terminal-dot--minimize" />
        <span class="terminal-dot terminal-dot--maximize" />
        <span class="ml-2 text-xs text-[var(--ir-text-muted)]">Runtime Logs</span>
      </div>
      <div class="flex items-center gap-3">
        <span class="font-mono text-[10px] text-[var(--ir-text-muted)]">{{ logs.length }} lines</span>
        <span
          class="h-1.5 w-1.5 rounded-full"
          :class="follow ? 'bg-[var(--ir-success)] status-pulse' : 'bg-slate-500'"
        />
      </div>
    </div>

    <!-- Log body -->
    <div
      ref="container"
      class="terminal-body flex-1 overflow-auto font-mono text-xs"
      :class="fullHeight ? '' : 'h-[520px]'"
      @scroll="handleScroll"
    >
      <!-- Log entries -->
      <table class="w-full border-collapse">
        <tbody>
          <tr
            v-for="(log, index) in logs"
            :key="index"
            class="group border-l-2 transition-colors hover:bg-white/[0.02]"
            :class="[levelColor(log.level).border, index === logs.length - 1 ? 'log-flash' : '']"
          >
            <!-- Line number -->
            <td class="w-[50px] select-none py-1 pl-2 pr-1 text-right align-top text-slate-700 group-hover:text-slate-600">
              {{ index + 1 }}
            </td>
            <!-- Timestamp -->
            <td class="w-[90px] py-1 px-1.5 align-top text-slate-500 whitespace-nowrap">
              {{ formatTimestamp(log.timestamp) }}
            </td>
            <!-- Level badge -->
            <td class="w-[58px] py-1 px-1 align-top">
              <span
                class="inline-block w-[50px] rounded px-1.5 py-0.5 text-center text-[9px] font-bold uppercase tracking-[0.06em]"
                :class="levelColor(log.level).badge"
              >
                {{ log.level }}
              </span>
            </td>
            <!-- Target -->
            <td v-if="log.target" class="w-[140px] py-1 px-1 align-top truncate text-slate-600 max-w-[140px]">
              {{ log.target }}
            </td>
            <!-- Message -->
            <td class="py-1 px-1.5 align-top break-all" :class="levelColor(log.level).text">
              {{ log.message }}
            </td>
          </tr>
        </tbody>
      </table>

      <div v-if="logs.length === 0" class="flex h-full items-center justify-center py-20">
        <div class="text-center">
          <p class="text-sm text-[var(--ir-text-muted)]">Waiting for log entries...</p>
          <p class="mt-1 text-[10px] text-slate-600">Logs will appear here in real-time once the stream connects.</p>
        </div>
      </div>
    </div>

    <!-- Scroll-to-bottom button -->
    <Transition name="fade">
      <div v-if="!follow && logs.length > 0" class="flex justify-center -mt-12 relative z-10 pointer-events-none pb-2">
        <button
          class="pointer-events-auto flex items-center gap-1.5 rounded-full glass-pane px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-[var(--ir-accent)] hover:bg-[var(--ir-accent-soft)] transition-colors shadow-lg"
          @click="follow = true; scrollToBottom()"
        >
          <ArrowDownIcon class="h-3 w-3" />
          Follow
        </button>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.fade-enter-active { transition: opacity 0.2s ease, transform 0.2s ease; }
.fade-leave-active { transition: opacity 0.15s ease, transform 0.15s ease; }
.fade-enter-from, .fade-leave-to { opacity: 0; transform: translateY(4px); }
</style>
