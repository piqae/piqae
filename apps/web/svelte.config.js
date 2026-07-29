import node from '@sveltejs/adapter-node';
import vercel from '@sveltejs/adapter-vercel';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

const target = process.env.SPOOL_DEPLOYMENT_TARGET ?? 'vercel';

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter:
      target === 'node'
        ? node({ out: 'build-node', precompress: true })
        : vercel({ runtime: 'nodejs22.x', regions: ['sfo1'] })
  }
};

export default config;
