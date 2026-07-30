import assert from 'node:assert/strict';
import { mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const scratch = await mkdtemp(join(tmpdir(), 'piqae-sdk-smoke-'));

function run(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: 'utf8',
    stdio: 'pipe'
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(' ')} failed\n${result.stdout}\n${result.stderr}`
    );
  }
}

try {
  run('npm', ['pack', '.', '--pack-destination', scratch], process.cwd());
  const archive = (await readdir(scratch)).find((entry) => entry.endsWith('.tgz'));
  assert.ok(archive, 'npm pack did not create an SDK archive');

  await writeFile(
    join(scratch, 'package.json'),
    JSON.stringify({ name: 'piqae-sdk-smoke', private: true, type: 'module' })
  );
  run(
    'npm',
    ['install', '--ignore-scripts', '--no-audit', '--no-fund', join(scratch, archive)],
    scratch
  );

  const sdk = await import(
    pathToFileURL(join(scratch, 'node_modules/@piqae/sdk/dist/index.js')).href
  );
  assert.equal(typeof sdk.PiqaeClient, 'function');
  assert.equal(typeof sdk.PiqaePlatform, 'function');
  assert.equal(typeof sdk.PiqaeError, 'function');
  assert.equal(new sdk.PiqaeClient().baseUrl, 'https://api.piqae.com');
} finally {
  await rm(scratch, { recursive: true, force: true });
}
