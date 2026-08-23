<script setup lang="ts">
import { MagnifyingGlassIcon } from '@heroicons/vue/24/outline';

const modelValue = defineModel<string>({ default: '' });
const props = withDefaults(defineProps<{ placeholder?: string }>(), {
  placeholder: 'Search...',
});

const field = ref<HTMLInputElement | null>(null);

function focusOnSlash(event: KeyboardEvent) {
  if (event.key !== '/' || event.ctrlKey || event.metaKey || event.altKey) return;
  const target = event.target as HTMLElement | null;
  if (target?.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target?.tagName ?? '')) return;
  event.preventDefault();
  field.value?.focus();
}

onMounted(() => window.addEventListener('keydown', focusOnSlash));
onBeforeUnmount(() => window.removeEventListener('keydown', focusOnSlash));
</script>

<template>
  <div class="input-with-icon">
    <MagnifyingGlassIcon class="input-icon" />
    <input
      ref="field"
      v-model="modelValue"
      class="input pr-10"
      :placeholder="props.placeholder"
      aria-keyshortcuts="/"
    />
    <kbd
      aria-hidden="true"
      class="absolute right-3 top-1/2 -translate-y-1/2 rounded border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] px-1.5 py-0.5 text-[10px] font-mono text-[var(--ir-text-muted)]"
    >/</kbd>
  </div>
</template>
