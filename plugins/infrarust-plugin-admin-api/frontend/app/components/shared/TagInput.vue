<script setup lang="ts">
import { XMarkIcon, PlusIcon, PencilIcon } from '@heroicons/vue/24/outline';

const modelValue = defineModel<string[]>({ default: [] });

const props = withDefaults(defineProps<{
  placeholder?: string;
  validate?: (value: string) => string | null;
}>(), {
  placeholder: 'Add value...',
  validate: undefined,
});

const draft = ref('');
const warning = ref<string | null>(null);
const editingIndex = ref<number | null>(null);
const editDraft = ref('');

const addTag = () => {
  const value = draft.value.trim();
  if (!value) return;

  if (props.validate) {
    const msg = props.validate(value);
    if (msg) {
      warning.value = msg;
    } else {
      warning.value = null;
    }
  }

  if (!modelValue.value.includes(value)) {
    modelValue.value = [...modelValue.value, value];
  }
  draft.value = '';
};

const removeTag = (index: number) => {
  modelValue.value = modelValue.value.filter((_, i) => i !== index);
  if (editingIndex.value === index) {
    editingIndex.value = null;
  }
};

const startEdit = (index: number) => {
  editingIndex.value = index;
  editDraft.value = modelValue.value[index] ?? '';
};

const confirmEdit = () => {
  if (editingIndex.value === null) return;
  const value = editDraft.value.trim();
  if (!value) {
    cancelEdit();
    return;
  }

  if (props.validate) {
    const msg = props.validate(value);
    warning.value = msg;
  }

  const idx = editingIndex.value;
  const isDuplicate = modelValue.value.some((tag, i) => i !== idx && tag === value);
  if (!isDuplicate) {
    const updated = [...modelValue.value];
    updated[idx] = value;
    modelValue.value = updated;
  }
  editingIndex.value = null;
};

const cancelEdit = () => {
  editingIndex.value = null;
  editDraft.value = '';
};

watch(draft, () => {
  if (warning.value) warning.value = null;
});
</script>

<template>
  <div class="rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-3">
    <div v-if="modelValue.length" class="mb-2.5 flex flex-wrap gap-1.5">
      <template v-for="(tag, index) in modelValue" :key="index">
        <!-- Editing state -->
        <div v-if="editingIndex === index" class="inline-flex items-center gap-1 rounded-md border border-[var(--ir-accent)] bg-[var(--ir-accent-soft)] px-1 py-0.5">
          <input
            v-model="editDraft"
            class="bg-transparent font-mono text-xs text-[var(--ir-accent)] outline-none w-32"
            @keydown.enter.prevent="confirmEdit"
            @keydown.escape="cancelEdit"
            @blur="confirmEdit"
            @vue:mounted="($event: any) => $event.el.focus()"
          />
        </div>

        <!-- Display state -->
        <span
          v-else
          class="group inline-flex items-center gap-1 rounded-md border border-[var(--ir-border-strong)] bg-[var(--ir-accent-soft)] px-2 py-0.5 text-xs font-medium text-[var(--ir-accent)] cursor-pointer transition-colors hover:border-[var(--ir-accent)]"
          @click="startEdit(index)"
        >
          <span class="font-mono">{{ tag }}</span>
          <PencilIcon class="h-2.5 w-2.5 opacity-0 group-hover:opacity-60 transition-opacity" />
          <button class="rounded-sm p-0.5 transition-colors hover:bg-white/10" @click.stop="removeTag(index)">
            <XMarkIcon class="h-3 w-3" />
          </button>
        </span>
      </template>
    </div>
    <div class="flex gap-2">
      <input v-model="draft" class="input flex-1" :placeholder="placeholder" @keydown.enter.prevent="addTag" />
      <button class="btn btn-ghost flex items-center gap-1 text-xs" @click="addTag">
        <PlusIcon class="h-3.5 w-3.5" />
        Add
      </button>
    </div>
    <p v-if="warning" class="mt-1.5 text-[10px] text-[var(--ir-danger)]">{{ warning }}</p>
  </div>
</template>
