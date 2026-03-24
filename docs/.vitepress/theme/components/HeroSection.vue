<script setup>
import { withBase } from 'vitepress'
import HeroParticles from './HeroParticles.vue'
import { useGitHubStars } from '../composables/useGitHubStars'
import { GITHUB_REPO, GITHUB_URL } from '../constants'
import IconLogo from '../icons/IconLogo.vue'

const stars = useGitHubStars(GITHUB_REPO)
</script>

<template>
  <section class="hero">
    <ClientOnly>
      <HeroParticles />
    </ClientOnly>

    <div class="hero-content">
      <span class="hero-badge">Open Source &middot; Written in Rust</span>

      <h1 class="hero-title">
        One port.<br />
        Infinite servers.
      </h1>

      <p class="hero-subtitle">
        A blazing-fast Minecraft reverse proxy. Route players by domain,
        with zero latency overhead.
      </p>

      <div class="hero-cta">
        <a :href="withBase('/guide/')" class="btn-primary">Get Started &#8594;</a>
        <a
          :href="GITHUB_URL"
          target="_blank"
          rel="noopener"
          class="btn-secondary"
        >
          View on GitHub &#9733; {{ stars ?? '...' }}
        </a>
      </div>

      <div class="hero-logo" aria-hidden="true">
        <IconLogo :size="260" />
      </div>
    </div>
  </section>
</template>

<style scoped>
.hero {
  position: relative;
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--ir-bg-dark);
  overflow: hidden;
}

.hero-content {
  position: relative;
  z-index: 1;
  text-align: center;
  padding: 120px 24px 80px;
  max-width: 720px;
}

.hero-badge {
  display: inline-block;
  font-family: var(--ir-font-sans);
  font-size: 13px;
  font-weight: 500;
  color: var(--ir-accent);
  border: 1px solid rgba(232, 131, 42, 0.4);
  border-radius: 999px;
  padding: 6px 14px;
  margin-bottom: 28px;
}

.hero-title {
  font-family: var(--ir-font-sans);
  font-weight: 700;
  font-size: 72px;
  color: var(--ir-text);
  letter-spacing: -0.03em;
  line-height: 1.05;
  margin: 0 0 20px;
}

.hero-subtitle {
  font-family: var(--ir-font-sans);
  font-size: 18px;
  line-height: 1.6;
  color: rgba(240, 237, 232, 0.65);
  max-width: 520px;
  margin: 0 auto 32px;
}

.hero-cta {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  flex-wrap: wrap;
}

.btn-primary {
  font-family: var(--ir-font-sans);
  font-size: 15px;
  font-weight: 700;
  color: #111111;
  background: var(--ir-accent);
  padding: 14px 28px;
  border-radius: 8px;
  text-decoration: none;
  transition: background 0.2s;
}

.btn-primary:hover {
  background: var(--ir-accent-dark);
}

.btn-secondary {
  font-family: var(--ir-font-sans);
  font-size: 15px;
  font-weight: 600;
  color: var(--ir-text);
  background: transparent;
  border: 1.5px solid rgba(240, 237, 232, 0.25);
  padding: 14px 28px;
  border-radius: 8px;
  text-decoration: none;
  transition: border-color 0.2s;
}

.btn-secondary:hover {
  border-color: rgba(240, 237, 232, 0.6);
}

.hero-logo {
  margin-top: 48px;
  filter: drop-shadow(0 0 40px rgba(232, 131, 42, 0.3));
}

.hero-logo svg {
  display: block;
  margin: 0 auto;
}

@media (max-width: 767px) {
  .hero-title {
    font-size: 42px;
  }

  .hero-subtitle {
    font-size: 16px;
  }

  .hero-content {
    padding: 100px 20px 60px;
  }

  .hero-logo svg {
    width: 180px;
    height: 180px;
  }
}
</style>
