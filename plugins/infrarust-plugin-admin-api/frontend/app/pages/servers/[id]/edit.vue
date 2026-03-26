<script setup lang="ts">
import { ArrowLeftIcon } from '@heroicons/vue/24/outline';
import type { ServerDetailDto, ApiEnvelope, MutationResult, UpdateServerRequest } from '~/types/api';

const route = useRoute();
const { request } = useApi();
const { push } = useToast();
const serverId = route.params.id as string;

const server = ref<ServerDetailDto | null>(null);
const submitting = ref(false);

const { data } = await useAsyncData(`server-edit:${serverId}`, async () => {
  const res = await request<ApiEnvelope<ServerDetailDto>>(`/servers/${encodeURIComponent(serverId)}`);
  return res.data;
});

if (data.value) {
  server.value = data.value;
}

const initialData = computed(() => {
  if (!server.value) return undefined;
  return {
    domains: server.value.domains,
    addresses: server.value.addresses,
    mode: server.value.proxy_mode,
    handlers: server.value.limbo_handlers ?? [],
  };
});

async function handleUpdate(payload: { domains: string[]; addresses: string[]; mode: string; handlers: string[] }) {
  submitting.value = true;
  try {
    const body: UpdateServerRequest = {
      domains: payload.domains,
      addresses: payload.addresses,
      proxy_mode: payload.mode,
      limbo_handlers: payload.handlers.length > 0 ? payload.handlers : undefined,
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

    <ServerForm
      v-else-if="server"
      :is-edit="true"
      :server-id="serverId"
      :initial-data="initialData"
      @submit="handleUpdate"
      @cancel="navigateTo(`/servers/${serverId}`)"
    />
  </div>
</template>
