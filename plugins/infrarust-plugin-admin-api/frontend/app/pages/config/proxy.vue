<script setup lang="ts">
import {
  ArrowLeftIcon,
  ArrowPathIcon,
  CheckCircleIcon,
  ExclamationTriangleIcon,
  XCircleIcon,
} from '@heroicons/vue/24/outline';
import type { ApiEnvelope, ProxyConfigDto, ValidationResultDto } from '~/types/api';

const { request, requestToml } = useApi();
const { push } = useToast();
const { ask } = useConfirm();
const { pending: restartPending, flag: flagRestart, clear: clearRestart } = useRestartRequired();

const config = ref<ProxyConfigDto | null>(null);
const savedToml = ref('');
const draft = ref('');
const saving = ref(false);

await useAsyncData('config:proxy', async () => {
  const [json, toml] = await Promise.all([
    request<ApiEnvelope<ProxyConfigDto>>('/config/proxy').then((r) => r.data).catch(() => null),
    requestToml('/config/proxy/raw').catch(() => ''),
  ]);
  config.value = json;
  savedToml.value = toml;
  draft.value = toml;
  return true;
});

const dirty = computed(() => draft.value !== savedToml.value);

const validation = ref<ValidationResultDto | null>(null);
const validating = ref(false);
const validationFailed = ref(false);
let validateTimer: ReturnType<typeof setTimeout> | null = null;

async function validate() {
  validating.value = true;
  validationFailed.value = false;
  try {
    const res = await request<ApiEnvelope<ValidationResultDto>>('/config/proxy/validate', {
      method: 'POST',
      body: draft.value,
      headers: { 'Content-Type': 'text/plain' },
    });
    validation.value = res.data;
  } catch {
    validation.value = null;
    validationFailed.value = true;
  } finally {
    validating.value = false;
  }
}

watch(draft, () => {
  if (validateTimer) clearTimeout(validateTimer);
  validateTimer = setTimeout(validate, 600);
});

onBeforeUnmount(() => { if (validateTimer) clearTimeout(validateTimer); });

async function save() {
  const confirmed = await ask(
    'Write the proxy config',
    'This rewrites infrarust.toml. Secrets you did not retype keep their current value.'
  );
  if (!confirmed) return;

  saving.value = true;
  try {
    await requestToml('/config/proxy/raw', draft.value);
    savedToml.value = draft.value;
    flagRestart();
    push({ type: 'success', title: 'Proxy config saved', message: 'Restart the proxy to apply it.' });
  } catch (e: unknown) {
    const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? 'Failed to save the config';
    push({ type: 'error', title: msg });
  } finally {
    saving.value = false;
  }
}

function revert() {
  draft.value = savedToml.value;
}

const summary = computed(() => {
  const c = config.value;
  if (!c) return [];
  return [
    { label: 'Bind address', value: c.bind ?? '—' },
    { label: 'Max connections', value: c.max_connections === 0 ? 'unlimited' : String(c.max_connections ?? '—') },
    { label: 'Connect timeout', value: c.connect_timeout ?? '—' },
    { label: 'Connect attempts', value: String(c.connect_max_attempts ?? '—') },
    { label: 'Worker threads', value: c.worker_threads === 0 ? 'auto' : String(c.worker_threads ?? '—') },
    { label: 'Servers dir', value: c.servers_dir ?? '—' },
    { label: 'Plugins dir', value: c.plugins_dir ?? '—' },
    { label: 'Receive PROXY protocol', value: c.receive_proxy_protocol ? 'yes' : 'no' },
    { label: 'SO_REUSEPORT', value: c.so_reuseport ? 'yes' : 'no' },
    { label: 'Unknown domain', value: c.unknown_domain_behavior ?? '—' },
    { label: 'Announce proxy commands', value: c.announce_proxy_commands ? 'yes' : 'no' },
  ];
});

