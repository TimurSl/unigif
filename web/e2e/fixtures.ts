import { readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/** The fixture GIFs written by e2e/serve.mjs for the server on `port`, in name order. */
export function fixtureFiles(port = Number(process.env.E2E_PORT ?? 8799)): string[] {
  const dir = join(tmpdir(), `unigif-e2e-${port}`, 'fixtures');
  return readdirSync(dir)
    .filter((f) => f.endsWith('.gif'))
    .sort()
    .map((f) => join(dir, f));
}
