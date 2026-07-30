import assert from 'node:assert/strict';
import test from 'node:test';

import { routeRequest } from './src/index.mjs';

test('keeps marketing traffic on the web origin', () => {
  assert.deepEqual(routeRequest('https://piqae.com/pricing?annual=true'), {
    kind: 'proxy',
    origin: 'web',
    pathname: '/pricing'
  });
});

test('moves authenticated paths to the app hostname', () => {
  assert.deepEqual(routeRequest('https://piqae.com/dashboard/jobs?state=queued'), {
    kind: 'redirect',
    destination: 'https://app.piqae.com/dashboard/jobs?state=queued'
  });
});

test('maps the app root to the dashboard', () => {
  assert.deepEqual(routeRequest('https://app.piqae.com/'), {
    kind: 'redirect',
    destination: 'https://app.piqae.com/dashboard'
  });
});

test('maps connect root to the pairing flow and preserves the code', () => {
  assert.deepEqual(routeRequest('https://connect.piqae.com/?user_code=ABCD1234'), {
    kind: 'redirect',
    destination: 'https://app.piqae.com/pair?user_code=ABCD1234'
  });
});

test('maps documentation and downloads onto existing web routes', () => {
  assert.deepEqual(routeRequest('https://docs.piqae.com/api/quickstart'), {
    kind: 'redirect',
    destination: 'https://piqae.com/docs/api/quickstart'
  });
  assert.deepEqual(routeRequest('https://downloads.piqae.com/'), {
    kind: 'redirect',
    destination: 'https://piqae.com/downloads'
  });
  assert.deepEqual(
    routeRequest('https://downloads.piqae.com/releases/stable/appcast-macos.xml'),
    {
      kind: 'proxy',
      origin: 'web',
      pathname: '/releases/stable/appcast-macos.xml'
    }
  );
});

test('keeps API and node sync on the control-plane origin', () => {
  assert.deepEqual(routeRequest('https://api.piqae.com/v1/jobs'), {
    kind: 'proxy',
    origin: 'api',
    pathname: '/v1/jobs'
  });
  assert.deepEqual(routeRequest('https://sync.piqae.com/v1/agents/node/sync'), {
    kind: 'proxy',
    origin: 'api',
    pathname: '/v1/agents/node/sync'
  });
});

test('rejects unregistered hosts', () => {
  assert.deepEqual(routeRequest('https://unknown.piqae.com/'), { kind: 'reject' });
});
