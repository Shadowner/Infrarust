<script setup>
import SiteNavbar from './SiteNavbar.vue'
import SiteFooter from './SiteFooter.vue'
import { GITHUB_URL, DISCORD_URL } from '../constants'
import IconShield from '../icons/IconShield.vue'
import IconDocument from '../icons/IconDocument.vue'
import IconPlugin from '../icons/IconPlugin.vue'
import IconCheck from '../icons/IconCheck.vue'
import IconX from '../icons/IconX.vue'
import IconAlertCircle from '../icons/IconAlertCircle.vue'
import IconBox from '../icons/IconBox.vue'

const canItems = [
  'Run it for any Minecraft network, commercial or not.',
  'Study and read the full source code.',
  'Modify it. Patch bugs, add features, change behaviour.',
  'Distribute copies to others.',
  'Charge money for a service or product built on top of it.',
  'Build closed-source, paid plugins and sell them freely.',
]

const mustItems = [
  'Keep the AGPL-3.0 license on any version of the core you distribute.',
  'Publish the full source of your core modifications when you distribute them.',
  'Make modified core source available to network users. If you run a modified Infrarust as a service, users can request your source.',
  'Preserve all existing copyright notices in source files.',
]

const cannotItems = [
  'Re-license the core proxy under a closed-source or proprietary license.',
  'Resell the compiled proxy binary as a standalone product.',
  'Incorporate core Infrarust code (outside infrarust-api) into a project with a more restrictive license.',
]

const scenarios = [
  {
    title: 'Running Infrarust for your Minecraft network',
    status: 'free',
    label: 'No obligations',
    text: 'Use it freely. No source-sharing required as long as you run the official release unmodified.',
  },
  {
    title: 'Selling a closed-source plugin',
    status: 'free',
    label: 'Fully allowed',
    text: 'Plugins built exclusively against the infrarust-api crate can be proprietary, closed-source, and sold for any price. No AGPL obligations apply to your plugin code.',
  },
  {
    title: 'Running a modified core as a service',
    status: 'conditional',
    label: 'Source required on request',
    text: 'If you modify Infrarust\'s core and players connect to it, you must make your modified source available to them on request. A public GitHub fork is sufficient.',
  },
  {
    title: 'Distributing Infrarust in a product',
    status: 'conditional',
    label: 'Must ship source',
    text: 'Include the source code (or a written offer to provide it) and keep the AGPL-3.0 license intact on the core.',
  },
  {
    title: 'Selling a plugin loaded via WASM or a Plugin Loader',
    status: 'free',
    label: 'Fully allowed',
    text: 'Plugins loaded by an officially designated Plugin Loader (e.g. a future WasmPluginLoader) are covered by the exception. You can keep them closed-source and sell them, even if they don\'t directly depend on infrarust-api.',
  },
  {
    title: 'Building a new Plugin Loader',
    status: 'free',
    label: 'Fully allowed',
    text: 'Building a new Plugin Loader is covered by the infrarust-api exception. License it however you want.',
  },
]
</script>

