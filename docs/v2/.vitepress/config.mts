import { defineConfig } from 'vitepress'

export default
  defineConfig({
    title: 'Infrarust',
    description: 'High-performance Minecraft reverse proxy written in Rust',
    lang: 'en-US',
    base: '/',
    cleanUrls: true,
    lastUpdated: true,
    ignoreDeadLinks: true,

    head: [
      ['link', { rel: 'icon', type: 'image/svg+xml', href: '/images/logo.svg' }],
      ['link', { rel: 'icon', type: 'image/png', sizes: '32x32', href: '/images/favicon-32x32.png' }],
      ['meta', { name: 'theme-color', content: '#E8832A' }],
      ['meta', { property: 'og:type', content: 'website' }],
      ['meta', { property: 'og:site_name', content: 'Infrarust' }],
      ['meta', { property: 'og:title', content: 'Infrarust — Minecraft Reverse Proxy' }],
      ['meta', { property: 'og:description', content: 'High-performance Minecraft reverse proxy written in Rust' }],
      ['meta', { property: 'og:url', content: 'https://infrarust.dev/' }],
      ['meta', { property: 'og:image', content: 'https://infrarust.dev/images/og-banner.png' }],
      ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
      ['meta', { name: 'twitter:title', content: 'Infrarust — Minecraft Reverse Proxy' }],
      ['meta', { name: 'twitter:description', content: 'High-performance Minecraft reverse proxy written in Rust' }],
    ],

    sitemap: {
      hostname: 'https://infrarust.dev',
    },

    markdown: {
      lineNumbers: true,
      theme: {
        light: 'github-light',
        dark: 'one-dark-pro',
      },
    },

    // Mermaid when vitepressmermaid update to 2.x
    // mermaid: {
    // https://mermaid.js.org/config/setup/modules/mermaidAPI.html#mermaidapi-configuration-defaults
    // },

    transformPageData(pageData) {
      const canonicalUrl = `https://infrarust.dev/${pageData.relativePath}`
        .replace(/index\.md$/, '')
        .replace(/\.md$/, '')

      pageData.frontmatter.head ??= []

      pageData.frontmatter.head.push(
        ['link', { rel: 'canonical', href: canonicalUrl }],
        ['meta', { property: 'og:url', content: canonicalUrl }],
      )

      if (pageData.title) {
        pageData.frontmatter.head.push(
          ['meta', { property: 'og:title', content: `${pageData.title} | Infrarust` }],
        )
      }

      if (pageData.description) {
        pageData.frontmatter.head.push(
          ['meta', { property: 'og:description', content: pageData.description }],
        )
      }
    },

    themeConfig: {
      logo: { src: '/images/logo.svg', width: 24, height: 24 },
      siteTitle: 'Infrarust',
      nav: [
        { text: 'Guide', link: '/guide/', activeMatch: '/guide/' },
        { text: 'Configuration', link: '/configuration/', activeMatch: '/configuration/' },
        { text: 'Plugins', link: '/plugins/', activeMatch: '/plugins/' },
        { text: 'Reference', link: '/reference/', activeMatch: '/reference/' },
        {
          text: 'v2.0-alpha01',
          items: [
            {
              text: 'Release',
              items: [
                { text: 'Changelog', link: 'https://github.com/Shadowner/Infrarust/blob/main/CHANGELOG.md' },
                { text: 'Contributing', link: 'https://github.com/Shadowner/Infrarust/blob/main/CONTRIBUTING.md' },
                { text: 'Acknowledgements', link: '/thank-you-open-source' },
              ],
            },
            {
              text: 'Previous versions',
              items: [
                { text: 'V1 Documentation', link: 'https://infrarust.dev/v1/' },
              ],
            },
          ],
        },
      ],

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
              { text: 'How it Works', link: '/guide/how-it-works' },
              { text: 'Proxy Modes', link: '/guide/proxy-modes' },
              { text: 'Routing & Wildcards', link: '/guide/routing' },
            ],
          },
          {
            text: 'Deployment',
            items: [
              { text: 'Docker', link: '/guide/docker' },
              { text: 'Docker Compose', link: '/guide/docker-compose' },
              { text: 'Systemd Service', link: '/guide/systemd' },
              { text: 'Behind a Load Balancer', link: '/guide/load-balancer' },
            ],
          },
          {
            text: 'Troubleshooting',
            items: [
              { text: 'Common Issues', link: '/guide/common-issues' },
              { text: 'FAQ', link: '/guide/faq' },
            ],
          },
        ],
        '/configuration/': [
          {
            text: 'Configuration',
            items: [
              { text: 'Overview', link: '/configuration/' },
              { text: 'Global Settings', link: '/configuration/global' },
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
              { text: 'Proxy Forwarding', link: '/configuration/proxy-forwarding' },
            ],
          },
          {
            text: 'Providers',
            items: [
              { text: 'File Provider', link: '/configuration/providers/file' },
              { text: 'Docker Discovery', link: '/configuration/providers/docker' },
            ],
          },
          {
            text: 'Security',
            items: [
              { text: 'Permissions', link: '/configuration/security/permissions' },
              { text: 'Rate Limiting', link: '/configuration/security/rate-limiting' },
              { text: 'Ban System', link: '/configuration/security/bans' },
              { text: 'IP Filtering', link: '/configuration/security/ip-filtering' },
              { text: 'Proxy Protocol', link: '/configuration/security/proxy-protocol' },
            ],
          },
          {
            text: 'Monitoring',
            items: [
              { text: 'Telemetry (OpenTelemetry)', link: '/configuration/monitoring/telemetry' },
              { text: 'Status Cache', link: '/configuration/monitoring/status-cache' },
            ],
          },
        ],
        '/plugins/': [
          {
            text: 'Using Plugins',
            items: [
              { text: 'Overview', link: '/plugins/' },
              { text: 'Installing Plugins', link: '/plugins/installing' },
            ],
          },
          {
            text: 'Built-in Plugins',
            items: [
              { text: 'Admin API & Web UI', link: '/plugins/builtin/admin-api' },
              { text: 'Auth', link: '/plugins/builtin/auth' },
              { text: 'Server Wake', link: '/plugins/builtin/server-wake' },
              { text: 'Queue', link: '/plugins/builtin/queue' },
            ],
          },
          {
            text: 'Developing Plugins',
            collapsed: false,
            items: [
              { text: 'Getting Started', link: '/plugins/dev/getting-started' },
              { text: 'Architecture & Pipeline', link: '/plugins/dev/architecture' },
              { text: 'Plugin Lifecycle', link: '/plugins/dev/lifecycle' },
              { text: 'Events Reference', link: '/plugins/dev/events' },
              { text: 'Commands API', link: '/plugins/dev/commands' },
              { text: 'Plugin API', link: '/plugins/dev/api' },
              { text: 'Testing Plugins', link: '/plugins/dev/testing' },
            ],
          },
        ],
        '/reference/': [
          {
            text: 'Reference',
            items: [
              { text: 'Overview', link: '/reference/' },
              { text: 'Config Schema', link: '/reference/config-schema' },
              { text: 'CLI Reference', link: '/reference/cli' },
              { text: 'Error Codes', link: '/reference/error-codes' },
              { text: 'Proxy Protocol Spec', link: '/reference/proxy-protocol' },
            ],
          },
          {
            text: 'Advanced',
            items: [
              { text: 'Architecture Overview', link: '/reference/architecture' },
              { text: 'Performance Tuning', link: '/reference/performance' },
              { text: 'Zerocopy & Splice', link: '/reference/zerocopy' },
            ],
          },
          {
            text: 'Migration',
            items: [
              { text: 'Migration from V1', link: '/reference/migration-v1' },
            ],
          },
        ],
      },
      socialLinks: [
        { icon: 'github', link: 'https://github.com/Shadowner/Infrarust' },
        { icon: 'discord', link: 'https://discord.gg/sqbJhZVSgG' },
      ],
      editLink: {
        pattern: 'https://github.com/Shadowner/Infrarust/edit/main/docs/v2/:path',
        text: 'Edit this page on GitHub',
      },
      search: {
        provider: 'local',
        options: {
          detailedView: true,
          miniSearch: {
            searchOptions: {
              fuzzy: 0.2,
              prefix: true,
              boost: { title: 4, text: 2, titles: 1 },
            },
          },
        },
      },
      footer: {
        message: 'Released under the <a href="https://github.com/Shadowner/Infrarust/blob/main/LICENSE">AGPL-3.0 License</a>.',
        copyright: `Copyright © 2024-${new Date().getFullYear()} Infrarust Contributors`,
      },
      lastUpdated: {
        text: 'Last updated',
        formatOptions: {
          dateStyle: 'medium',
        },
      },
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
      externalLinkIcon: true,
    },
  })