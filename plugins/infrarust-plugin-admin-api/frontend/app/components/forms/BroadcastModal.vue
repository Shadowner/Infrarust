<script setup lang="ts">
import { MegaphoneIcon } from '@heroicons/vue/24/outline';
import type { ApiEnvelope, MutationResult } from '~/types/api';

const modelValue = defineModel<boolean>({ default: false });
const { request } = useApi();
const { push } = useToast();

const message = ref('');
const submitting = ref(false);

async function submit() {
  const text = message.value.trim();
  if (!text) {
    push({ type: 'error', title: 'Message cannot be empty' });
    return;
  }

  submitting.value = true;
  try {
    await request<ApiEnvelope<MutationResult>>('/players/broadcast', {
      method: 'POST',
      body: { text },
    });
    push({ type: 'success', title: 'Broadcast sent', message: text.length > 60 ? text.slice(0, 60) + '…' : text });
    message.value = '';
    modelValue.value = false;
  } catch {
    push({ type: 'error', title: 'Broadcast failed' });
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
          <div class="absolute inset-x-0 top-0 h-[3px] bg-[var(--ir-accent-gradient)]" />

          <div class="flex items-center gap-3">
            <div class="flex h-9 w-9 items-center justify-center rounded-lg bg-[var(--ir-accent-soft)]">
              <MegaphoneIcon class="h-4.5 w-4.5 text-[var(--ir-accent)]" />
            </div>
            <div>
              <h2 class="text-lg font-semibold">Broadcast Message</h2>
              <p class="text-xs text-[var(--ir-text-muted)]">Send a message to all connected players</p>
            </div>
          </div>

          <div class="mt-5">
            <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">
              Message
            </label>
            <textarea
              v-model="message"
              class="input min-h-[100px] resize-y"
              placeholder="Type your message here..."
              @keydown.meta.enter.prevent="submit"
              @keydown.ctrl.enter.prevent="submit"
            />
            <p class="mt-1.5 text-[10px] text-[var(--ir-text-muted)]">
              Press Ctrl+Enter to send
            </p>
          </div>

          <div class="mt-5 flex justify-end gap-2">
            <button class="btn btn-secondary" @click="modelValue = false">Cancel</button>
            <button class="btn btn-primary flex items-center gap-2" :disabled="submitting || !message.trim()" @click="submit">
              <MegaphoneIcon class="h-4 w-4" />
              {{ submitting ? 'Sending...' : 'Broadcast' }}
            </button>
          </div>
        </div>
      </Transition>
    </div>
  </Transition>
</template>
