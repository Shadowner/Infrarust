<script setup lang="ts">
import SiteNavbar from './SiteNavbar.vue'
import SiteFooter from './SiteFooter.vue'
import IconGitHub from '../icons/IconGitHub.vue'
import { useMultiGitHubStars, formatStars } from '../composables/useMultiGitHubStars'

const minecraftProjects = [
  {
    name: "Infrared",
    
    repo: "https://github.com/haveachin/infrared",
    desc : "The original Go Minecraft proxy that inspired me to build Infrarust in the first place. I learned a lot from reading through the code and issues. Even if Infrarust don't share the architecture anymore, the V1 codebase (inspired by it) was the spark that started it all.",
  },
  {
    name: 'Velocity',

    repo: 'https://github.com/PaperMC/Velocity',
    desc: 'The single biggest influence on Infrarust\'s plugin architecture. The ResultedEvent pattern, EventBus with priorities, the separation between api/ and proxy internals. I borrowed all of it. It also shaped how I think about a proxy\'s relationship with the Minecraft protocol.',
  },
  {
    name: 'LimboAPI',

    repo: 'https://github.com/Elytrium/LimboAPI',
    desc: 'Proved that a proxy can hold players in virtual worlds for authentication, queuing, or anti-bot checks. The key lesson was actually what not to do, building virtual worlds as a plugin that hacks into proxy internals is fragile. In V2, the Limbo is a first-class concept.',
  },
  {
    name: 'PicoLimbo',

    repo: 'https://github.com/Quozul/PicoLimbo',
    desc: 'A Rust limbo server covering 1.7.2 through 1.21.1 with only 44 packets across 49+ protocol versions. The biggest help was understanding the join and registry sequence from 1.20.2 onward. The config phase is poorly documented and version-sensitive.',
  },
  {
    name: 'Pumpkin',

    repo: 'https://github.com/Pumpkin-MC/Pumpkin',
    desc: 'Showed how to structure a Minecraft project in Rust. The crate layering, protocol organization by connection state, chunk encoding. The build.rs approach of generating Rust code from JSON assets was the answer to my registry data problem.',
  },
  {
    name: 'Valence',

    repo: 'https://github.com/valence-rs/valence',
    desc: 'The best packet serialization design in the Rust MC space. Encode/Decode traits writing to impl Write, the Bounded<T, MAX> type to cap allocations on decode. I wrote my own protocol crate, but the trait design came straight from here.',
  },
]

const networkingProjects = [
  {
    name: 'Pingora',

    repo: 'https://github.com/cloudflare/pingora',
    desc: 'Cloudflare\'s proxy framework gave me the connection lifecycle model. The ProxyHttp trait with ~32 optional callback methods maps almost perfectly onto a Minecraft proxy\'s flow. Also confirmed the workspace structure. If Cloudflare runs 20 crates at scale, my 8-crate split is fine.',
  },
  {
    name: 'Tower',

    repo: 'https://github.com/tower-rs/tower',
    desc: 'The Service trait is the foundation of Infrarust\'s middleware pipeline. A Layer wrapping a Service into another Service gave composable, testable middleware for the connection handshake phase. The split between tower-service and tower mirrors infrarust-api vs infrarust-core.',
  },
  {
    name: 'Rama',

    repo: 'https://github.com/plabayo/rama',
    desc: 'The proxy framework closest to the use case. Built on Tower\'s Service/Layer pattern but operates at the TCP level. Seeing the Tower model work for raw TCP connections confirmed the pipeline design was the right call.',
  },
]

const pluginProjects = [
  {
    name: 'Bevy',

    repo: 'https://github.com/bevyengine/bevy',
    desc: 'Bevy\'s Plugin trait, build(&self, app: &mut App), is the most ergonomic plugin registration pattern I found in Rust. One required method, sensible defaults, chainable builder. Infrarust\'s plugin trait follows the same idea.',
  },
  {
    name: 'Envoy Proxy',

    repo: 'https://github.com/envoyproxy/envoy',
    desc: 'Proved that WASM plugins work for network proxies in production. The VM-per-plugin model, shared VM option, documented trade-offs between isolation and performance. Helped me design the PluginLoader and future WASM implementation.',
  },
]