const activeHealth = computed(() => config.value?.active_health ?? null);
const plugins = computed(() => Object.entries(config.value?.plugins ?? {}));
</script>

<template>
  <div class="grid gap-5">
    <NuxtLink to="/config" class="inline-flex items-center gap-1.5 text-sm text-[var(--ir-text-muted)] transition-colors hover:text-white">
      <ArrowLeftIcon class="h-4 w-4" />
      Back to configuration
    </NuxtLink>

    <div>
      <h2 class="text-xl font-bold tracking-tight">Proxy config</h2>
      <p class="mt-1 text-sm text-[var(--ir-text-muted)]">
        The global <span class="font-mono">infrarust.toml</span>. Secrets are stripped on read and kept on write.
      </p>
    </div>

    <div
      v-if="restartPending"
      class="glass-pane flex flex-wrap items-center gap-3 border-[rgba(233,160,71,0.3)] bg-[rgba(233,160,71,0.08)] p-4"
    >
      <ExclamationTriangleIcon class="h-4 w-4 shrink-0 text-[var(--ir-warn)]" />
      <p class="flex-1 text-sm text-[#ffd8ad]">
        The saved config is on disk but the running proxy still uses the old one. Restart it to apply.
      </p>
      <button class="btn btn-secondary text-xs" @click="clearRestart()">Dismiss</button>
    </div>

    <!-- Readable view -->
    <div class="grid gap-4 lg:grid-cols-2">
      <div class="glass-pane p-5">
        <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Runtime</h3>
        <dl v-if="summary.length" class="space-y-2 text-sm">
          <div v-for="row in summary" :key="row.label" class="flex justify-between gap-3">
            <dt class="text-[var(--ir-text-muted)]">{{ row.label }}</dt>
            <dd class="text-right font-mono text-xs">{{ row.value }}</dd>
          </div>
        </dl>
        <p v-else class="text-sm text-[var(--ir-text-muted)]">The proxy did not return a parsed config.</p>
      </div>

      <div class="grid gap-4">
        <div class="glass-pane p-5">
          <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Active health defaults</h3>
          <dl v-if="activeHealth" class="space-y-2 text-sm">
            <div class="flex justify-between gap-3"><dt class="text-[var(--ir-text-muted)]">Enabled</dt><dd class="font-mono text-xs">{{ activeHealth.enabled ? 'yes' : 'no' }}</dd></div>
            <div class="flex justify-between gap-3"><dt class="text-[var(--ir-text-muted)]">Probe kind</dt><dd class="font-mono text-xs">{{ activeHealth.kind ?? 'tcp' }}</dd></div>
            <div class="flex justify-between gap-3"><dt class="text-[var(--ir-text-muted)]">Recovery interval</dt><dd class="font-mono text-xs">{{ activeHealth.unhealthy_interval ?? '—' }}</dd></div>
            <div class="flex justify-between gap-3"><dt class="text-[var(--ir-text-muted)]">Sweep interval</dt><dd class="font-mono text-xs">{{ activeHealth.interval ?? '—' }}</dd></div>
            <div class="flex justify-between gap-3"><dt class="text-[var(--ir-text-muted)]">Probe timeout</dt><dd class="font-mono text-xs">{{ activeHealth.timeout ?? '—' }}</dd></div>
            <div class="flex justify-between gap-3"><dt class="text-[var(--ir-text-muted)]">Probe healthy too</dt><dd class="font-mono text-xs">{{ activeHealth.probe_healthy ? 'yes' : 'no' }}</dd></div>
            <div class="flex justify-between gap-3"><dt class="text-[var(--ir-text-muted)]">Max concurrent</dt><dd class="font-mono text-xs">{{ activeHealth.max_concurrent ?? '—' }}</dd></div>
          </dl>
          <p v-else class="text-sm text-[var(--ir-text-muted)]">Not reported.</p>
        </div>

        <div class="glass-pane p-5">
          <h3 class="mb-3 text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Plugins</h3>
          <ul v-if="plugins.length" class="space-y-2 text-sm">
            <li v-for="[id, plugin] in plugins" :key="id" class="flex items-center justify-between gap-3">
              <span class="font-mono text-xs">{{ id }}</span>
              <StatusBadge :status="plugin.enabled === false ? 'Disabled' : 'Enabled'" />
            </li>
          </ul>
          <p v-else class="text-sm text-[var(--ir-text-muted)]">No plugins configured.</p>
        </div>
      </div>
    </div>

    <!-- Raw editor -->
    <div class="glass-pane relative overflow-hidden p-5">
      <div class="absolute inset-x-0 top-0 h-[3px] bg-[var(--ir-accent-gradient)]" />

      <div class="mb-3 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h3 class="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">Raw TOML</h3>
          <p class="mt-1 text-[11px] text-[var(--ir-text-muted)]">
            Omitting a secret keeps the value already on disk, so an edit-and-save round trip is safe.
          </p>
        </div>
        <div class="flex items-center gap-2">
          <span v-if="dirty" class="text-[11px] text-[var(--ir-warn)]">unsaved changes</span>
          <button class="btn btn-secondary flex items-center gap-1.5 text-xs" :disabled="validating" @click="validate">
            <ArrowPathIcon class="h-3.5 w-3.5" :class="validating ? 'animate-spin' : ''" />
            Validate
          </button>
          <button class="btn btn-secondary text-xs" :disabled="!dirty" @click="revert">Revert</button>
          <button class="btn btn-primary text-xs" :disabled="saving || !dirty" @click="save">
            {{ saving ? 'Saving...' : 'Save' }}
          </button>
        </div>
      </div>

      <textarea
        v-model="draft"
        aria-label="Proxy configuration TOML"
        spellcheck="false"
        rows="24"
        class="input font-mono text-xs leading-relaxed"
      />

      <div class="mt-3 rounded-[var(--ir-radius-md)] border border-[var(--ir-border)] bg-[var(--ir-surface-soft)] p-4">
        <p v-if="validationFailed" class="text-xs text-[var(--ir-text-muted)]">
          The proxy did not answer the validation request.
        </p>
        <p v-else-if="!validation" class="text-xs text-[var(--ir-text-muted)]">
          {{ validating ? 'Validating...' : 'Edit the TOML or press Validate.' }}
        </p>
        <template v-else>
          <p
            class="flex items-center gap-2 text-sm font-medium"
            :class="validation.valid ? 'text-[#bce5b6]' : 'text-[#ffc0bc]'"
          >
            <CheckCircleIcon v-if="validation.valid" class="h-4 w-4" />
            <XCircleIcon v-else class="h-4 w-4" />
            {{ validation.valid ? 'Valid configuration' : `${validation.errors.length} error(s)` }}
          </p>

          <ul v-if="validation.errors.length" class="mt-3 grid gap-1.5">
            <li
              v-for="(error, index) in validation.errors"
              :key="`e-${index}`"
              class="flex items-start gap-2 rounded-lg border border-[rgba(204,62,56,0.25)] bg-[rgba(204,62,56,0.1)] px-3 py-2 text-xs text-[#ffc0bc]"
            >
              <XCircleIcon class="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span class="font-mono">{{ error }}</span>
            </li>
          </ul>

          <ul v-if="validation.warnings.length" class="mt-3 grid gap-1.5">
            <li
              v-for="(warning, index) in validation.warnings"
              :key="`w-${index}`"
              class="flex items-start gap-2 rounded-lg border border-[rgba(233,160,71,0.25)] bg-[rgba(233,160,71,0.1)] px-3 py-2 text-xs text-[#ffd8ad]"
            >
              <ExclamationTriangleIcon class="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span class="font-mono">{{ warning }}</span>
            </li>
          </ul>
        </template>
      </div>
    </div>
  </div>
</template>
