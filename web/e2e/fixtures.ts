import { readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/** The fixture GIFs written by e2e/serve.mjs, in name order. */
export function fixtureFiles(): string[] {
  const dir = join(tmpdir(), `unigif-e2e-${process.env.E2E_PORT ?? 8799}`, 'fixtures');
  return readdirSync(dir)
    .filter((f) => f.endsWith('.gif'))
    .sort()
    .map((f) => join(dir, f));
}
