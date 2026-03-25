<script setup>
import { withBase } from 'vitepress'
import { useGitHubStars } from '../composables/useGitHubStars'
import { GITHUB_REPO, GITHUB_URL, DISCORD_URL } from '../constants'
import IconGitHub from '../icons/IconGitHub.vue'
import IconDiscord from '../icons/IconDiscord.vue'
import IconShield from '../icons/IconShield.vue'
import IconCheck from '../icons/IconCheck.vue'

const stars = useGitHubStars(GITHUB_REPO)

const floatingBlocks = [
  { color: 'var(--ir-mc-grass)',   size: 10, top: '12%',  left: '6%',  delay: '0s',    duration: '18s' },
  { color: 'var(--ir-mc-dirt)',    size:  7, top: '70%',  left: '4%',  delay: '3s',    duration: '22s' },
  { color: 'var(--ir-mc-stone)',   size: 12, top: '35%',  left: '91%', delay: '1.5s',  duration: '20s' },
  { color: 'var(--ir-mc-diamond)', size:  6, top: '80%',  left: '88%', delay: '5s',    duration: '16s' },
  { color: 'var(--ir-mc-cactus)',  size:  8, top: '55%',  left: '94%', delay: '2s',    duration: '24s' },
  { color: 'var(--ir-accent)',     size:  9, top: '18%',  left: '88%', delay: '0.5s',  duration: '19s' },
  { color: 'var(--ir-mc-dirt)',    size: 11, top: '88%',  left: '14%', delay: '4s',    duration: '21s' },
  { color: 'var(--ir-mc-grass)',   size:  7, top: '42%',  left: '2%',  delay: '6s',    duration: '17s' },
  { color: 'var(--ir-mc-stone)',   size:  8, top: '25%',  left: '50%', delay: '8s',    duration: '26s' },
  { color: 'var(--ir-mc-diamond)', size:  6, top: '62%',  left: '75%', delay: '2.5s',  duration: '23s' },
  { color: 'var(--ir-accent)',     size: 10, top: '8%',   left: '30%', delay: '7s',    duration: '20s' },
  { color: 'var(--ir-mc-cactus)',  size:  7, top: '78%',  left: '60%', delay: '1s',    duration: '25s' },
]
</script>

<template>
  <section class="cta-section">
    <div class="cta-banner">

      <div class="banner-texture" aria-hidden="true" />

      <div
        v-for="(b, i) in floatingBlocks"
        :key="i"
        class="pixel-block"
        aria-hidden="true"
        :style="{
          width:  b.size + 'px',
          height: b.size + 'px',
          background: b.color,
          top: b.top,
          left: b.left,
          animationDelay: b.delay,
          animationDuration: b.duration,
        }"
      />

      <div class="cta-content">
        <a :href="withBase('/thank-you-open-source')" class="cta-badge">
          <img width="20" height="20" src="/images/creeper.svg" alt="Creeper Icon" />
          <span>Community project</span>
        </a>

        <a :href="withBase('/thank-you-open-source')" class="cta-heading-link">
          <h2 class="cta-heading">Open source &amp;<br/>free forever.</h2>
        </a>

        <p class="cta-text">
          Built for the Minecraft community. Core is AGPL-3.0.
          Plugins can be closed-source, proprietary, and sold freely.
          Contributions welcome.
        </p>

        <div class="cta-license-pills">
          <span class="license-pill license-pill--agpl">
            <IconShield :size="12" />
            Core: AGPL-3.0
          </span>
          <span class="license-pill license-pill--plugin">
            <IconCheck :size="12" />
            Plugins: any license
          </span>
          <a :href="withBase('/license')" class="license-pill license-pill--link">
            Read the license &rarr;
          </a>
        </div>

        <div class="cta-buttons">
          <a :href="GITHUB_URL" target="_blank" rel="noopener" class="cta-github">
            <IconGitHub :size="18" />
            Star on GitHub
          </a>

          <div class="cta-stars-wrap">
            <span class="stars-icon">&#9733;</span>
            <span class="stars-count">{{ stars ?? '...' }}</span>
            <span class="stars-label">stars</span>
          </div>

          <a :href="DISCORD_URL" target="_blank" rel="noopener" class="cta-discord">
            <IconDiscord :size="18" />
            Join Discord
          </a>
        </div>
      </div>

    </div>
  </section>
