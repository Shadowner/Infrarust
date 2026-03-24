<script setup lang="ts">
import { ArrowLeftIcon } from '@heroicons/vue/24/outline';
import type { ServerDetailDto, ApiEnvelope, MutationResult, UpdateServerRequest } from '~/types/api';

const route = useRoute();
const { request } = useApi();
const { push } = useToast();
const serverId = route.params.id as string;

const server = ref<ServerDetailDto | null>(null);
const domains = ref<string[]>([]);
const addresses = ref<string[]>([]);
const mode = ref('passthrough');
const handlers = ref<string[]>([]);
const submitting = ref(false);

const { data } = await useAsyncData(`server-edit:${serverId}`, async () => {
  const res = await request<ApiEnvelope<ServerDetailDto>>(`/servers/${encodeURIComponent(serverId)}`);
  return res.data;
});

if (data.value) {
  server.value = data.value;
  domains.value = [...data.value.domains];
  addresses.value = [...data.value.addresses];
  mode.value = data.value.proxy_mode;
  handlers.value = [...(data.value.limbo_handlers ?? [])];
}

async function submit() {
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
    const body: UpdateServerRequest = {
      domains: domains.value,
      addresses: addresses.value,
      proxy_mode: mode.value,
      limbo_handlers: handlers.value.length > 0 ? handlers.value : undefined,
    };
    await request<ApiEnvelope<MutationResult>>(`/servers/${encodeURIComponent(serverId)}`, {
      method: 'PUT',
      body: body as unknown as Record<string, unknown>,
    });
    push({ type: 'success', title: `Server '${serverId}' updated` });
    navigateTo(`/servers/${serverId}`);
  } catch (e: unknown) {
    const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? 'Failed to update server';
    push({ type: 'error', title: msg });
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div class="grid gap-5">
    <NuxtLink :to="`/servers/${serverId}`" class="inline-flex items-center gap-1.5 text-sm text-[var(--ir-text-muted)] hover:text-white transition-colors">
      <ArrowLeftIcon class="h-4 w-4" />
      Back to {{ serverId }}
    </NuxtLink>

    <div>
      <div class="flex items-center gap-2">
        <h2 class="text-xl font-bold tracking-tight">Edit Server</h2>
        <span class="rounded bg-[var(--ir-accent-soft)] px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-[var(--ir-accent)]">API</span>
      </div>
      <p class="mt-1 text-sm text-[var(--ir-text-muted)]">Update the configuration for {{ serverId }}.</p>
    </div>

    <div v-if="server && !server.is_api_managed" class="glass-pane flex items-start gap-3 p-5">
      <p class="text-sm text-[var(--ir-text-muted)]">This server is managed by a config provider and cannot be edited from here.</p>
    </div>

    <div v-else-if="server" class="glass-pane relative overflow-hidden p-6">
      <div class="absolute inset-x-0 top-0 h-[3px] bg-[var(--ir-accent-gradient)]" />

      <div class="grid gap-4">
        <div>
          <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Server ID</label>
          <input :value="serverId" class="input font-mono" disabled />
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
        <NuxtLink :to="`/servers/${serverId}`" class="btn btn-secondary">Cancel</NuxtLink>
        <button class="btn btn-primary" :disabled="submitting" @click="submit">
          {{ submitting ? 'Saving...' : 'Update Server' }}
        </button>
      </div>
    </div>
  </div>
</template>
