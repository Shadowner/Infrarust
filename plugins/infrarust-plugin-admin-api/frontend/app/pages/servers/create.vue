<script setup lang="ts">
import { ArrowLeftIcon } from '@heroicons/vue/24/outline';
import type { CreateServerRequest, ApiEnvelope, MutationResult } from '~/types/api';

const { request } = useApi();
const { push } = useToast();
const submitting = ref(false);

async function handleCreate(payload: { id?: string; domains: string[]; addresses: string[]; mode: string; handlers: string[] }) {
  submitting.value = true;
  try {
    const body: CreateServerRequest = {
      id: payload.id!,
      domains: payload.domains,
      addresses: payload.addresses,
      proxy_mode: payload.mode,
      limbo_handlers: payload.handlers.length > 0 ? payload.handlers : undefined,
    };
    await request<ApiEnvelope<MutationResult>>('/servers', {
      method: 'POST',
      body: body as unknown as Record<string, unknown>,
    });
    push({ type: 'success', title: `Server '${payload.id}' created` });
    navigateTo(`/servers/${payload.id}`);
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
        <h2 class="text-xl font-bold tracking-tight">Create Server</h2>
        <span class="rounded bg-[var(--ir-accent-soft)] px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-[var(--ir-accent)]">API</span>
      </div>
      <p class="mt-1 text-sm text-[var(--ir-text-muted)]">Add a new API-managed server to the proxy.</p>
    </div>

    <ServerForm
      :is-edit="false"
      @submit="handleCreate"
      @cancel="navigateTo('/servers')"
    />
  </div>
</template>
