<script setup lang="ts">
import { ArrowLeftIcon } from '@heroicons/vue/24/outline';
import type { ApiEnvelope, MutationResult, ServerConfig } from '~/types/api';

const { request } = useApi();
const { push } = useToast();
const submitting = ref(false);

async function handleSubmit(payload: { kind: 'config'; config: ServerConfig } | { kind: 'raw'; toml: string }) {
  if (payload.kind === 'raw') {
    push({ type: 'error', title: 'Raw TOML is only available on an existing server' });
    return;
  }

  submitting.value = true;
  try {
    await request<ApiEnvelope<MutationResult>>('/servers', {
      method: 'POST',
      body: payload.config as unknown as Record<string, unknown>,
    });
    // The proxy files the server under `name ?? id`, so a Display name becomes its identity.
    const createdId = payload.config.name ?? payload.config.id;
    push({ type: 'success', title: `Server '${createdId}' created` });
    navigateTo(`/servers/${createdId}`);
  } catch (e: unknown) {
    const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? 'Failed to create server';
    push({ type: 'error', title: msg });
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div class="grid gap-5">
    <NuxtLink to="/servers" class="inline-flex items-center gap-1.5 text-sm text-[var(--ir-text-muted)] hover:text-white transition-colors">
      <ArrowLeftIcon class="h-4 w-4" />
      Back to servers
    </NuxtLink>

    <div>
      <div class="flex items-center gap-2">
        <h2 class="text-xl font-bold tracking-tight">Create server</h2>
        <span class="rounded bg-[var(--ir-accent-soft)] px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-[var(--ir-accent)]">API</span>
      </div>
      <p class="mt-1 text-sm text-[var(--ir-text-muted)]">Add a new API-managed server to the proxy.</p>
    </div>

    <ServerForm
      :is-edit="false"
      :submitting="submitting"
      @submit="handleSubmit"
      @cancel="navigateTo('/servers')"
    />
  </div>
</template>
