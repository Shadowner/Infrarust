<script setup lang="ts">
import { XMarkIcon } from '@heroicons/vue/24/outline';

const { toasts, dismiss } = useToast();

function accentBorder(type: string) {
  if (type === 'success') return 'border-l-[3px] border-l-[#5daf50]';
  if (type === 'error') return 'border-l-[3px] border-l-[#cc3e38]';
  return 'border-l-[3px] border-l-[var(--ir-accent)]';
}
</script>

<template>
  <div class="fixed top-[calc(var(--ir-ticker-height)+0.75rem)] right-4 z-[var(--ir-z-toast)] flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2">
    <TransitionGroup name="toast">
      <div
        v-for="toast in toasts"
        :key="toast.id"
        class="glass-pane overflow-hidden p-3"
        :class="accentBorder(toast.type)"
      >
        <div class="flex items-start justify-between gap-2">
          <div class="min-w-0">
            <p class="text-sm font-semibold">{{ toast.title }}</p>
            <p v-if="toast.message" class="mt-0.5 text-xs text-[var(--ir-text-muted)]">{{ toast.message }}</p>
          </div>
          <button
            class="shrink-0 rounded p-0.5 text-[var(--ir-text-muted)] transition-colors hover:bg-white/[0.06] hover:text-white"
            @click="dismiss(toast.id)"
          >
            <XMarkIcon class="h-4 w-4" />
          </button>
        </div>
        <!-- Auto-dismiss progress bar -->
        <div class="mt-2 h-[2px] w-full overflow-hidden rounded-full bg-white/[0.06]">
          <div
            class="h-full rounded-full bg-[var(--ir-accent)]"
            :style="{ animation: 'shrink 4s linear forwards' }"
          />
        </div>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
@keyframes shrink {
  from { width: 100%; }
  to { width: 0%; }
}
</style>
