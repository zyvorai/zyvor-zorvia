import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'Zorvia',
  tagline: 'The control plane for KubeVirt VMs — create, operate, and govern from CLI, TUI, and web.',
  favicon: 'img/favicon.svg',

  future: {
    v4: true,
  },

  url: 'https://zyvorai.github.io',
  baseUrl: '/zyvor-zorvia/',

  organizationName: 'zyvorai',
  projectName: 'zyvor-zorvia',

  onBrokenLinks: 'warn',

  markdown: {
    format: 'md',
    hooks: {
      onBrokenMarkdownLinks: 'warn',
    },
  },

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  // Serve README screenshots and social/OG cards in place instead of duplicating them.
  staticDirectories: ['static', '../docs/screenshots', '../docs/social'],

  presets: [
    [
      'classic',
      {
        docs: {
          path: '../docs',
          routeBasePath: 'docs',
          sidebarPath: './sidebars.ts',
          editUrl: 'https://github.com/zyvorai/zyvor-zorvia/tree/main/docs/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    image: 'zorvia-share-card.png',
    colorMode: {
      defaultMode: 'dark',
      respectPrefersColorScheme: false,
    },
    navbar: {
      title: 'Zorvia',
      logo: {
        alt: 'Zorvia',
        src: 'img/favicon.svg',
      },
      hideOnScroll: false,
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'docsSidebar',
          position: 'right',
          label: 'Docs',
        },
        {
          href: 'https://github.com/zyvorai/zyvor-zorvia',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Docs',
          items: [
            {label: 'Lab deploy', to: '/docs/LAB'},
            {label: 'Web console', to: '/docs/WEB_CONSOLE'},
            {label: 'Pods: logs & exec', to: '/docs/PODS'},
            {label: 'Interactive TUI', to: '/docs/INTERACTIVE_TUI'},
            {label: 'Snapshots', to: '/docs/SNAPSHOTS'},
          ],
        },
        {
          title: 'Project',
          items: [
            {label: 'GitHub', href: 'https://github.com/zyvorai/zyvor-zorvia'},
            {label: 'Changelog', href: 'https://github.com/zyvorai/zyvor-zorvia/blob/main/CHANGELOG.md'},
            {label: 'License (Apache-2.0)', href: 'https://github.com/zyvorai/zyvor-zorvia/blob/main/LICENSE'},
          ],
        },
        {
          title: 'Zyvor',
          items: [
            {label: 'zyvor.dev', href: 'https://zyvor.dev'},
            {label: 'sales@zyvor.dev', href: 'mailto:sales@zyvor.dev'},
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} Zyvor. Zorvia is Apache-2.0 licensed.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
