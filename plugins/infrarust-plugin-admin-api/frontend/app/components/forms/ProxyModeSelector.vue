<script setup lang="ts">
import {
  ArrowsRightLeftIcon,
  BoltIcon,
  ServerIcon,
  ShieldCheckIcon,
  LockOpenIcon,
  InformationCircleIcon,
} from '@heroicons/vue/24/outline';
import type { Component } from 'vue';

const modelValue = defineModel<string>({ default: 'passthrough' });

interface ModeOption {
  value: string;
  label: string;
  description: string;
  icon: Component;
}

const passthroughModes: ModeOption[] = [
  {
    value: 'passthrough',
    label: 'Passthrough',
    description: 'Raw TCP relay. The default mode.',
    icon: ArrowsRightLeftIcon,
  },
  {
    value: 'zero_copy',
    label: 'Zero Copy',
    description: 'Kernel-level relay via splice(2) on Linux. Falls back to passthrough elsewhere.',
    icon: BoltIcon,
  }
];

const interceptedModes: ModeOption[] = [
  {
    value: 'client_only',
    label: 'Client Only',
    description: 'Proxy handles Mojang auth. Backend runs in online_mode=false.',
    icon: ShieldCheckIcon,
  },
  {
    value: 'offline',
    label: 'Offline',
    description: 'No authentication. For cracked/offline servers.',
    icon: LockOpenIcon,
  },
];
</script>

<template>
  <div class="flex flex-col gap-4 lg:flex-row lg:items-start">
    <!-- Passthrough section -->
    <div class="flex-1 rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-4">
      <h4 class="mb-2.5 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Passthrough</h4>
      <div class="mb-3 flex items-start gap-2 rounded-lg border border-[var(--ir-border-strong)] bg-[var(--ir-accent-soft)] px-3 py-2">
        <InformationCircleIcon class="mt-0.5 h-4 w-4 flex-shrink-0 text-[var(--ir-accent)]" />
        <p class="text-[11px] leading-relaxed text-[var(--ir-text-muted)]">
          Enough for most setups, work with 'online_mode=true' servers and has lower resource usage.
          <br />
          Works with every Minecraft version, modded or not.
        </p>
      </div>
      <div class="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-2">
        <button
          v-for="mode in passthroughModes"
          :key="mode.value"
          type="button"
          class="rounded-[var(--ir-radius-sm)] border p-3 text-left transition-all"
          :class="modelValue === mode.value
            ? 'border-[var(--ir-accent)] bg-[var(--ir-accent-soft)] shadow-[0_0_12px_rgba(var(--ir-accent-rgb),0.15)]'
            : 'border-[var(--ir-border)] bg-[rgba(8,8,8,0.4)] hover:border-[var(--ir-border-strong)]'"
          @click="modelValue = mode.value"
        >
          <div class="mb-1.5 flex items-center gap-2">
            <component
              :is="mode.icon"
              class="h-5 w-5 transition-colors"
              :class="modelValue === mode.value ? 'text-[var(--ir-accent)]' : 'text-[var(--ir-text-muted)]'"
            />
            <span class="text-sm font-semibold" :class="modelValue === mode.value ? 'text-white' : 'text-[var(--ir-text)]'">
              {{ mode.label }}
            </span>
          </div>
          <p class="text-[11px] leading-relaxed text-[var(--ir-text-muted)]">{{ mode.description }}</p>
        </button>
      </div>
    </div>

    <!-- Intercepted section -->
    <div class="flex-1 rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-4">
      <h4 class="mb-2.5 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Intercepted</h4>
      <div class="mb-3 flex items-start gap-2 rounded-lg border border-[var(--ir-border-strong)] bg-[var(--ir-surface-soft)] px-3 py-2">
        <InformationCircleIcon class="mt-0.5 h-4 w-4 flex-shrink-0 text-[var(--ir-text-muted)]" />
        <p class="text-[11px] leading-relaxed text-[var(--ir-text-muted)]">
          Packet inspection, plugins and server switching. Requires 'online_mode=false' servers and may have higher resource usage.
          <br />
          Works with every official minecraft release, from 1.7.x to 1.21.x, but may have issues with some modded clients/servers.
        </p>
      </div>
      <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <button
          v-for="mode in interceptedModes"
          :key="mode.value"
          type="button"
          class="rounded-[var(--ir-radius-sm)] border p-3 text-left transition-all"
          :class="modelValue === mode.value
            ? 'border-[var(--ir-accent)] bg-[var(--ir-accent-soft)] shadow-[0_0_12px_rgba(var(--ir-accent-rgb),0.15)]'
            : 'border-[var(--ir-border)] bg-[rgba(8,8,8,0.4)] hover:border-[var(--ir-border-strong)]'"
          @click="modelValue = mode.value"
        >
          <div class="mb-1.5 flex items-center gap-2">
            <component
              :is="mode.icon"
              class="h-5 w-5 transition-colors"
              :class="modelValue === mode.value ? 'text-[var(--ir-accent)]' : 'text-[var(--ir-text-muted)]'"
            />
            <span class="text-sm font-semibold" :class="modelValue === mode.value ? 'text-white' : 'text-[var(--ir-text)]'">
              {{ mode.label }}
            </span>
          </div>
          <p class="text-[11px] leading-relaxed text-[var(--ir-text-muted)]">{{ mode.description }}</p>
        </button>
      </div>
    </div>
  </div>
</template>
