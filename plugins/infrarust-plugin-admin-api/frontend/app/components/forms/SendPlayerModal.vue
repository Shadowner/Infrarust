<script setup lang="ts">
import { PaperAirplaneIcon } from '@heroicons/vue/24/outline';
import type { ServerDto, ApiEnvelope, MutationResult } from '~/types/api';

const modelValue = defineModel<boolean>({ default: false });
const props = defineProps<{ username: string }>();
const emit = defineEmits<{ done: [] }>();

const { request } = useApi();
const { push } = useToast();

const target = ref('');
const submitting = ref(false);
const servers = ref<ServerDto[]>([]);

// Fetch available servers when modal opens
watch(modelValue, async (open) => {
  if (open) {
    try {
      const res = await request<ApiEnvelope<ServerDto[]>>('/servers');
      servers.value = res.data;
    } catch {
      // Fallback to manual input
    }
  }
});

async function submit() {
  if (!target.value.trim()) {
    push({ type: 'error', title: 'Select a destination' });
    return;
  }
  submitting.value = true;
  try {
    await request<ApiEnvelope<MutationResult>>(`/players/${encodeURIComponent(props.username)}/send`, {
      method: 'POST',
      body: { server: target.value.trim() },
    });
    push({ type: 'success', title: `Sent ${props.username} to ${target.value}` });
    target.value = '';
    modelValue.value = false;
    emit('done');
  } catch {
    push({ type: 'error', title: `Failed to send ${props.username}` });
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
              <PaperAirplaneIcon class="h-4.5 w-4.5 text-[var(--ir-accent)]" />
            </div>
            <div>
              <h2 class="text-lg font-semibold">Send Player</h2>
              <p class="text-xs text-[var(--ir-text-muted)]">Transfer <span class="font-medium text-white">{{ username }}</span> to another server</p>
            </div>
          </div>

          <div class="mt-5">
            <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">
              Destination
            </label>

            <!-- Server cards -->
            <div v-if="servers.length > 0" class="grid gap-2 max-h-60 overflow-auto">
              <button
                v-for="srv in servers"
                :key="srv.id"
                class="flex items-center justify-between rounded-lg border p-3 text-left text-sm transition-colors"
                :class="target === srv.id
                  ? 'border-[var(--ir-border-strong)] bg-[var(--ir-accent-soft)]'
                  : 'border-[var(--ir-border)] bg-[var(--ir-surface-soft)] hover:border-[var(--ir-border-strong)]'
                "
                @click="target = srv.id"
              >
                <div class="flex items-center gap-2">
                  <span
                    class="h-2 w-2 rounded-full"
                    :class="srv.state === 'online' ? 'bg-[var(--ir-success)]' : 'bg-slate-500'"
                  />
                  <span class="font-medium">{{ srv.id }}</span>
                </div>
                <span class="text-[10px] text-[var(--ir-text-muted)]">{{ srv.proxy_mode }}</span>
              </button>
            </div>

            <!-- Fallback manual input -->
            <div class="mt-2">
              <input
                v-model="target"
                class="input font-mono"
                placeholder="Or type a server ID / limbo handler..."
              />
            </div>
          </div>

          <div class="mt-5 flex justify-end gap-2">
            <button class="btn btn-secondary" @click="modelValue = false">Cancel</button>
            <button class="btn btn-primary" :disabled="submitting || !target.trim()" @click="submit">
              {{ submitting ? 'Sending...' : 'Send' }}
            </button>
          </div>
        </div>
      </Transition>
    </div>
  </Transition>
</template>
