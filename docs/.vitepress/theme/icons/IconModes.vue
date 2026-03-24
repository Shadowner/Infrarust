<script setup>
defineProps({
  size: { type: Number, default: 20 },
})
</script>

<template>
  <svg :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor"
       stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <polyline points="7 3 7 9" />
    <polyline points="17 3 17 9" />
    <polyline points="7 15 7 21" />
    <polyline points="17 15 17 21" />
    <line x1="4" y1="6" x2="10" y2="6" />
    <line x1="14" y1="6" x2="20" y2="6" />
    <line x1="4" y1="18" x2="10" y2="18" />
    <line x1="14" y1="18" x2="20" y2="18" />
  </svg>
</template>
