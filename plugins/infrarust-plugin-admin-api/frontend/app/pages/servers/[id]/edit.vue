<script setup lang="ts">
import { ArrowLeftIcon, ExclamationTriangleIcon, LockClosedIcon } from '@heroicons/vue/24/outline';
import type { ApiEnvelope, MutationResult, ServerConfig, ServerDetailDto } from '~/types/api';

const route = useRoute();
const { request, requestToml } = useApi();
const { push } = useToast();
const serverId = route.params.id as string;
const encodedId = encodeURIComponent(serverId);
const submitting = ref(false);

const { data } = await useAsyncData(`server-edit:${serverId}`, async () => {
  const [detail, config, raw] = await Promise.all([
    request<ApiEnvelope<ServerDetailDto>>(`/servers/${encodedId}`).then((r) => r.data).catch(() => null),
    request<ApiEnvelope<ServerConfig>>(`/servers/${encodedId}/config`).then((r) => r.data).catch(() => null),
    requestToml(`/servers/${encodedId}/raw`).catch(() => undefined),
  ]);
  return { detail, config, raw };
});

const detail = computed(() => data.value?.detail ?? null);
const rawToml = computed(() => data.value?.raw);
const configIncomplete = computed(() => data.value != null && data.value.config == null);

/** Falls back to the list DTO so the form still opens when `/config` is unavailable. */
const config = computed<ServerConfig | undefined>(() => {
  if (data.value?.config) return data.value.config;
  const dto = detail.value;
  if (!dto) return undefined;
  return {
    id: dto.id,
    domains: dto.domains,
    addresses: dto.addresses,
    proxy_mode: dto.proxy_mode as ServerConfig['proxy_mode'],
    limbo_handlers: dto.limbo_handlers,
  };
});

async function handleSubmit(payload: { kind: 'config'; config: ServerConfig } | { kind: 'raw'; toml: string }) {
  submitting.value = true;
  try {
    if (payload.kind === 'raw') {
      await requestToml(`/servers/${encodedId}/raw`, payload.toml);
    } else {
      await request<ApiEnvelope<MutationResult>>(`/servers/${encodedId}`, {
        method: 'PUT',
        body: payload.config as unknown as Record<string, unknown>,
      });
    }
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
        <h2 class="text-xl font-bold tracking-tight">Edit server</h2>
        <span v-if="detail" class="rounded bg-[var(--ir-surface-soft)] px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-[var(--ir-text-muted)]">
          {{ detail.source }}
        </span>
      </div>
      <p class="mt-1 text-sm text-[var(--ir-text-muted)]">Saving replaces the whole configuration of {{ serverId }}.</p>
    </div>

    <div v-if="detail && !detail.editable" class="glass-pane flex items-start gap-3 p-5">
      <LockClosedIcon class="mt-0.5 h-4 w-4 shrink-0 text-[var(--ir-text-muted)]" />
      <p class="text-sm text-[var(--ir-text-muted)]">
        This server comes from the <span class="font-mono">{{ detail.source }}</span> provider and is read-only here.
      </p>
    </div>

    <template v-else-if="config">
      <div
        v-if="configIncomplete"
        class="glass-pane flex items-start gap-3 border-[rgba(233,160,71,0.3)] bg-[rgba(233,160,71,0.08)] p-5"
      >
        <ExclamationTriangleIcon class="mt-0.5 h-4 w-4 shrink-0 text-[var(--ir-warn)]" />
        <p class="text-sm text-[#ffd8ad]">
          Only the summary fields loaded, so a full replace would drop everything else this server defines.
          Use the raw TOML editor on the Advanced tab, which is seeded from the server's real file.
        </p>
      </div>

      <ServerForm
        :is-edit="true"
        :server-id="serverId"
        :config="config"
        :raw-toml="rawToml"
        :submitting="submitting"
        @submit="handleSubmit"
        @cancel="navigateTo(`/servers/${serverId}`)"
      />
    </template>
  </div>
</template>
