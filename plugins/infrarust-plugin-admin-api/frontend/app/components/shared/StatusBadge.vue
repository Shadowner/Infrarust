<script setup lang="ts">
const props = defineProps<{
  status: string;
}>();

const tooltips: Record<string, string> = {
  intercepted: 'The proxy can read and modify packets for this player. Commands like send, message, and server transfer are available.',
  passthrough: 'Traffic is encrypted end-to-end, the proxy forwards packets without inspecting them. Only kick and ban are available.',
};

const tooltip = computed(() => tooltips[props.status.toLowerCase()]);
const hovered = ref(false);
const badgeEl = ref<HTMLElement | null>(null);
const tooltipPos = reactive({ top: 0, left: 0 });

function showTooltip() {
  if (!tooltip.value || !badgeEl.value) return;
  const rect = badgeEl.value.getBoundingClientRect();
  tooltipPos.top = rect.top - 8;
  tooltipPos.left = rect.left + rect.width / 2;
  hovered.value = true;
}

function hideTooltip() {
  hovered.value = false;
}

const config = computed(() => {
  const normalized = props.status.toLowerCase();
  if (['online', 'enabled', 'ok', 'intercepted'].includes(normalized))
    return { dot: 'bg-[#5daf50]', classes: 'border-[rgba(93,175,80,0.4)] bg-[rgba(93,175,80,0.16)] text-[#bce5b6]', pulse: normalized !== 'intercepted' };
  if (['sleeping', 'offline', 'disabled', 'passthrough'].includes(normalized))
    return { dot: 'bg-slate-400', classes: 'border-[rgba(148,163,184,0.28)] bg-[rgba(148,163,184,0.15)] text-slate-200', pulse: false };
  if (['starting', 'stopping', 'warn'].includes(normalized))
    return { dot: 'bg-[#e9a047]', classes: 'border-[rgba(233,160,71,0.34)] bg-[rgba(233,160,71,0.15)] text-[#ffd8ad]', pulse: true };
  if (['crashed', 'error'].includes(normalized))
    return { dot: 'bg-[#cc3e38]', classes: 'border-[rgba(204,62,56,0.4)] bg-[rgba(204,62,56,0.16)] text-[#ffc0bc]', pulse: true };
  return { dot: 'bg-[var(--ir-accent)]', classes: 'border-[var(--ir-border-strong)] bg-[var(--ir-accent-soft)] text-[var(--ir-text)]', pulse: false };
});
</script>

<template>
  <span
    ref="badgeEl"
    class="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-semibold uppercase tracking-[0.08em]"
    :class="[config.classes, tooltip ? 'cursor-help' : '']"
    @mouseenter="showTooltip"
    @mouseleave="hideTooltip"
  >
    <span class="h-1.5 w-1.5 rounded-full" :class="[config.dot, config.pulse ? 'status-pulse' : '']" />
    {{ status }}
  </span>

  <!-- Tooltip teleported to body so it's never clipped -->
  <Teleport to="body">
    <Transition name="fade">
      <div
        v-if="hovered && tooltip"
        class="pointer-events-none fixed z-[9999] w-56 -translate-x-1/2 -translate-y-full rounded-lg border border-[var(--ir-border)] bg-[var(--ir-bg-dark)] px-3 py-2 text-[11px] font-normal normal-case tracking-normal leading-relaxed text-[var(--ir-text)] shadow-lg"
        :style="{ top: `${tooltipPos.top}px`, left: `${tooltipPos.left}px` }"
      >
        {{ tooltip }}
        <span class="absolute left-1/2 top-full -translate-x-1/2 border-4 border-transparent border-t-[var(--ir-bg-dark)]" />
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.fade-enter-active { transition: opacity 0.15s ease; }
.fade-leave-active { transition: opacity 0.1s ease; }
.fade-enter-from, .fade-leave-to { opacity: 0; }
</style>
