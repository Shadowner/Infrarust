import { defineConfig } from 'vitepress'

// https://vitepress.dev/reference/site-config
export default defineConfig({
  title: "Infrarust",
  description: "High-Performance Minecraft Reverse Proxy in Rust",

  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: 'img/logo.svg' }],
  ],

  themeConfig: {
    // https://vitepress.dev/reference/default-theme-config
    logo: { src: 'img/logo.svg', width: 24, height: 24 },

    nav: [
      { text: 'Home', link: '/' },
      { text: 'Guide', link: '/docs/installation' },
      { text: 'Documentation', link: '/docs/configuration' },
      { text: 'Reference', link: '/docs/api' }
    ],

    sidebar: [
      {
        text: 'Getting Started',
        items: [
          { text: 'Installation', link: '/docs/installation' },
          { text: 'Quick Start', link: '/docs/quick-start' }
        ]
      },
      {
        text: 'Configuration',
        items: [
          { text: 'Basic Setup', link: '/docs/configuration' },
          { text: 'Proxy Modes', link: '/docs/proxy-modes' },
          { text: 'Domain Routing', link: '/docs/domain-routing' }
        ]
      },
      {
        text: 'Features',
        items: [
          { text: 'Authentication', link: '/docs/authentication' },
          { text: 'Security', link: '/docs/security' },
          { text: 'Performance', link: '/docs/performance' }
        ]
      },
      {
        text: 'API Reference',
        items: [
          { text: 'Overview', link: '/docs/api' },
          { text: 'Configuration API', link: '/docs/api/configuration' }
        ]
      }
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/shadowner/infrarust' }
    ],

    footer: {
      message: 'Released under the AGPL-3.0 License.',
      copyright: `Copyright © ${new Date().getFullYear()} Infrarust Contributors`
    },

    search: {
      provider: 'local'
    }
  },

  // Personnalisation du markdown
  markdown: {
    theme: {
      light: 'github-light',
      dark: 'github-dark'
    },
    lineNumbers: true
  }
})