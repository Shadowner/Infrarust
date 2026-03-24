<script setup lang="ts">
const props = defineProps<{
  label: string;
  value: string | number;
  hint?: string;
}>();

const numericValue = computed(() => typeof props.value === 'number' ? props.value : null);
const sourceRef = computed(() => numericValue.value ?? 0);
const animatedValue = numericValue.value !== null ? useAnimatedNumber(sourceRef) : null;
const displayValue = computed(() => animatedValue ? animatedValue.value : props.value);
</script>

<template>
  <article class="glass-pane glass-pane-interactive relative overflow-hidden p-5">
    <p class="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">{{ label }}</p>
    <p class="mt-2.5 text-3xl font-bold tracking-tight text-gradient-accent">{{ displayValue }}</p>
    <p v-if="hint" class="mt-1.5 text-xs text-[var(--ir-text-muted)]">{{ hint }}</p>
  </article>
</template>
