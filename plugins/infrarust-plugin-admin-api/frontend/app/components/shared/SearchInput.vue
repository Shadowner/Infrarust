<script setup lang="ts">
import { MagnifyingGlassIcon } from '@heroicons/vue/24/outline';

const modelValue = defineModel<string>({ default: '' });
const props = withDefaults(defineProps<{ placeholder?: string }>(), {
  placeholder: 'Search...',
});
</script>

<template>
  <div class="input-with-icon">
    <MagnifyingGlassIcon class="input-icon" />
    <input
      v-model="modelValue"
      class="input pr-10"
      :placeholder="props.placeholder"
    />
    <kbd class="absolute right-3 top-1/2 -translate-y-1/2 rounded border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] px-1.5 py-0.5 text-[10px] font-mono text-[var(--ir-text-muted)]">/</kbd>
  </div>
</template>