</template>

<style scoped>
.cta-section {
  padding: 96px 24px 96px;
  background: var(--ir-bg-dark);
}

.cta-banner {
  position: relative;
  max-width: 1100px;
  margin: 0 auto;
  border-radius: 20px;
  padding: 80px 48px;
  text-align: center;
  overflow: hidden;

  background:
    linear-gradient(160deg, #0F0900 0%, #0C0C0C 55%, #050D07 100%);

  box-shadow:
    0 0 0 1px rgba(232, 131, 42, 0.25),
    0 0 0 3px rgba(232, 131, 42, 0.06),
    0 32px 80px rgba(0, 0, 0, 0.6),
    inset 0 1px 0 rgba(255, 255, 255, 0.04);
}

.banner-texture {
  position: absolute;
  inset: 0;
  background-image:
    linear-gradient(rgba(255,255,255,0.015) 1px, transparent 1px),
    linear-gradient(90deg, rgba(255,255,255,0.015) 1px, transparent 1px);
  background-size: 16px 16px;
  pointer-events: none;
  border-radius: inherit;
}

.cta-banner::before {
  content: '';
  position: absolute;
  top: -40%;
  left: 50%;
  transform: translateX(-50%);
  width: 60%;
  height: 70%;
  background: radial-gradient(ellipse, rgba(232, 131, 42, 0.12) 0%, transparent 70%);
  pointer-events: none;
}

.cta-banner::after {
  content: '';
  position: absolute;
  bottom: -20%;
  left: -10%;
  width: 40%;
  height: 60%;
  background: radial-gradient(ellipse, rgba(74, 124, 63, 0.1) 0%, transparent 70%);
  pointer-events: none;
}

.pixel-block {
  position: absolute;
  image-rendering: pixelated;
  opacity: 0.35;
  animation: floatBlock linear infinite;
  border-radius: 1px;
  pointer-events: none;
}

@keyframes floatBlock {
  0%   { transform: translateY(0px)   rotate(0deg);   opacity: 0;    }
  10%  { opacity: 0.35; }
  50%  { transform: translateY(-40px) rotate(180deg); opacity: 0.35; }
  90%  { opacity: 0.35; }
  100% { transform: translateY(-80px) rotate(360deg); opacity: 0;    }
}

.cta-license-pills {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 32px;
}

.license-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-family: var(--ir-font-mono);
  font-size: 12px;
  font-weight: 500;
  padding: 5px 12px;
  border-radius: 999px;
  letter-spacing: 0.02em;
}

.license-pill--agpl {
  color: var(--ir-accent);
  background: rgba(232, 131, 42, 0.08);
  border: 1px solid rgba(232, 131, 42, 0.22);
}

.license-pill--plugin {
  color: var(--ir-green);
  background: var(--ir-green-soft);
  border: 1px solid var(--ir-green-border);
}

.license-pill--link {
  color: rgba(240, 237, 232, 0.5);
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.1);
  text-decoration: none;
  transition: color 0.2s, border-color 0.2s;
}

.license-pill--link:hover {
  color: var(--ir-text);
  border-color: rgba(255, 255, 255, 0.25);
}

.corner-block {
  position: absolute;
  opacity: 0.22;
  image-rendering: pixelated;
  pointer-events: none;
}

.corner-tl {
  top: 0;
  left: 0;
  border-radius: 20px 0 0 0;
}

.corner-tr {
  top: 0;
  right: 0;
  border-radius: 0 20px 0 0;
  transform: scaleX(-1);
}

.diamond-deco {
  position: absolute;
  top: 20px;
  right: 100px;
  display: flex;
  align-items: center;
  gap: 7px;
  opacity: 0.5;
  pointer-events: none;
}

.diamond-deco span {
  font-family: var(--ir-font-mono);
  font-size: 10px;
  color: var(--ir-mc-diamond);
  letter-spacing: 0.04em;
}

.cta-content {
  position: relative;
  z-index: 1;
}