<template>
  <div class="license-page">
    <SiteNavbar />

    <header class="license-hero">
      <div class="hero-glow" aria-hidden="true" />
      <div class="hero-inner">
        <div class="license-chip">
          <IconShield :size="14" />
          AGPL-3.0 + Plugin Exception
        </div>

        <h1 class="hero-title">
          Open core.<br/>
          <span class="hero-accent">Free plugins.</span>
        </h1>

        <p class="hero-sub">
          Infrarust's core is AGPL-3.0. Modify it, and those changes stay open.
          Plugins built against the public API can be closed-source, commercial, or proprietary.
          No restrictions on your plugin code.
        </p>

        <div class="hero-links">
          <a :href="`${GITHUB_URL}/blob/main/LICENSE`"
             target="_blank" rel="noopener" class="hero-btn">
            <IconDocument :size="15" />
            Full license text
          </a>
          <a :href="DISCORD_URL" target="_blank" rel="noopener" class="hero-link-plain">
            Questions? Ask on Discord
          </a>
        </div>
      </div>
    </header>

    <section class="plugin-section">
      <div class="plugin-inner">
        <div class="plugin-card">
          <div class="plugin-left">
            <div class="plugin-icon" aria-hidden="true">
              <IconPlugin :size="26" />
            </div>
            <div>
              <h2 class="plugin-title">Plugin Exception</h2>
              <p class="plugin-sub">Built into the LICENSE file</p>
            </div>
          </div>
          <div class="plugin-body">
            <p class="plugin-text">
              Plugins that interact with Infrarust exclusively through the
              <code class="inline-code">infrarust-api</code> crate are
              <strong>not considered covered works</strong> under AGPL-3.0.
              You can license your plugin however you want: proprietary, closed-source,
              commercial, subscription-based, or any open-source license you prefer.
              No AGPL obligations apply to your plugin code.
            </p>
            <div class="plugin-rules">
              <div class="plugin-rule plugin-rule--yes">
                <IconCheck :size="14" />
                Sell your plugin for any price
              </div>
              <div class="plugin-rule plugin-rule--yes">
                <IconCheck :size="14" />
                Keep your source code private
              </div>
              <div class="plugin-rule plugin-rule--yes">
                <IconCheck :size="14" />
                Use any license you choose
              </div>
              <div class="plugin-rule plugin-rule--yes">
                <IconCheck :size="14" />
                Build a subscription-based service
              </div>
              <div class="plugin-rule plugin-rule--no">
                <IconX :size="14" />
                Only via <code class="inline-code">infrarust-api</code> or an official Plugin Loader ABI — not core internals
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>

    <section class="rules-section">
      <div class="rules-inner">

        <div class="rules-col rules-col--can">
          <div class="col-header">
            <div class="col-icon col-icon--can">
              <IconCheck :size="18" />
            </div>
            <h2 class="col-title">You can</h2>
          </div>
          <ul class="rule-list">
            <li v-for="item in canItems" :key="item" class="rule-item">
              <span class="rule-dot rule-dot--can" aria-hidden="true"/>
              {{ item }}
            </li>
          </ul>
        </div>

        <div class="rules-col rules-col--must">
          <div class="col-header">
            <div class="col-icon col-icon--must">
              <IconAlertCircle :size="18" />
            </div>
            <h2 class="col-title">You must <span class="col-subtitle">(core only)</span></h2>
          </div>
          <ul class="rule-list">
            <li v-for="item in mustItems" :key="item" class="rule-item">
              <span class="rule-dot rule-dot--must" aria-hidden="true"/>
              {{ item }}
            </li>
          </ul>
        </div>

        <div class="rules-col rules-col--cannot">
          <div class="col-header">
            <div class="col-icon col-icon--cannot">
              <IconX :size="18" />
            </div>
            <h2 class="col-title">You cannot</h2>
          </div>
          <ul class="rule-list">
            <li v-for="item in cannotItems" :key="item" class="rule-item">
              <span class="rule-dot rule-dot--cannot" aria-hidden="true"/>
              {{ item }}
            </li>
          </ul>
        </div>

      </div>
    </section>

    <section class="insight-section">
      <div class="insight-inner">
        <div class="insight-card">
          <div class="insight-icon" aria-hidden="true">
            <IconBox :size="22" />
          </div>
          <div>
            <h3 class="insight-title">The AGPL network clause</h3>
            <p class="insight-text">
              Regular GPL requires sharing source only when you <em>distribute</em> software.
              AGPL goes further: running a <em>modified core</em> as a network service also counts.
              If your players connect to a modified Infrarust, they can request your source.
              Running the official release (or a custom plugin on top of it) carries no such obligation.
            </p>
          </div>
        </div>
      </div>
    </section>

    <section class="scenarios-section">
      <div class="scenarios-inner">
        <h2 class="scenarios-heading">Common scenarios</h2>

        <div class="scenarios-grid">
          <div
            v-for="s in scenarios"
            :key="s.title"
            class="scenario-card"
            :class="`scenario-card--${s.status}`"
          >
            <div class="scenario-top">
              <span class="scenario-badge" :class="`scenario-badge--${s.status}`">
                {{ s.label }}
              </span>
            </div>
            <h3 class="scenario-title">{{ s.title }}</h3>
            <p class="scenario-text">{{ s.text }}</p>
          </div>
        </div>
      </div>
    </section>

    <SiteFooter />
  </div>
</template>

<style scoped>
.license-page {
  font-family: var(--ir-font-sans);
  color: var(--ir-text);
  background: var(--ir-bg);
  min-height: 100vh;
  overflow-x: hidden;
}

.license-hero {
  position: relative;
  background: var(--ir-bg-dark);
  padding: 140px 24px 100px;
  text-align: center;
  overflow: hidden;
  border-bottom: 1px solid var(--ir-border);
}

.hero-glow {
  position: absolute;
  top: -60px;
  left: 50%;
  transform: translateX(-50%);
  width: 600px;
  height: 400px;
  background: radial-gradient(ellipse, rgba(232, 131, 42, 0.1) 0%, transparent 65%);
  pointer-events: none;
}

