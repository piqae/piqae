import { sentrySvelteKit } from '@sentry/sveltekit';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import { resolveSentryBuildConfiguration } from './src/lib/observability/sentry-build';

const sentryBuild = resolveSentryBuildConfiguration(process.env);

export default defineConfig({
  plugins: [
    sentrySvelteKit({
      autoUploadSourceMaps: sentryBuild.uploadSourceMaps,
      autoInstrument: process.env.NODE_ENV !== 'test',
      authToken: sentryBuild.authToken,
      org: sentryBuild.organization,
      project: sentryBuild.project,
      release: sentryBuild.release
        ? {
            name: sentryBuild.release,
            inject: true
          }
        : undefined,
      telemetry: false,
      debug: false
    }),
    sveltekit()
  ],
  resolve: {
    conditions: ['browser']
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    setupFiles: ['./src/test-setup.ts']
  }
});
