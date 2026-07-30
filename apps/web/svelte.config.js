import node from '@sveltejs/adapter-node';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';

const sdkSource = fileURLToPath(new URL('../../sdk/typescript/src/index.ts', import.meta.url));

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: node({ out: 'build-node', precompress: true }),
    alias: {
      '@piqae/sdk': sdkSource
    }
  }
};

export default config;
