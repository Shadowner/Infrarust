import { defineConfig } from 'vitepress'
import { withMermaid } from "vitepress-plugin-mermaid";

export default withMermaid({
  title: "Infrarust",
  description: "High-Performance Minecraft Reverse Proxy in Rust",

  locales: {
    root: {
      label: 'English',
      lang: 'en'
    },
    fr: {
      label: 'Français',
      lang: 'fr', 
      link: '/fr/',
    }
  },

  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/img/logo.svg' }],
  ],

  themeConfig: {
    logo: { src: '/img/logo.svg', width: 24, height: 24 },

    nav: [
      { text: 'Home', link: '/' },
      { text: 'Getting Started', link: '/quickstart/' },
      { text: 'Documentation', link: '/proxy/' },
      { text: 'Contributing', link: '/contributing/' }
    ],

    sidebar: {
      '/fr/': [
        {
          text: 'Pour Commencer',
          items: [
            { text: 'Introduction', link: '/fr/quickstart/' },
            { text: 'Installation', link: '/fr/quickstart/installation' },
            {
              text: 'Configuration',
              link: '/fr/quickstart/configuration/',
            },
            {
              text: 'Déploiement',
              link: '/fr/quickstart/deployment/',
            },
          ]
        },
        {
          text: 'Proxy',
          link: '/fr/proxy/',
          items: [
            { text: 'Performance', link: '/fr/proxy/performance' },
            {
              text: 'Modes de Proxy', link: '/fr/proxy/modes', items: [
                { text: 'Passthrough', link: '/fr/proxy/modes/passthrough' },
                { text: 'ClientOnly', link: '/fr/proxy/modes/client-only' },
                { text: 'Hors-ligne', link: '/fr/proxy/modes/offline' },
                {
                  collapsed: true,
                  text: 'Non fonctionnel', items: [
                    { text: 'Complet', link: '/fr/proxy/modes/full' },
                  ]
                },
              ]
            },
          ]
        },
        {
          text: 'Fonctionnalités',
          items: [
            { text: 'Mise en Cache', link: '/fr/features/caching' },
            { text: 'Rate Limiting', link: '/fr/features/rate-limiting' },
            { text: 'Télémétrie', link: '/fr/features/telemetry' },
            {
              text: 'Feuille de Route', link: "/fr/roadmap/", collapsed: true, items: [
                { text: 'Authentification', link: '/fr/roadmap/authentication' },
                { text: 'Système de Plugins', link: '/fr/roadmap/plugins' },
                { text: 'Api', link: '/fr/roadmap/api' },
                { text: 'Tableau de bord', link: '/fr/roadmap/dashboard' },
              ]
            },
          ]
        },
        {
          text: "Développement",
          link: '/fr/development/',
          collapsed: true,
          items: [
            {
              text: 'Architecture',
              link: '/fr/development/architecture/',
              items: [
                { text: 'Réseau', link: '/fr/development/architecture/network' },
                { text: 'Protocole', link: '/fr/development/architecture/protocol' },
                { text: 'Sécurité', link: '/fr/development/architecture/security' }
              ]
            },
            {
              text: 'API',
              link: '/fr/development/api/',
              items: [
                { text: 'Référence', link: '/fr/development/api/reference' }
              ]
            },

          ]
        },
        {
          text: 'Contribuer',
          items: [],
          link: '/fr/contributing',
        },
      ],
      '/': [
        {
          text: 'Getting Started',
          items: [
            { text: 'Introduction', link: '/quickstart/' },
            { text: 'Installation', link: '/quickstart/installation' },
            {
              text: 'Configuration',
              link: '/quickstart/configuration/',
            },
            {
              text: 'Deployment',
              link: '/quickstart/deployment/',
            },
          ]
        },
        {
          text: 'Proxy',
          link: '/proxy/',
          items: [
            { text: 'Performance', link: '/proxy/performance' },
            {
              text: 'Proxy Modes', link: '/proxy/modes', items: [
                { text: 'Passthrough', link: '/proxy/modes/passthrough' },
                { text: 'ClientOnly', link: '/proxy/modes/client-only' },
                { text: 'Offline', link: '/proxy/modes/offline' },
                {
                  collapsed: true,
                  text: 'Not working', items: [
                    { text: 'Full', link: '/proxy/modes/full' },
                  ]
                },
              ]
            },
          ]
        },
        {
          text: 'Features',
          items: [
            { text: 'Caching', link: '/features/caching' },
            { text: 'Rate Limiting', link: '/features/rate-limiting' },
            { text: 'Telemetry', link: '/features/telemetry' },
            {
              text: 'Roadmap', link: "/roadmap/", collapsed: true, items: [
                { text: 'Authentication', link: '/roadmap/authentication' },
                { text: 'Plugin System', link: '/roadmap/plugins' },
                { text: 'Api', link: '/roadmap/api' },
                { text: 'Dashboard web', link: '/roadmap/dashboard' },
              ]
            },
          ]
        },
        {
          text: "Development",
          link: '/development/',
          collapsed: true,
          items: [
            {
              text: 'Architecture',
              link: '/development/architecture/',
              items: [
                { text: 'Network', link: '/development/architecture/network' },
                { text: 'Protocol', link: '/development/architecture/protocol' },
                { text: 'Security', link: '/development/architecture/security' }
              ]
            },
            {
              text: 'API',
              link: '/development/api/',
              items: [
                { text: 'Reference', link: '/development/api/reference' }
              ]
            },

          ]
        },
        {
          text: 'Contributing',
          link: '/contributing',
          items: []
        },
      ],
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/shadowner/infrarust' },
      {
        icon: 'discord',
        link: 'https://discord.gg/sqbJhZVSgG',
      }
    ],

    footer: {
      message: 'Released under the AGPL-3.0 License.',
      copyright: `Copyright © ${new Date().getFullYear()} Infrarust Contributors`
    },

    search: {
      provider: 'local'
    }
  },

  markdown: {
    theme: {
      light: 'github-light',
      dark: 'github-dark'
    },
    lineNumbers: true,
  }
})