<script setup lang="ts">
import type { CreateBanRequest, ApiEnvelope, MutationResult } from '~/types/api';

const modelValue = defineModel<boolean>({ default: false });
const emit = defineEmits<{ created: [] }>();

const { request } = useApi();
const { push } = useToast();

const targetType = ref<'ip' | 'username' | 'uuid'>('username');
const value = ref('');
const reason = ref('');
const duration = ref('permanent');
const submitting = ref(false);

const durationMap: Record<string, number | null> = {
  '1h': 3600,
  '6h': 21600,
  '1d': 86400,
  '7d': 604800,
  '30d': 2592000,
  'permanent': null,
};

async function submit() {
  if (!value.value.trim()) {
    push({ type: 'error', title: 'Target value is required' });
    return;
  }

  submitting.value = true;
  try {
    const body: CreateBanRequest = {
      target: { type: targetType.value, value: value.value.trim() },
      reason: reason.value.trim() || null,
      duration_seconds: durationMap[duration.value] ?? null,
    };
    await request<ApiEnvelope<MutationResult>>('/bans', { method: 'POST', body: body as unknown as Record<string, unknown> });
    push({ type: 'success', title: `Banned ${value.value}` });
    value.value = '';
    reason.value = '';
    duration.value = 'permanent';
    emit('created');
  } catch (e: unknown) {
    const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? 'Failed to create ban';
    push({ type: 'error', title: msg });
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
        <div class="glass-pane relative w-full max-w-lg overflow-hidden p-6">
          <div class="absolute inset-x-0 top-0 h-[3px] bg-[var(--ir-accent-gradient)]" />

          <p class="accent-chip">Moderation</p>
          <h2 class="mt-3 text-lg font-semibold">Create Ban</h2>

          <div class="mt-5 grid gap-4">
            <div>
              <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Target Type</label>
              <select v-model="targetType" class="input">
                <option value="ip">IP Address</option>
                <option value="username">Username</option>
                <option value="uuid">UUID</option>
              </select>
            </div>
            <div>
              <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Value</label>
              <input v-model="value" class="input" placeholder="Target value" />
            </div>
            <div>
              <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Reason</label>
              <input v-model="reason" class="input" placeholder="Optional reason" />
            </div>
            <div>
              <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Duration</label>
              <select v-model="duration" class="input">
                <option value="1h">1 Hour</option>
                <option value="6h">6 Hours</option>
                <option value="1d">1 Day</option>
                <option value="7d">7 Days</option>
                <option value="30d">30 Days</option>
                <option value="permanent">Permanent</option>
              </select>
            </div>
          </div>

          <div class="mt-5 flex justify-end gap-2">
            <button class="btn btn-secondary" @click="modelValue = false">Cancel</button>
            <button class="btn btn-primary" :disabled="submitting" @click="submit">
              {{ submitting ? 'Creating...' : 'Create Ban' }}
            </button>
          </div>
        </div>
      </Transition>
    </div>
  </Transition>
</template>
