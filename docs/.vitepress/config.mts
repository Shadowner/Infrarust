import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Infrarust',
  description: 'High-performance Minecraft reverse proxy written in Rust',
  lang: 'en-US',

  head: [
    ['link', { rel: 'icon', href: '/favicon.ico' }],
    ['meta', { name: 'theme-color', content: '#c0392b' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'Infrarust' }],
    ['meta', { property: 'og:description', content: 'High-performance Minecraft reverse proxy written in Rust' }],
    ['meta', { property: 'og:url', content: 'https://infrarust.dev/' }],
  ],

  // Disable clean URLs to keep .html extension for compatibility
  cleanUrls: true,

  // Markdown configuration
  markdown: {
    lineNumbers: true,
    theme: {
      light: 'github-light',
      dark: 'one-dark-pro',
    },
  },

  // Sitemap for SEO
  sitemap: {
    hostname: 'https://infrarust.dev',
  },

  themeConfig: {
    logo: '/images/logo.svg',
    siteTitle: 'Infrarust',

    // ──────────────────────────────────
    //  Top navigation bar
    // ──────────────────────────────────
    nav: [
      { text: 'Guide', link: '/guide/', activeMatch: '/guide/' },
      { text: 'Configuration', link: '/configuration/', activeMatch: '/configuration/' },
      { text: 'Plugins', link: '/plugins/', activeMatch: '/plugins/' },
      {
        text: 'Reference',
        items: [
          { text: 'Config Reference', link: '/reference/config-reference' },
          { text: 'Proxy Protocol', link: '/reference/proxy-protocol' },
          { text: 'Error Codes', link: '/reference/error-codes' },
        ],
      },
      {
        text: 'v2.0',
        items: [
          { text: 'Changelog', link: 'https://github.com/Shadowner/Infrarust/blob/main/CHANGELOG.md' },
          { text: 'Contributing', link: 'https://github.com/Shadowner/Infrarust/blob/main/CONTRIBUTING.md' },
          { text: 'v1 Docs', link: 'https://v1.infrarust.dev/' },
        ],
      },
    ],

    // ──────────────────────────────────
    //  Sidebars (one per section)
    // ──────────────────────────────────
    sidebar: {
      '/guide/': [
        {
          text: 'Getting Started',
          items: [
            { text: 'What is Infrarust?', link: '/guide/' },
            { text: 'Installation', link: '/guide/installation' },
            { text: 'Quick Start', link: '/guide/quick-start' },
          ],
        },
        {
          text: 'Core Concepts',
          items: [
            { text: 'How it Works', link: '/guide/concepts' },
            { text: 'Proxy Modes', link: '/guide/proxy-modes' },
            { text: 'Routing & Wildcards', link: '/guide/routing' },
            { text: 'Pipeline & Middleware', link: '/guide/pipeline' },
          ],
        },
        {
          text: 'Deployment',
          items: [
            { text: 'Docker', link: '/guide/docker' },
            { text: 'Systemd Service', link: '/guide/systemd' },
            { text: 'Behind a Load Balancer', link: '/guide/load-balancer' },
          ],
        },
      ],

      '/configuration/': [
        {
          text: 'Configuration',
          items: [
            { text: 'Overview', link: '/configuration/' },
            { text: 'Global Config', link: '/configuration/global' },
            { text: 'Server Definitions', link: '/configuration/servers' },
          ],
        },
        {
          text: 'Proxy Modes',
          collapsed: false,
          items: [
            { text: 'Overview', link: '/configuration/proxy-modes/' },
            { text: 'Passthrough', link: '/configuration/proxy-modes/passthrough' },
            { text: 'Zero-Copy', link: '/configuration/proxy-modes/zerocopy' },
            { text: 'Client-Only', link: '/configuration/proxy-modes/client-only' },
            { text: 'Offline', link: '/configuration/proxy-modes/offline' },
            { text: 'Server-Only', link: '/configuration/proxy-modes/server-only' },
          ],
        },
        {
          text: 'Providers',
          items: [
            { text: 'File Provider', link: '/configuration/file-provider' },
            { text: 'Docker Discovery', link: '/configuration/docker' },
          ],
        },
        {
          text: 'Security',
          items: [
            { text: 'Rate Limiting', link: '/configuration/rate-limiting' },
            { text: 'Ban System', link: '/configuration/bans' },
            { text: 'IP Filtering', link: '/configuration/ip-filtering' },
            { text: 'Proxy Protocol', link: '/configuration/proxy-protocol' },
          ],
        },
        {
          text: 'Monitoring',
          items: [
            { text: 'Telemetry (OpenTelemetry)', link: '/configuration/telemetry' },
            { text: 'Status Cache', link: '/configuration/status-cache' },
          ],
        },
      ],

      '/plugins/': [
        {
          text: 'Plugin System',
          items: [
            { text: 'Overview', link: '/plugins/' },
            { text: 'Plugin Lifecycle', link: '/plugins/lifecycle' },
            { text: 'Events Reference', link: '/plugins/events' },
            { text: 'Commands', link: '/plugins/commands' },
          ],
        },
        {
          text: 'Built-in Plugins',
          items: [
            { text: 'Auth', link: '/plugins/auth' },
            { text: 'Server Wake', link: '/plugins/server-wake' },
            { text: 'Queue', link: '/plugins/queue' },
          ],
        },
        {
          text: 'Developing Plugins',
          items: [
            { text: 'Writing a Plugin', link: '/plugins/writing-plugins' },
            { text: 'Plugin API', link: '/plugins/api' },
            { text: 'Testing Plugins', link: '/plugins/testing' },
          ],
        },
      ],

      '/reference/': [
        {
          text: 'Reference',
          items: [
            { text: 'Config Reference', link: '/reference/config-reference' },
            { text: 'Proxy Protocol', link: '/reference/proxy-protocol' },
            { text: 'Error Codes', link: '/reference/error-codes' },
          ],
        },
      ],

      '/advanced/': [
        {
          text: 'Advanced',
          items: [
            { text: 'Architecture', link: '/advanced/architecture' },
            { text: 'Performance Tuning', link: '/advanced/performance' },
            { text: 'Zerocopy & Splice', link: '/advanced/zerocopy' },
            { text: 'Migration from V1', link: '/advanced/migration-v1' },
          ],
        },
      ],
    },

    // ──────────────────────────────────
    //  Social links
    // ──────────────────────────────────
    socialLinks: [
      { icon: 'github', link: 'https://github.com/Shadowner/Infrarust' },
      { icon: 'discord', link: 'https://discord.gg/infrarust' },
    ],

    // ──────────────────────────────────
    //  Edit on GitHub
    // ──────────────────────────────────
    editLink: {
      pattern: 'https://github.com/Shadowner/Infrarust/edit/main/docs/:path',
      text: 'Edit this page on GitHub',
    },

    // ──────────────────────────────────
    //  Search (built-in local search)
    // ──────────────────────────────────
    search: {
      provider: 'local',
      options: {
        detailedView: true,
      },
    },

    // ──────────────────────────────────
    //  Footer
    // ──────────────────────────────────
    footer: {
      message: 'Released under the AGPL-3.0 License.',
      copyright: 'Copyright © 2024-present Infrarust Contributors',
    },

    // ──────────────────────────────────
    //  Last updated timestamp
    // ──────────────────────────────────
    lastUpdated: {
      text: 'Last updated',
      formatOptions: {
        dateStyle: 'medium',
      },
    },

    // ──────────────────────────────────
    //  Navigation labels
    // ──────────────────────────────────
    docFooter: {
      prev: 'Previous',
      next: 'Next',
    },
    outline: {
      label: 'On this page',
      level: [2, 3],
    },
    returnToTopLabel: 'Back to top',
    darkModeSwitchLabel: 'Theme',
    sidebarMenuLabel: 'Menu',
  },
})