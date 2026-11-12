// Starts a throwaway unigif instance for the end-to-end tests and writes the fixture GIFs.
import { spawn, spawnSync } from 'node:child_process';
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const port = process.env.E2E_PORT ?? '8799';
const root = resolve(import.meta.dirname, '..', '..');
const bin = process.env.UNIGIF_BIN ?? join(root, 'target', 'debug', 'unigif');
// A fixed, per-port directory (wiped at start) so the tests can find the fixtures.
const dir = join(tmpdir(), `unigif-e2e-${port}`);
rmSync(dir, { recursive: true, force: true });
mkdirSync(dir, { recursive: true });

const env = {
  ...process.env,
  UNIGIF_DATA_DIR: join(dir, 'data'),
  UNIGIF_CACHE_DIR: join(dir, 'cache'),
  UNIGIF_LISTEN: `127.0.0.1:${port}`,
  RUST_LOG: 'warn',
};

// Fixtures: six different animated GIFs (different bytes, so none is a duplicate).
const fixtures = join(dir, 'fixtures');
mkdirSync(fixtures);
const sources = ['testsrc', 'testsrc2', 'smptebars', 'rgbtestsrc', 'yuvtestsrc', 'pal75bars'];
sources.forEach((src, i) => {
  const r = spawnSync(
    'ffmpeg',
    ['-v', 'error', '-f', 'lavfi', '-i', `${src}=size=160x120:rate=10:duration=1`, '-y', join(fixtures, `fixture-${i + 1}.gif`)],
    { stdio: 'inherit' },
  );
  if (r.status !== 0) throw new Error('ffmpeg could not create the fixtures');
});
console.log(`e2e fixtures in ${fixtures}`);

writeFileSync(join(dir, 'password'), 'e2e-password-1\n');
const add = spawnSync(bin, ['user', 'add', 'e2e', '--password-file', join(dir, 'password')], { env, stdio: 'inherit' });
if (add.status !== 0) throw new Error('could not create the e2e user');

const server = spawn(bin, ['serve'], { env, stdio: 'inherit' });
server.on('exit', (code) => process.exit(code ?? 1));
for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, () => server.kill());
