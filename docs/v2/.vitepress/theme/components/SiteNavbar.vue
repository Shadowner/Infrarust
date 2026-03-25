<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { withBase } from 'vitepress'
import { useGitHubStars } from '../composables/useGitHubStars'
import { GITHUB_REPO, GITHUB_URL, DISCORD_URL, VERSION } from '../constants'
import IconLogo from '../icons/IconLogo.vue'
import IconGitHub from '../icons/IconGitHub.vue'
import IconDiscord from '../icons/IconDiscord.vue'
import IconArrow from '../icons/IconArrow.vue'

const scrolled = ref(false)
const menuOpen = ref(false)
const stars = useGitHubStars(GITHUB_REPO)

let onScroll = null

onMounted(() => {
  onScroll = () => {
    scrolled.value = window.scrollY > 20
  }
  window.addEventListener('scroll', onScroll, { passive: true })
})

onUnmounted(() => {
  if (onScroll) window.removeEventListener('scroll', onScroll)
})
</script>

<template>
  <nav class="site-navbar" :class="{ scrolled, 'menu-open': menuOpen }">
    <div class="navbar-spotlight" />

    <div class="navbar-inner">
      <a :href="withBase('/')" class="navbar-brand">
        <div class="brand-logo-wrap">
          <IconLogo :size="36" />
        </div>
        <div class="brand-text">
          <span class="brand-name">Infrarust</span>
          <span class="brand-version">{{ VERSION }}</span>
        </div>
      </a>

      <div class="navbar-links">
        <a :href="withBase('/guide/')" class="nav-link">
          <span>Guide</span>
        </a>
        <a :href="withBase('/configuration/')" class="nav-link">
          <span>Configuration</span>
        </a>
        <a :href="withBase('/plugins/')" class="nav-link">
          <span>Plugins</span>
        </a>
        <a :href="withBase('/reference/')" class="nav-link">
          <span>Reference</span>
        </a>
      </div>

      <div class="navbar-actions">
        <a
          :href="DISCORD_URL"
          target="_blank"
          rel="noopener"
          class="icon-pill"
          title="Join Discord"
        >
          <IconDiscord :size="17" />
        </a>

        <a
          :href="GITHUB_URL"
          target="_blank"
          rel="noopener"
          class="github-pill"
          title="View on GitHub"
        >
          <IconGitHub :size="16" />
          <span v-if="stars" class="github-stars">&#9733; {{ stars }}</span>
        </a>

        <a :href="withBase('/guide/')" class="navbar-cta">
          Get Started
          <IconArrow :size="14" />
        </a>

        <button class="hamburger" :class="{ open: menuOpen }" @click="menuOpen = !menuOpen" aria-label="Toggle menu">
          <span /><span /><span />
        </button>
      </div>
    </div>

    <div class="mobile-menu" :class="{ open: menuOpen }">
      <a :href="withBase('/guide/')" class="mobile-link" @click="menuOpen = false">Guide</a>
      <a :href="withBase('/configuration/')" class="mobile-link" @click="menuOpen = false">Configuration</a>
      <a :href="withBase('/plugins/')" class="mobile-link" @click="menuOpen = false">Plugins</a>
      <a :href="withBase('/reference/')" class="mobile-link" @click="menuOpen = false">Reference</a>
      <div class="mobile-divider" />
      <a :href="GITHUB_URL" target="_blank" rel="noopener" class="mobile-link">
        GitHub <span v-if="stars" class="mobile-stars">&#9733; {{ stars }}</span>
      </a>
      <a :href="DISCORD_URL" target="_blank" rel="noopener" class="mobile-link">Discord</a>
      <a :href="withBase('/guide/')" class="mobile-cta" @click="menuOpen = false">Get Started</a>
    </div>
  </nav>
</template>

<style scoped>
.site-navbar {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  z-index: 200;
  height: 68px;
  background: rgba(12, 12, 12, 0.75);
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  transition: background 0.3s, box-shadow 0.3s;
}

.site-navbar.scrolled {
  background: rgba(9, 9, 9, 0.92);
  box-shadow: 0 1px 0 rgba(255, 255, 255, 0.05), 0 4px 24px rgba(0, 0, 0, 0.4);
}

/* Spotlight underline — fades in on scroll */
.navbar-spotlight {
  position: absolute;
  bottom: 0;
  left: 50%;
  transform: translateX(-50%);
  width: 480px;
  height: 1px;
  background: linear-gradient(
    90deg,
    transparent 0%,
    rgba(232, 131, 42, 0.6) 50%,
    transparent 100%
  );
  opacity: 0;
  transition: opacity 0.4s;
}

.site-navbar.scrolled .navbar-spotlight {
  opacity: 1;
}

.navbar-inner {
  max-width: 1280px;
  margin: 0 auto;
  height: 68px;
  display: flex;
  align-items: center;
  padding: 0 28px;
  gap: 0;
}

.navbar-brand {
  display: flex;
  align-items: center;
  gap: 12px;
  text-decoration: none;
  flex-shrink: 0;
  margin-right: auto;
}

.brand-logo-wrap {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 44px;
  height: 44px;
  border-radius: 10px;
  background: rgba(232, 131, 42, 0.08);
  border: 1px solid rgba(232, 131, 42, 0.18);
  transition: background 0.2s, border-color 0.2s, box-shadow 0.2s;
}