.hero-inner {
  position: relative;
  max-width: 760px;
  margin: 0 auto;
}

.license-chip {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-family: var(--ir-font-mono);
  font-size: 12px;
  font-weight: 500;
  letter-spacing: 0.04em;
  color: var(--ir-accent);
  background: rgba(232, 131, 42, 0.08);
  border: 1px solid rgba(232, 131, 42, 0.22);
  padding: 7px 16px;
  border-radius: 999px;
  margin-bottom: 32px;
}

.hero-title {
  font-family: var(--ir-font-sans);
  font-weight: 700;
  font-size: 64px;
  line-height: 1.05;
  letter-spacing: -0.03em;
  color: var(--ir-text);
  margin: 0 0 20px;
}

.hero-accent { color: var(--ir-accent); }

.hero-sub {
  font-size: 17px;
  line-height: 1.65;
  color: var(--ir-text-muted);
  margin: 0 auto 32px;
  max-width: 560px;
}

.hero-links {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 24px;
  flex-wrap: wrap;
}

.hero-btn {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-family: var(--ir-font-sans);
  font-size: 14px;
  font-weight: 600;
  color: #111;
  background: var(--ir-accent);
  padding: 10px 22px;
  border-radius: 7px;
  text-decoration: none;
  transition: background 0.2s;
}

.hero-btn:hover { background: var(--ir-accent-dark); }

.hero-link-plain {
  font-size: 14px;
  color: var(--ir-text-muted);
  text-decoration: none;
  border-bottom: 1px solid rgba(240, 237, 232, 0.2);
  transition: color 0.2s, border-color 0.2s;
}

.hero-link-plain:hover {
  color: var(--ir-text);
  border-color: rgba(240, 237, 232, 0.5);
}

.plugin-section {
  background: var(--ir-bg);
  padding: 72px 24px 0;
}

.plugin-inner {
  max-width: 1100px;
  margin: 0 auto;
}

.plugin-card {
  background: linear-gradient(135deg, rgba(232,131,42,0.05) 0%, rgba(232,131,42,0.02) 100%);
  border: 1px solid rgba(232, 131, 42, 0.25);
  border-radius: 16px;
  padding: 36px 40px;
  display: flex;
  flex-direction: column;
  gap: 24px;
}

.plugin-left {
  display: flex;
  align-items: center;
  gap: 16px;
}

.plugin-icon {
  width: 52px;
  height: 52px;
  background: rgba(232, 131, 42, 0.08);
  border: 1px solid rgba(232, 131, 42, 0.2);
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: var(--ir-accent);
}

.plugin-title {
  font-family: var(--ir-font-sans);
  font-weight: 700;
  font-size: 22px;
  color: var(--ir-text);
  margin: 0 0 2px;
}

.plugin-sub {
  font-family: var(--ir-font-mono);
  font-size: 11px;
  color: rgba(232, 131, 42, 0.6);
  margin: 0;
  letter-spacing: 0.04em;
}

.plugin-text {
  font-size: 15px;
  line-height: 1.7;
  color: rgba(240, 237, 232, 0.75);
  margin: 0 0 20px;
}

.plugin-text strong {
  color: var(--ir-text);
  font-weight: 700;
}

.plugin-rules {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}

.plugin-rule {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  font-family: var(--ir-font-sans);
  font-size: 13px;
  font-weight: 500;
  padding: 6px 12px;
  border-radius: 6px;
}

.plugin-rule--yes {
  color: var(--ir-green);
  background: var(--ir-green-soft);
  border: 1px solid rgba(74, 124, 63, 0.2);
}

.plugin-rule--no {
  color: rgba(240, 237, 232, 0.45);
  background: rgba(255, 255, 255, 0.03);
  border: 1px solid rgba(255, 255, 255, 0.07);
}

.rules-section {
  padding: 72px 24px;
  background: var(--ir-bg);
}

.rules-inner {
  max-width: 1100px;
  margin: 0 auto;
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 20px;
}

.rules-col {
  background: var(--ir-surface);
  border: 1px solid var(--ir-border);
  border-radius: 16px;
  padding: 28px;
}

.rules-col--can    { border-top: 3px solid var(--ir-green-dark); }
.rules-col--must   { border-top: 3px solid var(--ir-accent); }
.rules-col--cannot { border-top: 3px solid var(--ir-red-dark); }

.col-header {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 24px;
}

