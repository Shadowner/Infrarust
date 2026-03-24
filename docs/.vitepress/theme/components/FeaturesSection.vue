<template>
  <section class="features-section">
    <div class="features-inner">
      <span class="features-label">Features</span>
      <h2 class="features-heading">What Infrarust ships with</h2>

      <div class="features-grid">
        <div class="feature-card" v-for="feature in features" :key="feature.title">
          <div class="feature-icon-wrap">
            <component :is="iconMap[feature.icon]" :size="20" />
          </div>
          <div v-if="feature.badge" class="feature-badge">{{ feature.badge }}</div>
          <h3 class="feature-title">{{ feature.title }}</h3>
          <p class="feature-text">{{ feature.text }}</p>
        </div>
      </div>
    </div>
  </section>
</template>

<script setup>
import IconRouting from '../icons/IconRouting.vue'
import IconModes from '../icons/IconModes.vue'
import IconTelemetry from '../icons/IconTelemetry.vue'
import IconPlugin from '../icons/IconPlugin.vue'

const iconMap = {
  routing: IconRouting,
  modes: IconModes,
  telemetry: IconTelemetry,
  plugin: IconPlugin,
}

const features = [
  {
    icon: 'routing',
    title: 'Domain-based routing',
    text: 'Route players to different backends by domain or subdomain. Exact matches resolve in O(1), wildcards compile at startup.',
  },
  {
    icon: 'modes',
    title: '4 proxy modes',
    text: 'Choose between passthrough, client_only, server_only, or offline mode depending on your infrastructure needs.',
  },
  {
    icon: 'telemetry',
    title: 'OpenTelemetry native',
    text: 'Built-in metrics, distributed traces, and structured logs. Drop into your existing monitoring stack.',
  },
  {
    icon: 'plugin',
    title: 'Sell your plugins',
    text: 'Plugins built against infrarust-api can be closed-source and sold at any price. The Plugin Exception in the license removes all AGPL obligations from your plugin code.',
    badge: 'Plugin Exception',
  },
]
</script>

<style scoped>
.features-section {
  background: var(--ir-bg);
  padding: 96px 24px;
}

.features-inner {
  max-width: 1200px;
  margin: 0 auto;
}

.features-label {
  display: block;
  font-family: var(--ir-font-sans);
  font-size: 13px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.1em;
  color: var(--ir-accent);
  margin-bottom: 12px;
}

.features-heading {
  font-family: var(--ir-font-sans);
  font-weight: 700;
  font-size: 40px;
  color: var(--ir-text);
  letter-spacing: -0.02em;
  margin: 0 0 48px;
  line-height: 1.15;
}

.features-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 24px;
}

.feature-card {
  background: var(--ir-surface);
  border: 1px solid var(--ir-border);
  border-radius: 12px;
  padding: 28px;
  transition: border-color 0.2s, transform 0.2s;
}

.feature-card:hover {
  border-color: rgba(232, 131, 42, 0.3);
  transform: translateY(-2px);
}

.feature-icon-wrap {
  width: 40px;
  height: 40px;
  background: rgba(232, 131, 42, 0.1);
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  margin-bottom: 16px;
  color: var(--ir-accent);
}

.feature-title {
  font-family: var(--ir-font-sans);
  font-weight: 700;
  font-size: 18px;
  color: var(--ir-text);
  margin: 0 0 10px;
}

.feature-text {
  font-family: var(--ir-font-sans);
  font-size: 15px;
  line-height: 1.6;
  color: var(--ir-text-muted);
  margin: 0;
}

.feature-badge {
  display: inline-block;
  font-family: var(--ir-font-mono);
  font-size: 10px;
  font-weight: 500;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--ir-green);
  background: var(--ir-green-soft);
  border: 1px solid var(--ir-green-border);
  padding: 3px 8px;
  border-radius: 4px;
  margin-bottom: 10px;
}

@media (max-width: 1200px) {
  .features-grid {
    grid-template-columns: repeat(2, 1fr);
  }
}

@media (max-width: 600px) {
  .features-grid {
    grid-template-columns: 1fr;
  }

  .features-heading {
    font-size: 30px;
  }
}
</style>
