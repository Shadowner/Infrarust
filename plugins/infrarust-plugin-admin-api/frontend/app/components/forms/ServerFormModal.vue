<script setup lang="ts">
import type { CreateServerRequest, UpdateServerRequest, ApiEnvelope, MutationResult } from '~/types/api';

const modelValue = defineModel<boolean>({ default: false });

const props = defineProps<{
  editId?: string;
  initialData?: { domains: string[]; addresses: string[]; mode: string; handlers: string[] };
}>();

const emit = defineEmits<{ saved: [] }>();

const { request } = useApi();
const { push } = useToast();

const serverId = ref(props.editId ?? '');
const domains = ref<string[]>(props.initialData?.domains ?? []);
const addresses = ref<string[]>(props.initialData?.addresses ?? []);
const mode = ref(props.initialData?.mode ?? 'passthrough');
const handlers = ref<string[]>(props.initialData?.handlers ?? []);
const submitting = ref(false);

const isEdit = computed(() => !!props.editId);

watch(() => props.initialData, (data) => {
  if (data) {
    domains.value = [...data.domains];
    addresses.value = [...data.addresses];
    mode.value = data.mode;
    handlers.value = [...data.handlers];
  }
});

async function submit() {
  if (!isEdit.value && !serverId.value.trim()) {
    push({ type: 'error', title: 'Server ID is required' });
    return;
  }
  if (domains.value.length === 0) {
    push({ type: 'error', title: 'At least one domain is required' });
    return;
  }
  if (addresses.value.length === 0) {
    push({ type: 'error', title: 'At least one address is required' });
    return;
  }

  submitting.value = true;
  try {
    if (isEdit.value) {
      const body: UpdateServerRequest = {
        domains: domains.value,
        addresses: addresses.value,
        proxy_mode: mode.value,
        limbo_handlers: handlers.value.length > 0 ? handlers.value : undefined,
      };
      await request<ApiEnvelope<MutationResult>>(`/servers/${encodeURIComponent(props.editId!)}`, {
        method: 'PUT',
        body: body as unknown as Record<string, unknown>,
      });
      push({ type: 'success', title: `Server '${props.editId}' updated` });
    } else {
      const body: CreateServerRequest = {
        id: serverId.value.trim(),
        domains: domains.value,
        addresses: addresses.value,
        proxy_mode: mode.value,
        limbo_handlers: handlers.value.length > 0 ? handlers.value : undefined,
      };
      await request<ApiEnvelope<MutationResult>>('/servers', {
        method: 'POST',
        body: body as unknown as Record<string, unknown>,
      });
      push({ type: 'success', title: `Server '${serverId.value}' created` });
    }
    emit('saved');
    modelValue.value = false;
  } catch (e: unknown) {
    const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? 'Operation failed';
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
        <div class="glass-pane relative w-full max-w-2xl overflow-hidden p-6">
          <div class="absolute inset-x-0 top-0 h-[3px] bg-[var(--ir-accent-gradient)]" />

          <p class="accent-chip">Server Management</p>
          <h2 class="mt-3 text-lg font-semibold">{{ isEdit ? 'Edit Server' : 'Create Server' }}</h2>

          <div class="mt-5 grid gap-4">
            <div v-if="!isEdit">
              <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Server ID</label>
              <input v-model="serverId" class="input font-mono" placeholder="my-server" />
            </div>
            <div>
              <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Domains</label>
              <TagInput v-model="domains" />
            </div>
            <div>
              <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Addresses (host:port)</label>
              <TagInput v-model="addresses" />
            </div>
            <div>
              <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Proxy Mode</label>
              <select v-model="mode" class="input">
                <option>passthrough</option>
                <option value="zero_copy">zerocopy</option>
                <option>client_only</option>
                <option>offline</option>
                <option>server_only</option>
              </select>
            </div>
            <div>
              <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Limbo Handlers</label>
              <TagInput v-model="handlers" />
            </div>
          </div>

          <div class="mt-5 flex justify-end gap-2">
            <button class="btn btn-secondary" @click="modelValue = false">Cancel</button>
            <button class="btn btn-primary" :disabled="submitting" @click="submit">
              {{ submitting ? 'Saving...' : isEdit ? 'Update Server' : 'Create Server' }}
            </button>
          </div>
        </div>
      </Transition>
    </div>
  </Transition>
</template>