const rustCrates = [
  { name: 'tokio', repo: 'https://github.com/tokio-rs/tokio', desc: 'Async runtime' },
  { name: 'fastnbt', repo: 'https://github.com/owengage/fastnbt', desc: 'NBT serialization for Minecraft registry data' },
  { name: 'libdeflater', repo: 'https://github.com/adamkewley/libdeflater', desc: 'Buffer-based compression/decompression' },
  { name: 'flate2', repo: 'https://github.com/rust-lang/flate2-rs', desc: 'Streaming compression fallback' },
  { name: 'zlib-rs', repo: 'https://github.com/trifectatechfoundation/zlib-rs', desc: 'Pure Rust zlib implementation' },
  { name: 'papaya', repo: 'https://github.com/ibraheemdev/papaya', desc: 'Concurrent HashMap for the player registry' },
  { name: 'arc-swap', repo: 'https://github.com/vorner/arc-swap', desc: 'Lock-free config hot-reload' },
  { name: 'mimalloc', repo: 'https://github.com/purpleprotocol/mimalloc_rust', desc: 'Allocator on musl builds' },
]

const docProjects = [
  { name: 'VitePress', repo: 'https://github.com/vuejs/vitepress', desc: 'The documentation framework I love, carried over from V1.' },
  { name: 'Minecraft Wiki', url: 'https://minecraft.wiki/', desc: 'The protocol documentation that makes projects like Infrarust possible at all.' },
  { name: 'wiki.vg (now merged)', url: 'https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge', desc: 'The original protocol reference. Thank you Tyler Kennedy for the awesome project.' },
]

function repoSlug(url: string): string {
  return url.replace('https://github.com/', '')
}

function repoOwner(url: string): string {
  return repoSlug(url).split('/')[0]
}

const allGithubRepos = [
  ...minecraftProjects,
  ...networkingProjects,
  ...pluginProjects,
  ...rustCrates,
  ...docProjects.filter((d) => d.repo),
].map((p) => repoSlug(p.repo!))

const stars = useMultiGitHubStars(allGithubRepos)
</script>

