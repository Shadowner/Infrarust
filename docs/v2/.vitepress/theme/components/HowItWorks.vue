<script setup>
import { withBase } from 'vitepress'

const steps = [
  {
    number: '01',
    title: 'Install',
    text: 'Grab the binary or install directly with Cargo. Single executable, no runtime dependencies.',
    code: 'cargo install infrarust',
    icon: `<path d="M12 2L2 7l10 5 10-5-10-5z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/>`,
  },
  {
    number: '02',
    title: 'Configure',
    text: 'Drop a config.toml in your working directory. Define your bind address and point to your servers folder.',
    code: 'servers_dir = "./servers"',
    icon: `<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/>`,
  },
  {
    number: '03',
    title: 'Route',
    text: 'Start Infrarust. Players connect to one port and are routed to the right backend by domain name.',
    code: 'infrarust start',
    icon: `<polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>`,
  },
]
</script>

<template>
  <section class="how-section">
    <div class="how-inner">

      <div class="how-header">
        <span class="how-label">Setup</span>
        <h2 class="how-heading">Up and running in minutes</h2>
        <p class="how-sub">Three steps from a blank machine to a working Minecraft proxy.</p>
      </div>

      <div class="steps-track">
        <div class="track-line" aria-hidden="true" />

        <div
          v-for="(step, i) in steps"
          :key="step.title"
          class="step-card"
        >
          <div class="step-badge">
            <span class="step-num">{{ step.number }}</span>
          </div>

          <div class="step-icon">
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none"
                 stroke="#E8832A" stroke-width="1.75"
                 stroke-linecap="round" stroke-linejoin="round"
                 aria-hidden="true"
                 v-html="step.icon" />
          </div>

          <h3 class="step-title">{{ step.title }}</h3>
          <p class="step-text">{{ step.text }}</p>

          <div class="step-terminal">
            <span class="terminal-prompt">$</span>
            <code class="terminal-cmd">{{ step.code }}</code>
          </div>
        </div>
      </div>

      <div class="how-footer">
        <a :href="withBase('/guide/quick-start')" class="how-cta">
          Read the quick-start guide
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor"
               stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <line x1="5" y1="12" x2="19" y2="12"/>
            <polyline points="12 5 19 12 12 19"/>
          </svg>
        </a>
      </div>

    </div>
  </section>
</template>

<style scoped>
.how-section {
  background: var(--ir-bg);
  padding: 112px 24px;
  padding-bottom: 25px;
}

.how-inner {
  max-width: 1100px;
  margin: 0 auto;
}

.how-header {
  text-align: center;
  margin-bottom: 72px;
}

.how-label {
  display: inline-block;
  font-family: var(--ir-font-sans);
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.12em;
  color: var(--ir-accent);
  margin-bottom: 14px;
}

.how-heading {
  font-family: var(--ir-font-sans);
  font-weight: 700;
  font-size: 42px;
  color: var(--ir-text);
  letter-spacing: -0.03em;
  margin: 0 0 14px;
  line-height: 1.1;
}

.how-sub {
  font-family: var(--ir-font-sans);
  font-size: 16px;
  color: var(--ir-text-muted);
  margin: 0;
}

.steps-track {
  position: relative;
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 24px;
}

.track-line {
  position: absolute;
  top: 52px;
  left: calc(33.33% - 20px);
  right: calc(33.33% - 20px);
  height: 1px;
  background: repeating-linear-gradient(
    90deg,
    rgba(232, 131, 42, 0.25) 0px,
    rgba(232, 131, 42, 0.25) 6px,
    transparent 6px,
    transparent 14px
  );
  pointer-events: none;
}

.step-card {
  position: relative;
  background: var(--ir-surface);
  border: 1px solid var(--ir-border);
  border-radius: 16px;
  padding: 32px 28px 28px;
  display: flex;
  flex-direction: column;
  gap: 0;
  transition: border-color 0.25s, box-shadow 0.25s, transform 0.25s;
}

.step-card:hover {
  border-color: rgba(232, 131, 42, 0.35);
  box-shadow: 0 0 0 1px rgba(232, 131, 42, 0.1), 0 8px 32px rgba(0, 0, 0, 0.35);
  transform: translateY(-3px);
}

.step-badge {
  position: absolute;
  top: 20px;
  right: 20px;
}

.step-num {
  font-family: var(--ir-font-mono);
  font-size: 11px;
  font-weight: 500;
  color: rgba(232, 131, 42, 0.5);
  letter-spacing: 0.06em;
  background: rgba(232, 131, 42, 0.07);
  border: 1px solid rgba(232, 131, 42, 0.15);
  padding: 3px 8px;
  border-radius: 4px;
}

.step-icon {
  width: 48px;
  height: 48px;
  background: rgba(232, 131, 42, 0.07);
  border: 1px solid rgba(232, 131, 42, 0.15);
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  margin-bottom: 20px;
  flex-shrink: 0;
  transition: background 0.2s, border-color 0.2s;
}

.step-card:hover .step-icon {
  background: rgba(232, 131, 42, 0.12);
  border-color: rgba(232, 131, 42, 0.3);
}

.step-title {
  font-family: var(--ir-font-sans);
  font-weight: 700;
  font-size: 19px;
  color: var(--ir-text);
  margin: 0 0 10px;
  letter-spacing: -0.01em;
}

.step-text {
  font-family: var(--ir-font-sans);
  font-size: 14px;
  line-height: 1.65;
  color: var(--ir-text-muted);
  margin: 0 0 20px;
  flex-grow: 1;
}

.step-terminal {
  display: flex;
  align-items: center;
  gap: 10px;
  background: #0D0D0D;
  border: 1px solid rgba(255, 255, 255, 0.06);
  border-radius: 8px;
  padding: 10px 14px;
  margin-top: auto;
}

.terminal-prompt {
  font-family: var(--ir-font-mono);
  font-size: 13px;
  color: rgba(232, 131, 42, 0.5);
  user-select: none;
  flex-shrink: 0;
}

.terminal-cmd {
  font-family: var(--ir-font-mono);
  font-size: 13px;
  color: var(--ir-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.how-footer {
  display: flex;
  justify-content: center;
  margin-top: 48px;
}

.how-cta {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-family: var(--ir-font-sans);
  font-size: 14px;
  font-weight: 600;
  color: var(--ir-accent);
  text-decoration: none;
  transition: gap 0.2s, color 0.2s;
}

.how-cta:hover {
  color: var(--ir-text);
  gap: 12px;
}

@media (max-width: 900px) {
  .steps-track {
    grid-template-columns: 1fr;
    gap: 16px;
  }

  .track-line {
    display: none;
  }

  .how-heading {
    font-size: 32px;
  }
}
</style>
