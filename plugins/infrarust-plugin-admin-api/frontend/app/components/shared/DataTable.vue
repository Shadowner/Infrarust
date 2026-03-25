<script setup lang="ts" generic="T extends Record<string, unknown>">
defineProps<{
  columns: Array<{ key: string; label: string }>;
  rows: T[];
}>();
</script>

<template>
  <!-- Desktop table -->
  <div class="glass-pane hidden overflow-auto lg:block">
    <table class="min-w-full text-left text-sm">
      <thead>
        <tr class="border-b border-[var(--ir-border)]" style="background: var(--ir-glass-bg-strong); backdrop-filter: blur(12px);">
          <th
            v-for="column in columns"
            :key="column.key"
            class="sticky top-0 px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.09em] text-[var(--ir-text-muted)]"
            style="background: var(--ir-glass-bg-strong);"
          >
            {{ column.label }}
          </th>
          <th
            class="sticky top-0 px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.09em] text-[var(--ir-text-muted)]"
            style="background: var(--ir-glass-bg-strong);"
          >
            Actions
          </th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="(row, index) in rows"
          :key="index"
          class="group border-t border-[var(--ir-border)] text-[var(--ir-text)] odd:bg-white/[0.02] transition-colors duration-100 hover:bg-[rgba(232,131,42,0.04)]"
        >
          <td v-for="column in columns" :key="column.key" class="relative px-4 py-2.5">
            <slot :name="`cell-${column.key}`" :row="row" :value="row[column.key]">
              {{ row[column.key] }}
            </slot>
          </td>
          <td class="px-4 py-2.5">
            <slot name="actions" :row="row" />
          </td>
        </tr>
        <tr v-if="rows.length === 0">
          <td :colspan="columns.length + 1" class="px-4 py-10 text-center text-sm text-[var(--ir-text-muted)]">
            No data available.
          </td>
        </tr>
      </tbody>
    </table>
  </div>

  <!-- Mobile card list -->
  <div class="flex flex-col gap-3 lg:hidden">
    <div v-if="rows.length === 0" class="glass-pane p-6 text-center text-sm text-[var(--ir-text-muted)]">
      No data available.
    </div>
    <div
      v-for="(row, index) in rows"
      :key="index"
      class="glass-pane p-4"
    >
      <div class="space-y-2">
        <div v-for="column in columns" :key="column.key" class="flex items-start justify-between gap-3">
          <span class="text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--ir-text-muted)]">{{ column.label }}</span>
          <span class="text-right text-sm text-[var(--ir-text)]">
            <slot :name="`cell-${column.key}`" :row="row" :value="row[column.key]">
              {{ row[column.key] }}
            </slot>
          </span>
        </div>
      </div>
      <div class="mt-3 flex items-center gap-2 border-t border-[var(--ir-border)] pt-3">
        <slot name="actions" :row="row" />
      </div>
    </div>
  </div>
</template>