.navbar-brand:hover .brand-logo-wrap {
  background: rgba(232, 131, 42, 0.14);
  border-color: rgba(232, 131, 42, 0.35);
  box-shadow: 0 0 16px rgba(232, 131, 42, 0.15);
}

.brand-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  line-height: 1;
}

.brand-name {
  font-family: var(--ir-font-sans);
  font-weight: 700;
  font-size: 17px;
  letter-spacing: -0.03em;
  color: var(--ir-text);
}

.brand-version {
  font-family: var(--ir-font-mono);
  font-size: 10px;
  font-weight: 500;
  color: var(--ir-accent);
  letter-spacing: 0.04em;
  opacity: 0.85;
}

.navbar-links {
  display: flex;
  align-items: center;
  gap: 4px;
  margin: 0 auto;
}

.nav-link {
  position: relative;
  font-family: var(--ir-font-sans);
  font-size: 14px;
  font-weight: 500;
  color: rgba(240, 237, 232, 0.55);
  text-decoration: none;
  padding: 6px 12px;
  border-radius: 6px;
  transition: color 0.2s, background 0.2s;
}

.nav-link:hover {
  color: var(--ir-text);
  background: rgba(255, 255, 255, 0.05);
}

.navbar-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-shrink: 0;
  margin-left: auto;
}

.icon-pill {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: rgba(240, 237, 232, 0.55);
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.08);
  width: 36px;
  height: 36px;
  border-radius: 8px;
  text-decoration: none;
  transition: color 0.2s, border-color 0.2s, background 0.2s;
}

.icon-pill:hover {
  color: var(--ir-discord);
  border-color: rgba(88, 101, 242, 0.4);
  background: rgba(88, 101, 242, 0.08);
}

.github-pill {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  font-family: var(--ir-font-sans);
  font-size: 13px;
  font-weight: 500;
  color: rgba(240, 237, 232, 0.55);
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.08);
  padding: 6px 12px;
  border-radius: 8px;
  text-decoration: none;
  transition: color 0.2s, border-color 0.2s, background 0.2s;
}

.github-pill:hover {
  color: var(--ir-text);
  border-color: rgba(255, 255, 255, 0.18);
  background: rgba(255, 255, 255, 0.07);
}

.github-stars {
  color: var(--ir-accent);
  font-weight: 600;
}

.navbar-cta {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-family: var(--ir-font-sans);
  font-size: 14px;
  font-weight: 700;
  color: #0e0e0e;
  background: var(--ir-accent);
  padding: 8px 18px;
  border-radius: 8px;
  text-decoration: none;
  letter-spacing: -0.01em;
  transition: background 0.2s, box-shadow 0.2s, gap 0.2s;
  box-shadow: 0 2px 12px rgba(232, 131, 42, 0.25);
}

.navbar-cta:hover {
  background: var(--ir-accent-dark);
  box-shadow: 0 2px 20px rgba(232, 131, 42, 0.4);
  gap: 9px;
}

.hamburger {
  display: none;
  flex-direction: column;
  justify-content: center;
  gap: 5px;
  width: 36px;
  height: 36px;
  background: none;
  border: none;
  cursor: pointer;
  padding: 6px;
}

.hamburger span {
  display: block;
  height: 1.5px;
  background: rgba(240, 237, 232, 0.7);
  border-radius: 2px;
  transition: transform 0.25s, opacity 0.25s, width 0.25s;
  transform-origin: center;
}

.hamburger.open span:nth-child(1) { transform: translateY(6.5px) rotate(45deg); }
.hamburger.open span:nth-child(2) { opacity: 0; transform: scaleX(0); }
.hamburger.open span:nth-child(3) { transform: translateY(-6.5px) rotate(-45deg); }

.mobile-menu {
  display: none;
  flex-direction: column;
  padding: 12px 20px 20px;
  background: rgba(9, 9, 9, 0.97);
  border-top: 1px solid var(--ir-border);
  max-height: 0;
  overflow: hidden;
  transition: max-height 0.3s ease;
}

.mobile-menu.open {
  max-height: 400px;
}

.mobile-link {
  font-family: var(--ir-font-sans);
  font-size: 16px;
  font-weight: 500;
  color: rgba(240, 237, 232, 0.65);
  text-decoration: none;
  padding: 12px 4px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  transition: color 0.2s;
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.mobile-link:last-of-type {
  border-bottom: none;
}

.mobile-link:hover {
  color: var(--ir-text);
}

.mobile-stars {
  color: var(--ir-accent);
  font-size: 13px;
}

.mobile-divider {
  height: 1px;
  background: rgba(232, 131, 42, 0.15);
  margin: 4px 0;
}

.mobile-cta {
  display: block;
  margin-top: 16px;
  font-family: var(--ir-font-sans);
  font-size: 15px;
  font-weight: 700;
  color: #0e0e0e;
  background: var(--ir-accent);
  text-align: center;
  padding: 13px;
  border-radius: 8px;
  text-decoration: none;
}

@media (max-width: 900px) {
  .navbar-links {
    display: none;
  }
}

@media (max-width: 767px) {
  .github-pill,
  .icon-pill {
    display: none;
  }

  .navbar-cta {
    display: none;
  }

  .hamburger {
    display: flex;
  }

  .mobile-menu {
    display: flex;
  }

  .site-navbar.menu-open {
    background: rgba(9, 9, 9, 0.98);
  }
}
</style>
