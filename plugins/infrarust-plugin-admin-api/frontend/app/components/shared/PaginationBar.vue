<script setup lang="ts">
import { ChevronLeftIcon, ChevronRightIcon } from '@heroicons/vue/24/outline';

defineProps<{
  page: number;
  totalPages: number;
}>();

const emit = defineEmits<{
  prev: [];
  next: [];
}>();
</script>

<template>
  <div class="mt-4 flex items-center justify-between">
    <span class="text-xs font-mono text-[var(--ir-text-muted)]">
      Page {{ page }} / {{ totalPages }}
    </span>
    <div class="flex gap-1.5">
      <button
        class="btn btn-ghost flex items-center gap-1"
        :disabled="page <= 1"
        @click="emit('prev')"
      >
        <ChevronLeftIcon class="h-4 w-4" />
        <span class="text-sm">Prev</span>
      </button>
      <button
        class="btn btn-ghost flex items-center gap-1"
        :disabled="page >= totalPages"
        @click="emit('next')"
      >
        <span class="text-sm">Next</span>
        <ChevronRightIcon class="h-4 w-4" />
      </button>
    </div>
  </div>
</template>
