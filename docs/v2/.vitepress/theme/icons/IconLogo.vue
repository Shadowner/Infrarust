<script setup>
defineProps({
  size: { type: Number, default: 36 },
})
</script>

<template>
  <svg viewBox="0 0 1000 1000" :width="size" :height="size" aria-hidden="true">
    <path d="M 85 310 v 400 q 0 37 25 50 l 350 190 q 25 10 25 -20 v -400 q 0 -37 -25 -50 l -350 -190 q -25 -10 -25 20 Z" fill="#C4621A"/>
    <path d="M 910 310 v 400 q 0 37 -25 50 l -350 190 q -25 10 -25 -20 v -400 q 0 -37 25 -50 l 350 -190 q 25 -10 25 20 Z" fill="#A84D15"/>
    <path d="M 120 270 l 345 180 q 30 15 60 0 l 345 -180 q 15 -15 0 -30 l -345 -180 q -30 -15 -60 0 l -345 180 q -15 15 0 30 Z" fill="#E8832A"/>
  </svg>
</template>