.col-icon {
  width: 36px;
  height: 36px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.col-icon--can    { background: rgba(74,124,63,0.12);  color: var(--ir-green); border: 1px solid rgba(74,124,63,0.25); }
.col-icon--must   { background: rgba(232,131,42,0.1);  color: var(--ir-accent); border: 1px solid rgba(232,131,42,0.25); }
.col-icon--cannot { background: rgba(166,50,40,0.1);   color: var(--ir-red); border: 1px solid rgba(166,50,40,0.25); }

.col-title {
  font-weight: 700;
  font-size: 19px;
  color: var(--ir-text);
  margin: 0;
}

.col-subtitle {
  font-size: 13px;
  font-weight: 500;
  color: var(--ir-text-muted);
}

.rule-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.rule-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  font-size: 14px;
  line-height: 1.6;
  color: rgba(240, 237, 232, 0.75);
}

.rule-dot {
  display: block;
  width: 6px;
  height: 6px;
  border-radius: 1px;
  flex-shrink: 0;
  margin-top: 7px;
}

.rule-dot--can    { background: var(--ir-green); }
.rule-dot--must   { background: var(--ir-accent); }
.rule-dot--cannot { background: var(--ir-red); }

.insight-section {
  background: var(--ir-bg-dark);
  border-top: 1px solid var(--ir-border);
  padding: 48px 24px;
}

.insight-inner {
  max-width: 1100px;
  margin: 0 auto;
}

.insight-card {
  display: flex;
  align-items: flex-start;
  gap: 20px;
  background: rgba(232, 131, 42, 0.05);
  border: 1px solid rgba(232, 131, 42, 0.18);
  border-left: 3px solid var(--ir-accent);
  border-radius: 12px;
  padding: 28px 32px;
}

.insight-icon {
  width: 44px;
  height: 44px;
  background: rgba(232, 131, 42, 0.08);
  border-radius: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: var(--ir-accent);
}

.insight-title {
  font-weight: 700;
  font-size: 17px;
  color: var(--ir-text);
  margin: 0 0 8px;
}

.insight-text {
  font-size: 15px;
  line-height: 1.7;
  color: var(--ir-text-muted);
  margin: 0;
}

.insight-text em {
  color: var(--ir-text);
  font-style: normal;
  font-weight: 600;
}

.scenarios-section {
  background: var(--ir-bg);
  padding: 72px 24px;
}

.scenarios-inner {
  max-width: 1100px;
  margin: 0 auto;
}

.scenarios-heading {
  font-weight: 700;
  font-size: 32px;
  color: var(--ir-text);
  letter-spacing: -0.02em;
  margin: 0 0 36px;
}

.scenarios-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: 16px;
}

.scenario-card {
  background: var(--ir-surface);
  border: 1px solid var(--ir-border);
  border-radius: 12px;
  padding: 24px 26px;
  transition: border-color 0.2s, transform 0.2s;
}

.scenario-card:hover { transform: translateY(-2px); }
.scenario-card--free:hover        { border-color: rgba(74,124,63,0.35); }
.scenario-card--conditional:hover { border-color: rgba(232,131,42,0.35); }

.scenario-top { margin-bottom: 14px; }

.scenario-badge {
  display: inline-block;
  font-family: var(--ir-font-mono);
  font-size: 11px;
  font-weight: 500;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  padding: 4px 10px;
  border-radius: 4px;
}

.scenario-badge--free {
  color: var(--ir-green);
  background: var(--ir-green-soft);
  border: 1px solid rgba(74,124,63,0.2);
}

.scenario-badge--conditional {
  color: var(--ir-accent);
  background: rgba(232,131,42,0.08);
  border: 1px solid rgba(232,131,42,0.2);
}

.scenario-title {
  font-weight: 700;
  font-size: 16px;
  color: var(--ir-text);
  margin: 0 0 8px;
}

.scenario-text {
  font-size: 14px;
  line-height: 1.65;
  color: var(--ir-text-muted);
  margin: 0;
}

.inline-code {
  font-family: var(--ir-font-mono);
  font-size: 13px;
  color: var(--ir-accent);
  background: rgba(232, 131, 42, 0.08);
  padding: 1px 6px;
  border-radius: 4px;
}

@media (max-width: 900px) {
  .rules-inner { grid-template-columns: 1fr; }
  .hero-title  { font-size: 42px; }
}

@media (max-width: 767px) {
  .license-hero { padding: 120px 20px 72px; }
  .hero-title   { font-size: 34px; }
  .plugin-card  { padding: 24px 20px; }
  .insight-card { flex-direction: column; gap: 14px; }
  .plugin-rules { flex-direction: column; }
  .strip-inner  { flex-direction: column; align-items: flex-start; }
}
</style>