.cta-badge {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-family: var(--ir-font-sans);
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.1em;
  color: var(--ir-green-dark);
  background: var(--ir-green-soft);
  border: 1px solid rgba(74, 124, 63, 0.25);
  padding: 5px 12px;
  border-radius: 6px;
  margin-bottom: 24px;
  text-decoration: none;
  transition: background 0.2s, border-color 0.2s;
}

.cta-badge:hover {
  background: rgba(74, 124, 63, 0.18);
  border-color: rgba(74, 124, 63, 0.4);
}

.cta-heading-link {
  text-decoration: none;
  display: block;
}

.cta-heading-link:hover .cta-heading {
  color: var(--ir-accent);
}

.cta-heading {
  font-family: var(--ir-font-sans);
  font-weight: 700;
  font-size: 52px;
  line-height: 1.05;
  letter-spacing: -0.03em;
  color: var(--ir-text);
  margin: 0 0 18px;
  transition: color 0.2s;
}

.cta-text {
  font-family: var(--ir-font-sans);
  font-size: 16px;
  line-height: 1.65;
  color: var(--ir-text-muted);
  margin: 0 auto 40px;
  max-width: 440px;
}

.cta-buttons {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  flex-wrap: wrap;
}

.cta-github {
  display: inline-flex;
  align-items: center;
  gap: 9px;
  font-family: var(--ir-font-sans);
  font-size: 15px;
  font-weight: 700;
  color: #0e0e0e;
  background: var(--ir-accent);
  padding: 13px 26px;
  border-radius: 4px;
  text-decoration: none;
  letter-spacing: -0.01em;
  box-shadow:
    0 4px 0 #7A3D10,
    0 5px 12px rgba(232, 131, 42, 0.3);
  transition: transform 0.1s, box-shadow 0.1s, background 0.15s;
}

.cta-github:hover {
  background: #F09040;
  transform: translateY(-2px);
  box-shadow:
    0 6px 0 #7A3D10,
    0 8px 20px rgba(232, 131, 42, 0.4);
}

.cta-github:active {
  transform: translateY(2px);
  box-shadow:
    0 2px 0 #7A3D10,
    0 2px 8px rgba(232, 131, 42, 0.2);
}

.cta-stars-wrap {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  background: rgba(232, 131, 42, 0.08);
  border: 1px solid rgba(232, 131, 42, 0.2);
  padding: 11px 16px;
  border-radius: 4px;
}

.stars-icon {
  color: var(--ir-accent);
  font-size: 15px;
}

.stars-count {
  font-family: var(--ir-font-mono);
  font-size: 15px;
  font-weight: 700;
  color: var(--ir-accent);
}

.stars-label {
  font-family: var(--ir-font-sans);
  font-size: 13px;
  color: rgba(232, 131, 42, 0.55);
}

.cta-discord {
  display: inline-flex;
  align-items: center;
  gap: 9px;
  font-family: var(--ir-font-sans);
  font-size: 15px;
  font-weight: 600;
  color: #c9cbff;
  background: rgba(88, 101, 242, 0.12);
  border: 1px solid rgba(88, 101, 242, 0.3);
  padding: 12px 24px;
  border-radius: 4px;
  text-decoration: none;
  box-shadow: 0 4px 0 rgba(55, 65, 180, 0.5);
  transition: transform 0.1s, box-shadow 0.1s, background 0.15s;
}

.cta-discord:hover {
  background: rgba(88, 101, 242, 0.2);
  transform: translateY(-2px);
  box-shadow: 0 6px 0 rgba(55, 65, 180, 0.5), 0 8px 20px rgba(88, 101, 242, 0.2);
}

.cta-discord:active {
  transform: translateY(2px);
  box-shadow: 0 2px 0 rgba(55, 65, 180, 0.5);
}

@media (max-width: 767px) {
  .cta-banner {
    padding: 56px 24px 48px;
  }

  .cta-heading {
    font-size: 34px;
  }

  .corner-block,
  .diamond-deco {
    display: none;
  }

  .cta-buttons {
    flex-direction: column;
    align-items: stretch;
  }

  .cta-github,
  .cta-discord {
    justify-content: center;
  }
}
</style>
