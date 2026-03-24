<script setup lang="ts">
import { XMarkIcon } from '@heroicons/vue/24/outline';
import type { ApiEnvelope, MutationResult } from '~/types/api';

const modelValue = defineModel<boolean>({ default: false });
const props = defineProps<{ username: string }>();
const emit = defineEmits<{ done: [] }>();

const { request } = useApi();
const { push } = useToast();

const reason = ref('');
const submitting = ref(false);

async function submit() {
  submitting.value = true;
  try {
    await request<ApiEnvelope<MutationResult>>(`/players/${encodeURIComponent(props.username)}/kick`, {
      method: 'POST',
      body: { reason: reason.value.trim() || null },
    });
    push({ type: 'success', title: `Kicked ${props.username}` });
    reason.value = '';
    modelValue.value = false;
    emit('done');
  } catch {
    push({ type: 'error', title: `Failed to kick ${props.username}` });
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <Transition name="modal-overlay">
    <div
      v-if="modelValue"
      class="fixed inset-0 z-[var(--ir-z-modal)] flex items-center justify-center bg-black/60 p-4 backdrop-blur-md"
      @click.self="modelValue = false"
    >
      <Transition name="modal" appear>
        <div class="glass-pane relative w-full max-w-md overflow-hidden p-6">
          <div class="absolute inset-x-0 top-0 h-[3px] bg-gradient-to-r from-[var(--ir-danger)] to-[var(--ir-warn)]" />

          <div class="flex items-center gap-3">
            <div class="flex h-9 w-9 items-center justify-center rounded-lg bg-[rgba(204,62,56,0.14)]">
              <XMarkIcon class="h-4.5 w-4.5 text-[var(--ir-danger)]" />
            </div>
            <div>
              <h2 class="text-lg font-semibold">Kick Player</h2>
              <p class="text-xs text-[var(--ir-text-muted)]">Disconnect <span class="font-medium text-white">{{ username }}</span> from the proxy</p>
            </div>
          </div>

          <div class="mt-5">
            <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">
              Reason (optional)
            </label>
            <input
              v-model="reason"
              class="input"
              placeholder="e.g. Disruptive behavior"
              @keydown.enter.prevent="submit"
            />
          </div>

          <div class="mt-5 flex justify-end gap-2">
            <button class="btn btn-secondary" @click="modelValue = false">Cancel</button>
            <button class="btn btn-danger" :disabled="submitting" @click="submit">
              {{ submitting ? 'Kicking...' : 'Kick' }}
            </button>
          </div>
        </div>
      </Transition>
    </div>
  </Transition>
</template>