<template>
  <div class="ack-page">
    <SiteNavbar />

    <!-- Hero -->
    <header class="ack-hero">
      <div class="hero-glow" aria-hidden="true" />
      <div class="hero-inner">
        <div class="ack-chip">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor"
               stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/>
          </svg>
          OPEN SOURCE
        </div>

        <h1 class="hero-title">
          Built on the shoulders<br/>
          <span class="hero-accent">of giants.</span>
        </h1>

        <p class="hero-sub">
          Infrarust V2 wouldn't exist in its current form without the open source projects listed here.
          Some of them I studied for weeks. Some I cloned locally and went through them at 2am trying to understand a design decision.
          All of them taught me something.
        </p>

        <p class="hero-attribution">This is my way of saying thanks.</p>
      </div>
    </header>

    <!-- Minecraft Ecosystem -->
    <section class="category-section">
      <div class="category-inner">
        <div class="category-header">
          <div class="category-icon category-icon--mc" aria-hidden="true">
            <img src="/images/dirt-block.png"
                 alt="" width="44" height="44" class="category-icon-img" />
          </div>
          <div>
            <h2 class="category-title">Minecraft Ecosystem</h2>
            <p class="category-sub">The projects that defined how a Minecraft proxy should work</p>
          </div>
        </div>

        <div class="projects-grid">
          <div v-for="p in minecraftProjects" :key="p.name" class="project-card project-card--mc">
            <div class="project-top">
              <img class="project-logo-img project-logo-img--mc"
                   :src="`https://github.com/${repoOwner(p.repo)}.png?size=96`"
                   :alt="p.name + ' logo'" loading="lazy" />
              <div class="project-meta">
                <h3 class="project-name">
                  {{ p.name }}
                  <span v-if="stars[repoSlug(p.repo)]" class="star-count">
                    &#9733; {{ formatStars(stars[repoSlug(p.repo)]) }}
                  </span>
                </h3>
                <a :href="p.repo" target="_blank" rel="noopener" class="star-btn">
                  <IconGitHub :size="13" />
                  <span>Star on GitHub</span>
                </a>
              </div>
            </div>
            <p class="project-desc">{{ p.desc }}</p>
          </div>
        </div>
      </div>
    </section>

    <!-- Rust Networking -->
    <section class="category-section category-section--alt">
      <div class="category-inner">
        <div class="category-header">
          <div class="category-icon category-icon--net" aria-hidden="true">
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                 stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>
            </svg>
          </div>
          <div>
            <h2 class="category-title">Rust Networking &amp; Proxy Frameworks</h2>
            <p class="category-sub">Where the proxy pipeline design came from</p>
          </div>
        </div>

        <div class="projects-grid">
          <div v-for="p in networkingProjects" :key="p.name" class="project-card project-card--net">
            <div class="project-top">
              <img class="project-logo-img project-logo-img--net"
                   :src="`https://github.com/${repoOwner(p.repo)}.png?size=96`"
                   :alt="p.name + ' logo'" loading="lazy" />
              <div class="project-meta">
                <h3 class="project-name">
                  {{ p.name }}
                  <span v-if="stars[repoSlug(p.repo)]" class="star-count">
                    &#9733; {{ formatStars(stars[repoSlug(p.repo)]) }}
                  </span>
                </h3>
                <a :href="p.repo" target="_blank" rel="noopener" class="star-btn">
                  <IconGitHub :size="13" />
                  <span>Star on GitHub</span>
                </a>
              </div>
            </div>
            <p class="project-desc">{{ p.desc }}</p>
          </div>
        </div>
      </div>
    </section>

    <!-- Plugin Inspirations -->
    <section class="category-section">
      <div class="category-inner">
        <div class="category-header">
          <div class="category-icon category-icon--plugin" aria-hidden="true">
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                 stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <rect x="2" y="6" width="20" height="12" rx="2"/>
              <path d="M12 12h.01"/>
              <path d="M17 12h.01"/>
              <path d="M7 12h.01"/>
            </svg>
          </div>
          <div>
            <h2 class="category-title">Plugin System Inspirations</h2>
            <p class="category-sub">Where the plugin API ideas came from</p>
          </div>
        </div>

        <div class="projects-grid">
          <div v-for="p in pluginProjects" :key="p.name" class="project-card project-card--plugin">
            <div class="project-top">
              <img class="project-logo-img project-logo-img--plugin"
                   :src="`https://github.com/${repoOwner(p.repo)}.png?size=96`"
                   :alt="p.name + ' logo'" loading="lazy" />
              <div class="project-meta">
                <h3 class="project-name">
                  {{ p.name }}
                  <span v-if="stars[repoSlug(p.repo)]" class="star-count">
                    &#9733; {{ formatStars(stars[repoSlug(p.repo)]) }}
                  </span>
                </h3>
                <a :href="p.repo" target="_blank" rel="noopener" class="star-btn">
                  <IconGitHub :size="13" />
                  <span>Star on GitHub</span>
                </a>
              </div>
            </div>
            <p class="project-desc">{{ p.desc }}</p>
          </div>
        </div>
      </div>
    </section>

    <!-- Rust Crates -->
    <section class="category-section category-section--alt">
      <div class="category-inner">
        <div class="category-header">
          <div class="category-icon category-icon--crate" aria-hidden="true">
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                 stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
              <polyline points="3.27 6.96 12 12.01 20.73 6.96"/>
              <line x1="12" y1="22.08" x2="12" y2="12"/>
            </svg>
          </div>
          <div>
            <h2 class="category-title">Rust Crates</h2>
            <p class="category-sub">Every one of these solved a problem I didn't need to solve anymore</p>
          </div>
        </div>

        <div class="crates-grid">
          <a v-for="c in rustCrates" :key="c.name"
             :href="c.repo" target="_blank" rel="noopener"
             class="crate-card">
            <div class="crate-top">
              <code class="crate-name">{{ c.name }}</code>
              <span class="crate-stars" v-if="stars[repoSlug(c.repo)]">
                &#9733; {{ formatStars(stars[repoSlug(c.repo)]) }}
              </span>
              <IconGitHub :size="14" class="crate-gh" />
            </div>
            <p class="crate-desc">{{ c.desc }}</p>
          </a>
        </div>
      </div>
    </section>

    <!-- Documentation -->
    <section class="category-section">
      <div class="category-inner">
        <div class="category-header">
          <div class="category-icon category-icon--doc" aria-hidden="true">
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                 stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/>
              <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/>
            </svg>
          </div>
          <div>
            <h2 class="category-title">Documentation</h2>
            <p class="category-sub">Without these, there's no protocol implementation</p>
          </div>
        </div>

        <div class="crates-grid">
          <a v-for="d in docProjects" :key="d.name"
             :href="d.repo || d.url" target="_blank" rel="noopener"
             class="crate-card">
            <div class="crate-top">
              <span class="crate-name doc-name">{{ d.name }}</span>
              <span class="crate-stars" v-if="d.repo && stars[repoSlug(d.repo)]">
                &#9733; {{ formatStars(stars[repoSlug(d.repo)]) }}
              </span>
            </div>
            <p class="crate-desc">{{ d.desc }}</p>
          </a>
        </div>
      </div>
    </section>

    <!-- Closing -->
    <section class="closing-section">
      <div class="closing-inner">
        <div class="closing-card">
          <p class="closing-text">
            Every project here gave something away for free so that others could build with it.
            Infrarust is one of those others.
          </p>
          <p class="closing-sign">
            Thank's for everything,
            <br/>
            <span class="closing-author">&mdash; Shadowner</span>
          </p>
        </div>
      </div>
    </section>

    <SiteFooter />
  </div>
