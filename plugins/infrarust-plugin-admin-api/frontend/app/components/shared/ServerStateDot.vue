<script setup lang="ts">
const props = defineProps<{ status: string }>();

const shape = computed(() => {
  const status = props.status.toLowerCase();
  if (status === 'online') return 'dot--online';
  if (status === 'starting' || status === 'stopping') return 'dot--moving';
  if (status === 'crashed' || status === 'unreachable') return 'dot--down';
  if (status === 'sleeping' || status === 'offline') return 'dot--idle';
  return 'dot--unknown';
});
</script>

<template>
  <span
    class="server-dot"
    :class="[shape, shape === 'dot--online' ? 'status-pulse' : '']"
    role="img"
    :aria-label="`State: ${status}`"
    :title="status"
  />
</template>

<style scoped>
/* The shape carries the state as well as the colour: circle, diamond, square, ring. */
.server-dot {
  display: inline-block;
  height: 0.625rem;
  width: 0.625rem;
  flex-shrink: 0;
  border: 2px solid transparent;
  border-radius: 9999px;
}

.dot--online {
  background: var(--ir-success);
}

.dot--moving {
  background: var(--ir-warn);
  border-radius: 1px;
  transform: rotate(45deg) scale(0.85);
}

.dot--down {
  background: var(--ir-danger);
  border-radius: 2px;
}

.dot--idle {
  border-color: rgb(148 163 184);
}

.dot--unknown {
  background: rgb(100 116 139);
}
</style>
