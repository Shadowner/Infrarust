<script setup lang="ts">
const route = useRoute();

const title = computed(() => {
  if (route.path.startsWith('/players')) return 'Players';
  if (route.path.startsWith('/bans')) return 'Bans';
  if (route.path.startsWith('/servers')) return 'Servers';
  if (route.path.startsWith('/plugins')) return 'Plugins';
  if (route.path.startsWith('/logs')) return 'Logs';
  if (route.path.startsWith('/config')) return 'Config';
  return 'Dashboard';
});

const auth = useAuth();

function switchKey() {
    auth.clear();
    navigateTo('/login');
}
</script>

<template>
  <header class="panel flex flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-between md:px-5 md:py-4">
    <div>
      <p class="text-[11px] font-semibold uppercase tracking-[0.12em] text-[var(--accent)]">Infrarust Control</p>
      <h1 class="mt-1 text-xl font-bold tracking-tight md:text-2xl">{{ title }}</h1>
      <p class="mt-1 text-xs text-[var(--text-dim)]">Administration surface for proxy operations</p>
    </div>

    <div class="flex items-center gap-2 self-start sm:self-auto">
      <NuxtLink to="/dashboard" class="btn btn-secondary">Overview</NuxtLink>
      <NuxtLink @click="switchKey" class="btn btn-secondary">Switch key</NuxtLink>
    </div>
  </header>
</template>
