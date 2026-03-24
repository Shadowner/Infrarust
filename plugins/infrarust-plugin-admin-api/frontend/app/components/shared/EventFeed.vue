<script setup lang="ts">
defineProps<{
  items: Array<{ type: string; timestamp?: string; summary?: string }>;
}>();

function typeColor(type: string) {
  const t = type.toLowerCase();
  if (t.includes('join')) return 'bg-[#5daf50]';
  if (t.includes('leave')) return 'bg-[#cc3e38]';
  if (t.includes('state')) return 'bg-[#e9a047]';
  return 'bg-[var(--ir-accent)]';
}

function typeBadgeClass(type: string) {
  const t = type.toLowerCase();
  if (t.includes('join')) return 'bg-[rgba(93,175,80,0.14)] text-[#bce5b6]';
  if (t.includes('leave')) return 'bg-[rgba(204,62,56,0.14)] text-[#ffc0bc]';
  if (t.includes('state')) return 'bg-[rgba(233,160,71,0.14)] text-[#ffd8ad]';
  return 'bg-[var(--ir-accent-soft)] text-[var(--ir-accent)]';
}

function relativeTime(ts?: string) {
  if (!ts) return 'just now';
  try {
    const diff = Date.now() - new Date(ts).getTime();
    if (diff < 5000) return 'just now';
    if (diff < 60000) return `${Math.floor(diff / 1000)}s ago`;
    if (diff < 3600000) return `${Math.floor(diff / 60000)}m ago`;
    return `${Math.floor(diff / 3600000)}h ago`;
  } catch {
    return ts;
  }
}
</script>

<template>
  <div class="max-h-80 overflow-auto">
    <div v-if="items.length === 0" class="py-6 text-center text-sm text-[var(--ir-text-muted)]">No events yet.</div>

    <TransitionGroup name="list" tag="div" class="relative">
      <div
        v-for="(item, index) in items"
        :key="item.timestamp ?? index"
        class="relative flex gap-3 py-2.5 pl-5"
      >
        <!-- Timeline line -->
        <div v-if="index < items.length - 1" class="absolute bottom-0 left-[7px] top-5 w-px bg-[var(--ir-border)]" />
        <!-- Timeline dot -->
        <div class="absolute left-0 top-3 h-[14px] w-[14px] rounded-full border-2 border-[var(--ir-bg-deep)] flex items-center justify-center" :class="typeColor(item.type)">
          <div class="h-1.5 w-1.5 rounded-full bg-white/60" />
        </div>

        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <span class="rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-[0.06em]" :class="typeBadgeClass(item.type)">
              {{ item.type }}
            </span>
            <span class="text-[10px] text-[var(--ir-text-muted)]">{{ relativeTime(item.timestamp) }}</span>
          </div>
          <p class="mt-1 text-sm text-[var(--ir-text)]">{{ item.summary ?? 'Event received' }}</p>
        </div>
      </div>
    </TransitionGroup>
  </div>
</template>
