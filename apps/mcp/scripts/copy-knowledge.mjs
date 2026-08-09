import { cp, mkdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const repositoryRoot = dirname(dirname(packageRoot));
const destination = join(packageRoot, 'dist', 'knowledge');
const sources = [
  'docs/operations/reliability-and-job-lifecycle.md',
  'docs/api/authentication.md',
  'docs/api/README.md',
  'sdk/typescript/README.md',
  'contracts/openapi/piqae-v1.yaml'
];

await mkdir(destination, { recursive: true });
for (const source of sources) {
  await cp(join(repositoryRoot, source), join(destination, source.replaceAll('/', '__')));
}