</template>

<style scoped>
/* ── Page ── */
.ack-page {
  font-family: var(--ir-font-sans);
  color: var(--ir-text);
  background: var(--ir-bg);
  min-height: 100vh;
  overflow-x: hidden;
}

/* ── Hero ── */
.ack-hero {
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

.ack-chip {
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
  margin: 0 0 24px;
}

.hero-accent { color: var(--ir-accent); }

.hero-sub {
  font-size: 17px;
  line-height: 1.7;
  color: var(--ir-text-muted);
  margin: 0 auto 16px;
  max-width: 600px;
}

.hero-attribution {
  font-size: 15px;
  font-style: italic;
  color: var(--ir-accent);
  margin: 0;
  opacity: 0.85;
}

/* ── Category sections ── */
.category-section {
  background: var(--ir-bg);
  padding: 72px 24px;
  border-bottom: 1px solid var(--ir-border);
}

.category-section--alt {
  background: var(--ir-bg-dark);
}

.category-inner {
  max-width: 1100px;
  margin: 0 auto;
}

.category-header {
  display: flex;
  align-items: center;
  gap: 16px;
  margin-bottom: 40px;
}

.category-icon {
  width: 52px;
  height: 52px;
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.category-icon-img {
  width: 52px;
  height: 52px;
  object-fit: contain;
  image-rendering: auto;
}

.category-icon--mc {
  background: none;
  border: none;
}

.category-icon--net {
  background: rgba(232, 131, 42, 0.08);
  border: 1px solid rgba(232, 131, 42, 0.2);
  color: var(--ir-accent);
}

.category-icon--plugin {
  background: rgba(79, 195, 212, 0.08);
  border: 1px solid rgba(79, 195, 212, 0.2);
  color: var(--ir-mc-diamond);
}

.category-icon--crate {
  background: rgba(91, 91, 91, 0.15);
  border: 1px solid rgba(91, 91, 91, 0.3);
  color: #999;
}

.category-icon--doc {
  background: rgba(232, 131, 42, 0.08);
  border: 1px solid rgba(232, 131, 42, 0.2);
  color: var(--ir-accent);
}

.category-title {
  font-weight: 700;
  font-size: 28px;
  color: var(--ir-text);
  letter-spacing: -0.02em;
  margin: 0 0 4px;
}

.category-sub {
  font-size: 14px;
  color: var(--ir-text-muted);
  margin: 0;
}

/* ── Project cards (major projects) ── */
.projects-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
  gap: 16px;
}

.project-card {
  background: var(--ir-surface);
  border: 1px solid var(--ir-border);
  border-radius: 14px;
  padding: 26px 28px;
  transition: border-color 0.2s, transform 0.2s;
}

.project-card:hover {
  transform: translateY(-2px);
}

.project-card--mc:hover    { border-color: rgba(74, 124, 63, 0.35); }
.project-card--net:hover   { border-color: rgba(232, 131, 42, 0.35); }
.project-card--plugin:hover { border-color: rgba(79, 195, 212, 0.35); }

.project-top {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-bottom: 16px;
}

.project-logo-img {
  width: 48px;
  height: 48px;
  border-radius: 10px;
  object-fit: cover;
  flex-shrink: 0;
  background: var(--ir-surface);
}

.project-logo-img--mc {
  border: 1px solid rgba(74, 124, 63, 0.25);
}

.project-logo-img--net {
  border: 1px solid rgba(232, 131, 42, 0.2);
}

.project-logo-img--plugin {
  border: 1px solid rgba(79, 195, 212, 0.2);
}

.project-meta {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.project-name {
  font-weight: 700;
  font-size: 18px;
  color: var(--ir-text);
  margin: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.star-count {
  font-family: var(--ir-font-mono);
  font-size: 12px;
  font-weight: 600;
  color: var(--ir-accent);
  background: rgba(232, 131, 42, 0.08);
  border: 1px solid rgba(232, 131, 42, 0.2);
  padding: 2px 8px;
  border-radius: 999px;
  white-space: nowrap;
}

.star-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-family: var(--ir-font-mono);
  font-size: 11px;
  font-weight: 500;
  color: rgba(240, 237, 232, 0.5);
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.08);
  padding: 4px 10px;
  border-radius: 6px;
  text-decoration: none;
  width: fit-content;
  transition: color 0.2s, border-color 0.2s, background 0.2s;
}

.star-btn:hover {
  color: var(--ir-text);
  border-color: rgba(255, 255, 255, 0.18);
  background: rgba(255, 255, 255, 0.07);
}

.project-desc {
  font-size: 14px;
  line-height: 1.65;
  color: var(--ir-text-muted);
  margin: 0;
}

/* ── Crate cards (compact) ── */
.crates-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 12px;
}

