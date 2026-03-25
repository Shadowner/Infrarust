<script setup lang="ts">
import { ClockIcon } from '@heroicons/vue/24/outline';

const { state, dismiss } = useRateLimit();

let timer: ReturnType<typeof setTimeout> | null = null;
const countdown = ref(0);
const total = ref(60);

watch(() => state.value.isOpen, (open) => {
  if (open) {
    total.value = state.value.retryAfter;
    countdown.value = state.value.retryAfter;
    timer = setInterval(() => {
      countdown.value--;
      if (countdown.value <= 0) {
        if (timer) clearInterval(timer);
        dismiss();
      }
    }, 1000);
  } else {
    if (timer) {
      clearInterval(timer);
      timer = null;
    }
  }
});
</script>

<template>
  <Transition name="modal-overlay">
    <div
      v-if="state.isOpen"
      class="fixed inset-0 z-[var(--ir-z-modal)] flex items-center justify-center bg-black/60 p-4 backdrop-blur-md"
    >
      <Transition name="modal" appear>
        <div class="glass-pane relative w-full max-w-sm overflow-hidden p-6 text-center">
          <div class="absolute inset-x-0 top-0 h-[3px] bg-gradient-to-r from-[#cc3e38] to-[#e8832a]" />

          <div class="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-full bg-[#cc3e38]/15">
            <ClockIcon class="h-6 w-6 text-[#cc3e38]" />
          </div>

          <h2 class="text-lg font-semibold">Rate Limited</h2>
          <p class="mt-2 text-sm text-[var(--ir-text-muted)]">
            Too many requests. Please wait before retrying.
          </p>

          <div class="mt-4 text-3xl font-bold tabular-nums text-[var(--ir-accent)]">
            {{ countdown }}<span class="text-base font-normal text-[var(--ir-text-muted)]">s</span>
          </div>

          <div class="mt-3 h-[3px] w-full overflow-hidden rounded-full bg-white/[0.06]">
            <div
              class="h-full rounded-full bg-[var(--ir-accent)] transition-all duration-1000 ease-linear"
              :style="{ width: `${(countdown / total) * 100}%` }"
            />
          </div>
        </div>
      </Transition>
    </div>
  </Transition>
</template>
