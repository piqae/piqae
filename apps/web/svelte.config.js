import node from '@sveltejs/adapter-node';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';

const sdkSource = fileURLToPath(new URL('../../sdk/typescript/src/index.ts', import.meta.url));
const deployedVersion =
  process.env.RAILWAY_GIT_COMMIT_SHA?.trim() ||
  process.env.GITHUB_SHA?.trim() ||
  process.env.PIQAE_VERSION?.trim();

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: node({ out: 'build-node', precompress: true }),
    alias: {
      '@piqae/sdk': sdkSource
    },
    version: {
      ...(deployedVersion ? { name: deployedVersion } : {}),
      pollInterval: 30_000
    }
  }
};

export default config;
