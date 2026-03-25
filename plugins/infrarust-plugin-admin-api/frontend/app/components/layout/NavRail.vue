<script setup lang="ts">
import {
  HomeIcon,
  UsersIcon,
  ShieldExclamationIcon,
  ServerStackIcon,
  PuzzlePieceIcon,
  DocumentTextIcon,
  Cog6ToothIcon,
  ArrowRightStartOnRectangleIcon,
  EllipsisHorizontalIcon,
} from '@heroicons/vue/24/outline';

const route = useRoute();
const { status } = useEventBus();
const auth = useAuth();
const { ask } = useConfirm();

const expanded = ref(false);
const mobileMore = ref(false);

const navItems = [
  { to: '/dashboard', label: 'Dashboard', icon: HomeIcon },
  { to: '/players', label: 'Players', icon: UsersIcon },
  { to: '/bans', label: 'Bans', icon: ShieldExclamationIcon },
  { to: '/servers', label: 'Servers', icon: ServerStackIcon },
  { to: '/plugins', label: 'Plugins', icon: PuzzlePieceIcon },
  { to: '/logs', label: 'Logs', icon: DocumentTextIcon },
  { to: '/config', label: 'Config', icon: Cog6ToothIcon },
];

const mobileMainNav = navItems.slice(0, 4);
const mobileOverflowNav = navItems.slice(4);

const isActive = (to: string) => route.path === to || route.path.startsWith(to + '/');

const activeIndex = computed(() => navItems.findIndex((item) => isActive(item.to)));

async function handleLogout() {
  const confirmed = await ask('Switch API Key', 'Disconnect and return to login?');
  if (confirmed) {
    auth.clear();
    navigateTo('/login');
  }
}
</script>

<template>
  <!-- Desktop Rail -->
  <aside
    class="fixed inset-y-0 left-0 z-[var(--ir-z-rail)] hidden flex-col border-r border-[var(--ir-border)] lg:flex"
    :class="expanded ? 'w-[var(--ir-rail-expanded)]' : 'w-[var(--ir-rail-width)]'"
    :style="{
      background: 'var(--ir-glass-bg-strong)',
      backdropFilter: 'blur(20px) saturate(1.4)',
      WebkitBackdropFilter: 'blur(20px) saturate(1.4)',
      transition: 'width var(--ir-duration-normal) var(--ir-ease-spring)',
    }"
    @mouseenter="expanded = true"
    @mouseleave="expanded = false"
  >
    <!-- Logo — colored when connected, grayscale when disconnected -->
    <div class="flex h-[var(--ir-ticker-height)] items-center gap-3 overflow-hidden border-b border-[var(--ir-border)] px-4">
      <img
        src="/logo.svg"
        alt="Infrarust"
        class="h-8 w-8 shrink-0 transition-[filter] duration-500"
        :class="status === 'open' ? '' : 'grayscale opacity-50'"
      />
      <Transition name="fade">
        <div v-if="expanded" class="flex flex-col overflow-hidden">
          <span class="whitespace-nowrap text-sm font-bold tracking-tight">Infrarust</span>
          <span
            class="whitespace-nowrap text-[10px] transition-colors duration-300"
            :class="status === 'open' ? 'text-[var(--ir-success)]' : 'text-[var(--ir-danger)]'"
          >
            {{ status === 'open' ? 'Connected' : 'Offline' }}
          </span>
        </div>
      </Transition>
    </div>

    <!-- Nav Items -->
    <nav class="relative flex flex-1 flex-col gap-1 px-2 py-3">
      <!-- Sliding active indicator -->
      <div
        v-if="activeIndex >= 0"
        class="absolute left-0 w-[3px] rounded-r-sm bg-[var(--ir-accent)]"
        :style="{
          height: '28px',
          top: `${12 + activeIndex * 44 + 8}px`,
          transition: 'top var(--ir-duration-normal) var(--ir-ease-spring)',
        }"
      />

      <NuxtLink
        v-for="item in navItems"
        :key="item.to"
        :to="item.to"
        class="group flex items-center gap-3 rounded-lg px-3 py-2.5 transition-colors duration-150"
        :class="
          isActive(item.to)
            ? 'bg-[var(--ir-accent-soft)] text-white'
            : 'text-[var(--ir-text-muted)] hover:bg-white/[0.04] hover:text-white'
        "
      >
        <component :is="item.icon" class="h-5 w-5 shrink-0" />
        <Transition name="fade">
          <span v-if="expanded" class="whitespace-nowrap text-sm font-medium">{{ item.label }}</span>
        </Transition>
      </NuxtLink>
    </nav>

    <!-- Bottom -->
    <div class="flex flex-col gap-2 border-t border-[var(--ir-border)] px-2 py-3">
      <!-- Logout -->
      <button
        class="flex items-center gap-3 rounded-lg px-3 py-2.5 text-[var(--ir-text-muted)] transition-colors duration-150 hover:bg-white/[0.04] hover:text-white"
        @click="handleLogout"
      >
        <ArrowRightStartOnRectangleIcon class="h-5 w-5 shrink-0" />
        <Transition name="fade">
          <span v-if="expanded" class="whitespace-nowrap text-sm font-medium">Switch Key</span>
        </Transition>
      </button>
    </div>
  </aside>

  <!-- Mobile Bottom Tab Bar -->
  <nav
    class="fixed inset-x-0 bottom-0 z-[var(--ir-z-rail)] flex items-center justify-around border-t border-[var(--ir-border)] lg:hidden"
    :style="{
      height: 'var(--ir-tab-bar-height)',
      background: 'var(--ir-glass-bg-strong)',
      backdropFilter: 'blur(20px) saturate(1.4)',
      WebkitBackdropFilter: 'blur(20px) saturate(1.4)',
    }"
  >
    <NuxtLink
      v-for="item in mobileMainNav"
      :key="item.to"
      :to="item.to"
      class="flex flex-col items-center gap-0.5 px-2 py-1.5"
      :class="isActive(item.to) ? 'text-[var(--ir-accent)]' : 'text-[var(--ir-text-muted)]'"
    >
      <component :is="item.icon" class="h-5 w-5" />
      <span class="text-[10px] font-medium">{{ item.label }}</span>
    </NuxtLink>

    <!-- More button -->
    <div class="relative">
      <button
        class="flex flex-col items-center gap-0.5 px-2 py-1.5"
        :class="mobileMore ? 'text-[var(--ir-accent)]' : 'text-[var(--ir-text-muted)]'"
        @click="mobileMore = !mobileMore"
      >
        <EllipsisHorizontalIcon class="h-5 w-5" />
        <span class="text-[10px] font-medium">More</span>
      </button>

      <!-- Overflow menu -->
      <Transition name="modal">
        <div
          v-if="mobileMore"
          class="absolute bottom-full right-0 mb-2 glass-pane min-w-[160px] p-2"
          @click="mobileMore = false"
        >
          <NuxtLink
            v-for="item in mobileOverflowNav"
            :key="item.to"
            :to="item.to"
            class="flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm"
            :class="isActive(item.to) ? 'bg-[var(--ir-accent-soft)] text-white' : 'text-[var(--ir-text-muted)] hover:text-white'"
          >
            <component :is="item.icon" class="h-4 w-4" />
            <span>{{ item.label }}</span>
          </NuxtLink>
        </div>
      </Transition>
    </div>
  </nav>
</template>

<style scoped>
.fade-enter-active { transition: opacity var(--ir-duration-fast) ease; }
.fade-leave-active { transition: opacity 0.08s ease; }
.fade-enter-from, .fade-leave-to { opacity: 0; }
</style>
