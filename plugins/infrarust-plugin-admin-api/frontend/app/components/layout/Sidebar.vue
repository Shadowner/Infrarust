<script setup lang="ts">
const route = useRoute();
const { status } = useSSE('/events', { types: ['stats.tick'] });

const navItems = [
  { to: '/dashboard', label: 'Dashboard', count: 0 },
  { to: '/players', label: 'Players', count: 0 },
  { to: '/bans', label: 'Bans', count: 0 },
  { to: '/servers', label: 'Servers', count: 0 },
  { to: '/plugins', label: 'Plugins', count: 0 },
  { to: '/logs', label: 'Logs' },
  { to: '/config', label: 'Config' },
];
</script>

<template>
  <aside class="panel hidden w-72 shrink-0 flex-col p-4 lg:flex">
    <div class="panel-muted mb-4 p-3">
      <div class="text-sm font-semibold uppercase tracking-[0.1em] text-[var(--accent)]">Infrarust v2</div>
      <div class="mt-2 text-xs text-[var(--text-dim)]">Uptime streaming...</div>
      <div class="mt-2 flex items-center gap-2 text-xs">
        <span
          class="h-2.5 w-2.5 rounded-full"
          :class="status === 'open' ? 'bg-[var(--ir-success)]' : 'bg-[var(--danger)]'"
        />
        <span class="text-[var(--text-dim)]">{{ status === 'open' ? 'Live' : 'Disconnected' }}</span>
      </div>
    </div>

    <nav class="flex flex-1 flex-col gap-2">
      <NuxtLink
        v-for="item in navItems"
        :key="item.to"
        :to="item.to"
        class="flex items-center justify-between rounded-lg border px-3 py-2 text-sm transition"
        :class="
          route.path.startsWith(item.to)
            ? 'border-[var(--ir-border-strong)] bg-[var(--ir-accent-soft)] text-white'
            : 'border-[var(--border)] text-[var(--text-dim)] hover:border-[var(--ir-border-strong)] hover:text-white'
        "
      >
        <span>{{ item.label }}</span>
        <span
          v-if="item.count !== undefined"
          class="rounded border border-[var(--border)] bg-black/20 px-2 py-0.5 text-xs"
        >
          {{ item.count }}
        </span>
      </NuxtLink>
    </nav>

    <button class="btn btn-danger mt-4 w-full">Shutdown</button>
  </aside>
</template>
