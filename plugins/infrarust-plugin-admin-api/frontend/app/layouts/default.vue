<script setup>
// Initialize the singleton SSE event bus once for all authenticated pages
const { connect } = useEventBus();
onMounted(() => connect());
</script>

<template>
  <div class="min-h-screen">
    <!-- Ambient background -->
    <div class="ambient-bg" />

    <!-- App shell -->
    <AppLayout>
      <NuxtPage />
    </AppLayout>

    <!-- Global overlays -->
    <ToastNotification />
    <ConfirmDialog />
    <RateLimitModal />
  </div>
</template>
