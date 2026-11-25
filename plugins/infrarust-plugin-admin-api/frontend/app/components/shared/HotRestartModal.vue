<script setup lang="ts">
import { ArrowPathRoundedSquareIcon } from '@heroicons/vue/24/outline';

const MAX_DELAY_SECONDS = 3600;
const MAX_MESSAGE_LENGTH = 256;
const DELAYS = [
  { label: 'Configured', value: 'configured' },
  { label: 'Now', value: '0' },
  { label: '10 s', value: '10' },
  { label: '30 s', value: '30' },
  { label: '1 min', value: '60' },
  { label: '5 min', value: '300' },
  { label: 'Other', value: 'custom' },
];

const { state, close, submit } = useHotRestart();

const delay = ref('configured');
const customSeconds = ref(120);
const message = ref('');
const submitting = ref(false);

const delaySeconds = computed<number | null>(() => {
  if (delay.value === 'configured') return null;
  if (delay.value === 'custom') return Math.floor(Number(customSeconds.value));
  return Number(delay.value);
});

const delayError = computed(() => {
  const seconds = delaySeconds.value;
  if (seconds === null) return '';
  if (!Number.isFinite(seconds) || seconds < 0) return 'Enter a number of seconds';
  if (seconds > MAX_DELAY_SECONDS) return 'The longest delay is one hour';
  return '';
});

const messageLength = computed(() => [...message.value.trim()].length);
const messageError = computed(() =>
  messageLength.value > MAX_MESSAGE_LENGTH ? `The message is limited to ${MAX_MESSAGE_LENGTH} characters` : '',
);
const invalid = computed(() => Boolean(delayError.value || messageError.value));

watch(
  () => state.value.isOpen,
  (open) => {
    if (!open) return;
    delay.value = 'configured';
    message.value = '';
  },
);

async function confirm() {
  if (invalid.value || submitting.value) return;
  submitting.value = true;
  try {
    await submit({ delaySeconds: delaySeconds.value, message: message.value });
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <Transition name="modal-overlay">
    <div
      v-if="state.isOpen"
      class="fixed inset-0 z-[var(--ir-z-modal)] flex items-center justify-center bg-black/60 p-4 backdrop-blur-md"
      @click.self="close"
      @keydown.esc="close"
    >
      <Transition name="modal" appear>
        <div class="glass-pane relative w-full max-w-lg overflow-hidden p-6" role="dialog" aria-labelledby="hot-restart-title">
          <div class="absolute inset-x-0 top-0 h-[3px] bg-[var(--ir-accent-gradient)]" />

          <div class="flex items-center gap-3">
            <div class="flex h-9 w-9 items-center justify-center rounded-lg bg-[var(--ir-accent-soft)]">
              <ArrowPathRoundedSquareIcon class="h-4.5 w-4.5 text-[var(--ir-accent)]" />
            </div>
            <div>
              <h2 id="hot-restart-title" class="text-lg font-semibold">Hot Restart</h2>
              <p class="text-xs text-[var(--ir-text-muted)]">The proxy restarts in place</p>
            </div>
          </div>

          <ul class="mt-4 list-disc space-y-1 pl-5 text-xs text-[var(--ir-text-muted)]">
            <li>Players on passthrough, zero copy and server only servers stay connected.</li>
            <li>Players on offline and client only servers come back on their own on Minecraft 1.20.5 and later.</li>
            <li>Older clients on those servers are disconnected with a message.</li>
          </ul>

          <div class="mt-5">
            <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]">
              Delay
            </span>
            <div class="flex flex-wrap gap-1.5">
              <button
                v-for="option in DELAYS"
                :key="option.value"
                type="button"
                class="rounded-md border px-2.5 py-1 text-xs font-medium transition-colors"
                :class="delay === option.value
                  ? 'border-[var(--ir-accent)] bg-[var(--ir-accent-soft)] text-[var(--ir-text)]'
                  : 'border-[var(--ir-border)] text-[var(--ir-text-muted)] hover:text-[var(--ir-text)]'"
                :aria-pressed="delay === option.value"
                @click="delay = option.value"
              >
                {{ option.label }}
              </button>
            </div>
            <div v-if="delay === 'custom'" class="mt-2 flex items-center gap-2">
              <input
                id="hot-restart-seconds"
                v-model.number="customSeconds"
                type="number"
                min="0"
                :max="MAX_DELAY_SECONDS"
                class="input w-28"
              >
              <label for="hot-restart-seconds" class="text-xs text-[var(--ir-text-muted)]">seconds</label>
            </div>
            <p v-if="delayError" class="mt-1.5 text-[11px] text-[var(--ir-danger)]">{{ delayError }}</p>
            <p v-else-if="delay === 'configured'" class="mt-1.5 text-[10px] text-[var(--ir-text-muted)]">
              Uses the delay and the message of the [restart] section of the proxy configuration.
            </p>
          </div>

          <div class="mt-5">
            <label
              for="hot-restart-message"
              class="mb-1.5 block text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--ir-text-muted)]"
            >
              Message to the players
            </label>
            <textarea
              id="hot-restart-message"
              v-model="message"
              class="input min-h-[72px] resize-y"
              placeholder="&amp;eThe proxy restarts in {delay}"
            />
            <p v-if="messageError" class="mt-1.5 text-[11px] text-[var(--ir-danger)]">{{ messageError }}</p>
            <p v-else class="mt-1.5 text-[10px] text-[var(--ir-text-muted)]">
              Optional. {delay} and {seconds} show the time left, &amp; codes set colors.
              Only players on offline and client only servers can receive it.
            </p>
          </div>

          <div class="mt-5 flex justify-end gap-2">
            <button class="btn btn-secondary" @click="close">Cancel</button>
            <button
              class="btn btn-primary flex items-center gap-2"
              :disabled="submitting || invalid"
              @click="confirm"
            >
              <ArrowPathRoundedSquareIcon class="h-4 w-4" />
              {{ submitting ? 'Requesting...' : 'Restart' }}
            </button>
          </div>
        </div>
      </Transition>
    </div>
  </Transition>
</template>
