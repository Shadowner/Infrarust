<script setup lang="ts">
const { state, close } = useConfirm();
</script>

<template>
  <Transition name="modal-overlay">
    <div
      v-if="state.isOpen"
      class="fixed inset-0 z-[var(--ir-z-modal)] flex items-center justify-center bg-black/60 p-4 backdrop-blur-md"
      @click.self="close(false)"
    >
      <Transition name="modal" appear>
        <div class="glass-pane relative w-full max-w-md overflow-hidden p-6">
          <!-- Accent top stripe -->
          <div class="absolute inset-x-0 top-0 h-[3px] bg-[var(--ir-accent-gradient)]" />

          <p class="accent-chip">Confirm Action</p>
          <h2 class="mt-3 text-lg font-semibold">{{ state.title }}</h2>
          <p class="mt-2 text-sm text-[var(--ir-text-muted)]">{{ state.message }}</p>
          <div class="mt-5 flex justify-end gap-2">
            <button class="btn btn-secondary" @click="close(false)">Cancel</button>
            <button class="btn btn-danger" @click="close(true)">Confirm</button>
          </div>
        </div>
      </Transition>
    </div>
  </Transition>
</template>
