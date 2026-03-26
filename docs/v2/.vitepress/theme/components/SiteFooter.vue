<script setup>
import { withBase } from 'vitepress'
import { GITHUB_URL, DISCORD_URL } from '../constants'
import IconShield from '../icons/IconShield.vue'

const year = new Date().getFullYear()
</script>

<template>
  <footer class="site-footer">
    <div class="footer-inner">
      <div class="footer-left">
        <span class="footer-copy">&copy; {{ year }} Infrarust Project</span>
        <a :href="withBase('/license')" class="footer-license">
          <IconShield :size="11" />
          AGPL-3.0 + Plugin Exception
        </a>
        <a :href="withBase('/thank-you-open-source')" class="footer-license footer-oss">
          &#9829;
          Open Source
        </a>
        <a href="https://ko-fi.com/C1C41WOEBB" target="_blank" rel="noopener" class="footer-license footer-kofi">
          <img src="/images/kofi_symbol.png" alt="Ko-fi" class="kofi-icon" />
          Buy me a coffee
        </a>
      </div>
      <div class="footer-links">
        <a :href="GITHUB_URL" target="_blank" rel="noopener">GitHub</a>
        <span class="footer-sep">&middot;</span>
        <a :href="DISCORD_URL" target="_blank" rel="noopener">Discord</a>
      </div>
    </div>
  </footer>
</template>

<style scoped>
.site-footer {
  background: var(--ir-bg-deep);
  border-top: 1px solid var(--ir-border);
  padding: 40px 24px;
}

.footer-inner {
  max-width: 1200px;
  margin: 0 auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.footer-left {
  display: flex;
  align-items: center;
  gap: 14px;
  flex-wrap: wrap;
}

.footer-copy {
  font-family: var(--ir-font-sans);
  font-size: 14px;
  color: rgba(240, 237, 232, 0.4);
}

.footer-license {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-family: var(--ir-font-mono);
  font-size: 11px;
  font-weight: 500;
  color: rgba(232, 131, 42, 0.55);
  text-decoration: none;
  border: 1px solid rgba(232, 131, 42, 0.15);
  padding: 3px 9px;
  border-radius: 999px;
  letter-spacing: 0.02em;
  transition: color 0.2s, border-color 0.2s;
}

.footer-license:hover {
  color: var(--ir-accent);
  border-color: rgba(232, 131, 42, 0.4);
}

.footer-oss {
  color: rgba(93, 175, 80, 0.55);
  border-color: rgba(93, 175, 80, 0.15);
}

.footer-oss:hover {
  color: var(--ir-green);
  border-color: rgba(93, 175, 80, 0.4);
}

.footer-kofi {
  color: rgba(244, 93, 34, 0.55);
  border-color: rgba(244, 93, 34, 0.15);
}

.footer-kofi:hover {
  color: #f45d22;
  border-color: rgba(244, 93, 34, 0.4);
}

.kofi-icon {
  height: 13px;
  width: auto;
}

.footer-links {
  display: flex;
  align-items: center;
  gap: 8px;
}

.footer-links a {
  font-family: var(--ir-font-sans);
  font-size: 14px;
  color: rgba(240, 237, 232, 0.4);
  text-decoration: none;
  transition: color 0.2s;
}

.footer-links a:hover {
  color: var(--ir-text);
}

.footer-sep {
  color: rgba(240, 237, 232, 0.2);
}

@media (max-width: 767px) {
  .footer-inner {
    flex-direction: column;
    gap: 12px;
    text-align: center;
  }
}
</style>