.crate-card {
  background: var(--ir-surface);
  border: 1px solid var(--ir-border);
  border-radius: 10px;
  padding: 18px 20px;
  text-decoration: none;
  transition: border-color 0.2s, transform 0.2s;
}

.crate-card:hover {
  transform: translateY(-1px);
  border-color: rgba(232, 131, 42, 0.3);
}

.crate-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}

.crate-name {
  font-family: var(--ir-font-mono);
  font-size: 14px;
  font-weight: 600;
  color: var(--ir-accent);
}

.doc-name {
  font-family: var(--ir-font-sans);
  color: var(--ir-text);
}

.crate-stars {
  font-family: var(--ir-font-mono);
  font-size: 11px;
  font-weight: 600;
  color: var(--ir-accent);
  margin-left: auto;
}

.crate-gh {
  color: rgba(240, 237, 232, 0.3);
  transition: color 0.2s;
  flex-shrink: 0;
}

.crate-card:hover .crate-gh {
  color: rgba(240, 237, 232, 0.6);
}

.crate-desc {
  font-size: 13px;
  line-height: 1.55;
  color: var(--ir-text-muted);
  margin: 0;
}

/* ── Closing ── */
.closing-section {
  background: var(--ir-bg-dark);
  border-top: 1px solid var(--ir-border);
  padding: 72px 24px;
}

.closing-inner {
  max-width: 760px;
  margin: 0 auto;
}

.closing-card {
  background: linear-gradient(135deg, rgba(232, 131, 42, 0.06) 0%, rgba(232, 131, 42, 0.02) 100%);
  border: 1px solid rgba(232, 131, 42, 0.2);
  border-left: 3px solid var(--ir-accent);
  border-radius: 12px;
  padding: 36px 40px;
  text-align: center;
}

.closing-text {
  font-size: 18px;
  line-height: 1.7;
  color: var(--ir-text);
  font-weight: 500;
  margin: 0 0 24px;
}

.closing-sign {
  font-size: 16px;
  color: var(--ir-text-muted);
  font-style: italic;
  margin: 0;
  line-height: 1.8;
}

.closing-author {
  color: var(--ir-accent);
  font-weight: 600;
  font-style: normal;
}

/* ── Responsive ── */
@media (max-width: 900px) {
  .hero-title { font-size: 42px; }
  .category-title { font-size: 22px; }
  .projects-grid { grid-template-columns: 1fr; }
}

@media (max-width: 767px) {
  .ack-hero { padding: 120px 20px 72px; }
  .hero-title { font-size: 34px; }
  .category-section { padding: 48px 20px; }
  .category-header { flex-direction: column; align-items: flex-start; gap: 12px; }
  .closing-card { padding: 24px 20px; }
  .crates-grid { grid-template-columns: 1fr; }
}
</style>
