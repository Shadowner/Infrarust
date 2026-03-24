<script setup lang="ts">
import { KeyIcon, InformationCircleIcon } from '@heroicons/vue/24/outline';

definePageMeta({
  layout: 'auth',
});

const { setApiKey } = useAuth();
const { request } = useApi();
const { push } = useToast();

const apiKeyInput = ref('');
const loading = ref(false);

const connect = async () => {
  if (!apiKeyInput.value.trim()) {
    push({ type: 'error', title: 'API key required' });
    return;
  }

  loading.value = true;
  try {
    setApiKey(apiKeyInput.value.trim());
    await request('/proxy');
    push({ type: 'success', title: 'Connected to admin API' });
    await navigateTo('/dashboard');
  } catch {
    push({ type: 'error', title: 'Authentication failed', message: 'Check your API key.' });
  } finally {
    loading.value = false;
  }
};
</script>

<template>
  <div class="relative flex min-h-screen items-center justify-center p-4">
    <!-- Ambient background -->
    <div class="ambient-bg" />

    <div class="relative z-10 w-full max-w-md">
      <!-- Logo -->
      <div class="mb-8 flex flex-col items-center">
        <img src="/logo.svg" alt="Infrarust" class="h-16 w-16 drop-shadow-[0_0_24px_rgba(232,131,42,0.3)]" />
        <h1 class="mt-4 text-2xl font-bold tracking-tight">Infrarust</h1>
        <p class="mt-1 text-sm text-[var(--ir-text-muted)]">Proxy Administration</p>
      </div>

      <!-- Login card -->
      <div class="glass-pane relative overflow-hidden p-6 md:p-8">
        <div class="absolute inset-x-0 top-0 h-[3px] bg-[var(--ir-accent-gradient)]" />

        <h2 class="text-lg font-semibold">Connect to Control Plane</h2>
        <p class="mt-1.5 text-sm text-[var(--ir-text-muted)]">
          Enter your API key to access the dashboard.
        </p>

        <div class="mt-6 grid gap-4">
          <div>
            <label class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]" for="api-key">
              API Key
            </label>
            <div class="input-with-icon">
              <KeyIcon class="input-icon" />
              <input
                id="api-key"
                v-model="apiKeyInput"
                type="password"
                class="input"
                placeholder="infrarust_api_key"
                @keydown.enter.prevent="connect"
              />
            </div>
          </div>
          <button
            class="btn btn-primary w-full py-3"
            :disabled="loading"
            @click="connect"
          >
            <span v-if="loading" class="inline-flex items-center gap-2">
              <svg class="h-4 w-4 animate-spin" viewBox="0 0 24 24" fill="none">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" />
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
              </svg>
              Connecting...
            </span>
            <span v-else>Connect</span>
          </button>
        </div>

        <div class="mt-4 flex items-start gap-2 rounded-lg border border-[var(--ir-border)] bg-[var(--ir-bg-secondary)] p-3 text-xs text-[var(--ir-text-muted)]">
          <InformationCircleIcon class="mt-0.5 h-4 w-4 shrink-0 text-[var(--ir-accent)]" />
          <p>
            On first launch, the API key is logged in the proxy console and saved to
            <code class="rounded bg-[var(--ir-bg-tertiary)] px-1 py-0.5 text-[10px]">plugins/admin_api/config.toml</code>.
          </p>
        </div>

        <p class="mt-3 text-center text-[10px] text-[var(--ir-text-muted)]">
          Key stored in localStorage for this session.
        </p>
      </div>
    </div>
  </div>
</template>
