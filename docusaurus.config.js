// @ts-check
import {themes as prismThemes} from 'prism-react-renderer';

/** @type {import('@docusaurus/types').Config} */
const config = {
  title: 'llm_brain',
  tagline: 'One LLM backend for every app and CLI — budget-first, cheap models today, own GPU tomorrow',
  favicon: 'img/favicon.svg',
  future: {v4: true},
  url: 'https://pjcau.github.io',
  baseUrl: '/llm_brain/',
  organizationName: 'pjcau',
  projectName: 'llm_brain',
  trailingSlash: false,
  onBrokenLinks: 'throw',
  markdown: {mermaid: true, hooks: {onBrokenMarkdownLinks: 'throw'}},
  themes: ['@docusaurus/theme-mermaid'],
  i18n: {defaultLocale: 'en', locales: ['en']},
  presets: [
    [
      'classic',
      /** @type {import('@docusaurus/preset-classic').Options} */
      ({
        docs: {
          routeBasePath: '/',
          sidebarPath: './sidebars.js',
          editUrl: 'https://github.com/pjcau/llm_brain/edit/main/',
          showLastUpdateTime: false,
        },
        blog: false,
        theme: {customCss: './src/css/custom.css'},
      }),
    ],
  ],
  themeConfig:
    /** @type {import('@docusaurus/preset-classic').ThemeConfig} */
    ({
      image: 'img/social-card.svg',
      colorMode: {respectPrefersColorScheme: true},
      navbar: {
        title: 'llm_brain',
        logo: {alt: 'llm_brain', src: 'img/logo.svg'},
        items: [
          {to: '/', label: 'Overview', position: 'left'},
          {to: '/roadmap', label: 'Roadmap', position: 'left'},
          {to: '/decisions', label: 'Decisions', position: 'left'},
          {to: '/changelog', label: 'Changelog', position: 'left'},
          {href: 'https://github.com/pjcau/llm_brain', label: 'GitHub', position: 'right'},
          {href: 'https://github.com/pjcau/agent-orchestrator', label: 'agent-orchestrator', position: 'right'},
        ],
      },
      footer: {
        style: 'dark',
        links: [
          {title: 'Docs', items: [{label: 'Overview', to: '/'}, {label: 'Budget', to: '/architecture/budget'}, {label: 'Diagrams', to: '/diagrams'}]},
          {title: 'Repos', items: [{label: 'llm_brain', href: 'https://github.com/pjcau/llm_brain'}, {label: 'agent-orchestrator', href: 'https://github.com/pjcau/agent-orchestrator'}, {label: 'claude-kit', href: 'https://github.com/pjcau/claude-kit'}]},
        ],
        copyright: 'llm_brain — a living document, updated at every iteration.',
      },
      prism: {theme: prismThemes.github, darkTheme: prismThemes.dracula, additionalLanguages: ['bash', 'yaml', 'json', 'sql']},
      mermaid: {theme: {light: 'neutral', dark: 'dark'}},
    }),
};

export default config;
