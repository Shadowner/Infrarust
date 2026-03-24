<script setup lang="ts">
import { XMarkIcon, PlusIcon } from '@heroicons/vue/24/outline';

const modelValue = defineModel<string[]>({ default: [] });
const draft = ref('');

const addTag = () => {
  const value = draft.value.trim();
  if (!value) return;
  if (!modelValue.value.includes(value)) {
    modelValue.value = [...modelValue.value, value];
  }
  draft.value = '';
};

const removeTag = (value: string) => {
  modelValue.value = modelValue.value.filter((tag) => tag !== value);
};
</script>

<template>
  <div class="rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-3">
    <div v-if="modelValue.length" class="mb-2.5 flex flex-wrap gap-1.5">
      <span
        v-for="tag in modelValue"
        :key="tag"
        class="inline-flex items-center gap-1 rounded-md border border-[var(--ir-border-strong)] bg-[var(--ir-accent-soft)] px-2 py-0.5 text-xs font-medium text-[var(--ir-accent)]"
      >
        <span class="font-mono">{{ tag }}</span>
        <button class="rounded-sm p-0.5 transition-colors hover:bg-white/10" @click="removeTag(tag)">
          <XMarkIcon class="h-3 w-3" />
        </button>
      </span>
    </div>
    <div class="flex gap-2">
      <input v-model="draft" class="input flex-1" placeholder="Add value..." @keydown.enter.prevent="addTag" />
      <button class="btn btn-ghost flex items-center gap-1 text-xs" @click="addTag">
        <PlusIcon class="h-3.5 w-3.5" />
        Add
      </button>
    </div>
  </div>
</template>
