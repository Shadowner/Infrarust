<script setup>
defineProps({
  size: { type: Number, default: 20 },
})
</script>

<template>
  <svg :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor"
       stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <circle cx="12" cy="5" r="3" />
    <circle cx="4" cy="19" r="3" />
    <circle cx="20" cy="19" r="3" />
    <line x1="12" y1="8" x2="4" y2="16" />
    <line x1="12" y1="8" x2="20" y2="16" />
  </svg>
</template>
