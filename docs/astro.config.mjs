import starlight from '@astrojs/starlight';
import { defineConfig } from 'astro/config';
import starlightPageActions from 'starlight-page-actions';

// ponytail: the agent prompts in src/content/docs repeat this URL, because page actions copy each page's source as is,
// so a change here needs a find-and-replace across the docs too.
const site = 'https://prolix.barclayd.workers.dev';

export default defineConfig({
  site,
  devToolbar: { enabled: false },
  integrations: [
    starlight({
      title: 'prolix',
      description: "A linter for comments that don't earn their place, judged by Jev. Set it up by handing a page to your coding agent.",
      favicon: '/favicon.svg',
      plugins: [starlightPageActions({ baseUrl: site, actions: { cursor: true } })],
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/barclayd/prolix' }],
      editLink: { baseUrl: 'https://github.com/barclayd/prolix/edit/main/docs/' },
      customCss: ['./src/styles/custom.css'],
      sidebar: [
        {
          label: 'Start here',
          items: [
            { label: 'Set up', slug: 'setup' },
            { label: 'Clean up an existing repo', slug: 'adopt' },
            { label: 'Use with coding agents', slug: 'agents' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'GitHub Action', slug: 'reference/action' },
            { label: 'CLI', slug: 'reference/cli' },
            { label: 'Levels and categories', slug: 'reference/levels' },
            { label: 'Configuration', slug: 'reference/configuration' },
          ],
        },
        { label: 'How it works', slug: 'how-it-works' },
      ],
    }),
  ],
});
