<script setup lang="ts">
const props = defineProps<{
  isEdit: boolean;
  serverId?: string;
  initialData?: { domains: string[]; addresses: string[]; mode: string; handlers: string[] };
}>();

const emit = defineEmits<{
  submit: [payload: { id?: string; domains: string[]; addresses: string[]; mode: string; handlers: string[] }];
  cancel: [];
}>();

const { push } = useToast();

const serverId = ref(props.serverId ?? '');
const domains = ref<string[]>(props.initialData?.domains ?? []);
const addresses = ref<string[]>(props.initialData?.addresses ?? []);
const mode = ref(props.initialData?.mode ?? 'passthrough');
const handlers = ref<string[]>(props.initialData?.handlers ?? []);
const submitting = ref(false);

watch(() => props.initialData, (data) => {
  if (data) {
    domains.value = [...data.domains];
    addresses.value = [...data.addresses];
    mode.value = data.mode;
    handlers.value = [...data.handlers];
  }
});

const isIntercepted = computed(() => ['client_only', 'offline'].includes(mode.value));

const validateAddress = (value: string): string | null => {
  if (/^.+:\d{1,5}$/.test(value)) return null;
  return 'Expected format: host:port (e.g., 127.0.0.1:25565)';
};

function submit() {
  if (!props.isEdit && !serverId.value.trim()) {
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

  emit('submit', {
    id: props.isEdit ? undefined : serverId.value.trim(),
    domains: domains.value,
    addresses: addresses.value,
    mode: mode.value,
    handlers: handlers.value,
  });
}

defineExpose({ submitting });
</script>

<template>
  <div class="glass-pane relative overflow-hidden p-6">
    <div class="absolute inset-x-0 top-0 h-[3px] bg-[var(--ir-accent-gradient)]" />

    <div class="grid gap-5">
      <!-- Server ID (create only) -->
      <div v-if="!isEdit">
        <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Server ID</label>
        <input v-model="serverId" class="input font-mono" placeholder="my-server" />
      </div>
      <div v-else>
        <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Server ID</label>
        <input :value="serverId" class="input font-mono" disabled />
      </div>

      <!-- Domains -->
      <div>
        <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Domains</label>
        <TagInput v-model="domains" placeholder="mc.example.com" />
      </div>

      <!-- Addresses -->
      <div>
        <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Addresses (host:port)</label>
        <TagInput v-model="addresses" placeholder="127.0.0.1:25565" :validate="validateAddress" />
      </div>

      <!-- Proxy Mode -->
      <div>
        <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Proxy Mode</label>
        <ProxyModeSelector v-model="mode" />
      </div>

      <!-- Limbo Handlers (intercepted modes only) -->
      <div v-if="isIntercepted">
        <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Limbo Handlers</label>
        <TagInput v-model="handlers" placeholder="handler-name" />
      </div>
    </div>

    <div class="mt-6 flex justify-end gap-2">
      <button class="btn btn-secondary" @click="emit('cancel')">Cancel</button>
      <button class="btn btn-primary" :disabled="submitting" @click="submit">
        {{ submitting ? 'Saving...' : isEdit ? 'Update Server' : 'Create Server' }}
      </button>
    </div>
  </div>
</template>
