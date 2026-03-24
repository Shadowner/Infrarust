<template>
  <div class="min-h-screen">
    <NavRail />
    <StatusTicker />

    <!-- Main content area -->
    <main
      class="relative min-h-screen p-4 pt-[calc(var(--ir-ticker-height)+1rem)] lg:pl-[calc(var(--ir-rail-width)+1.5rem)] lg:pr-6 lg:pt-[calc(var(--ir-ticker-height)+1.5rem)] lg:pb-6"
      :class="'pb-[calc(var(--ir-tab-bar-height)+1rem)]'"
    >
      <div class="mx-auto max-w-[1600px]">
        <slot />
      </div>
    </main>
  </div>
</template>
