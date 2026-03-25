<script setup>
import { withBase } from 'vitepress'
</script>

<template>
  <section class="code-section">
    <div class="code-inner">
      <div class="code-left">
        <h2 class="code-heading">Configure in seconds</h2>
        <p class="code-description">
          Define your proxy rules in a simple TOML file.
          No complex configuration. Just domains, addresses, and a mode.
        </p>
        <a :href="withBase('/configuration/')" class="code-link">Read the docs &#8594;</a>
      </div>

      <div class="code-right">
        <div class="terminal">
          <div class="terminal-header">
            <div class="terminal-dots">
              <span class="dot dot-red"></span>
              <span class="dot dot-yellow"></span>
              <span class="dot dot-green"></span>
            </div>
            <span class="terminal-filename">config.toml</span>
            <div class="terminal-dots-spacer"></div>
          </div>
          <div class="terminal-body">
            <pre><code><span class="hl-comment"># infrarust.toml</span>
<span class="hl-key">bind</span>        <span class="hl-text">=</span> <span class="hl-string">"0.0.0.0:25565"</span>
<span class="hl-key">servers_dir</span> <span class="hl-text">=</span> <span class="hl-string">"./servers"</span>

<span class="hl-comment"># servers/survival.toml</span>
<span class="hl-key">domains</span>    <span class="hl-text">=</span> <span class="hl-text">[</span><span class="hl-string">"survival.example.com"</span><span class="hl-text">]</span>
<span class="hl-key">addresses</span>  <span class="hl-text">=</span> <span class="hl-text">[</span><span class="hl-string">"10.0.1.10:25565"</span><span class="hl-text">]</span>
<span class="hl-key">proxy_mode</span> <span class="hl-text">=</span> <span class="hl-string">"passthrough"</span></code></pre>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.code-section {
  background: var(--ir-bg-dark);
  padding: 96px 24px;
}

.code-inner {
  max-width: 1200px;
  margin: 0 auto;
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 64px;
  align-items: center;
}

.code-heading {
  font-family: var(--ir-font-sans);
  font-weight: 700;
  font-size: 36px;
  color: var(--ir-text);
  letter-spacing: -0.02em;
  margin: 0 0 16px;
  line-height: 1.2;
}

.code-description {
  font-family: var(--ir-font-sans);
  font-size: 16px;
  line-height: 1.7;
  color: var(--ir-text-muted);
  margin: 0 0 24px;
  max-width: 440px;
}

.code-link {
  font-family: var(--ir-font-sans);
  font-size: 15px;
  font-weight: 600;
  color: var(--ir-accent);
  text-decoration: none;
  transition: color 0.2s;
}

.code-link:hover {
  color: var(--ir-accent-dark);
}

.terminal {
  border-radius: 12px;
  overflow: hidden;
  border: 1px solid var(--ir-border);
}

.terminal-header {
  background: var(--ir-surface);
  padding: 12px 16px;
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.terminal-dots {
  display: flex;
  gap: 6px;
}

.dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
}

.dot-red { background: #FF5F57; }
.dot-yellow { background: #FEBC2E; }
.dot-green { background: #28C840; }

.terminal-filename {
  font-family: var(--ir-font-mono);
  font-size: 13px;
  color: rgba(240, 237, 232, 0.5);
}

.terminal-dots-spacer {
  width: 44px;
}

.terminal-body {
  background: #0D0D0D;
  padding: 24px;
}

.terminal-body pre {
  margin: 0;
  font-family: var(--ir-font-mono);
  font-size: 14px;
  line-height: 1.7;
  overflow-x: auto;
}

.terminal-body code {
  font-family: inherit;
}

.hl-key { color: #9CDCFE; }
.hl-string { color: #CE9178; }
.hl-text { color: var(--ir-text); }
.hl-value { color: var(--ir-text); }
.hl-comment { color: #6A9955; }
.hl-section { color: #C586C0; }

@media (max-width: 1023px) {
  .code-inner {
    grid-template-columns: 1fr;
    gap: 40px;
  }

  .code-heading {
    font-size: 28px;
  }
}
</style>
