// Starts a throwaway unigif instance for the end-to-end tests and writes the fixture GIFs.
import { spawn, spawnSync } from 'node:child_process';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
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

// The scale suite: 50 000 GIF rows (no media files), the same seed as tests/scale.rs.
if (process.env.E2E_SEED_SCALE) {
  const { DatabaseSync } = await import('node:sqlite');
  const db = new DatabaseSync(join(dir, 'data', 'unigif.db'));
  db.exec(readFileSync(join(root, 'tests', 'scale', 'seed.sql'), 'utf8'));
  db.close();
  console.log('seeded 50 000 GIFs');
}

// The search suite talks to a stand-in for GIPHY (a real one needs a key and the internet).
if (process.env.E2E_PROVIDER) {
  const mockPort = Number(port) + 100;
  const gifs = { 'e2e-one': readFileSync(join(fixtures, 'fixture-1.gif')), 'e2e-two': readFileSync(join(fixtures, 'fixture-2.gif')) };
  const base = `http://127.0.0.1:${mockPort}`;
  createServer((req, res) => {
    const url = new URL(req.url, base);
    if (url.pathname === '/files/a.gif') {
      res.writeHead(200, { 'content-type': 'image/gif' }).end(gifs[url.searchParams.get('id')] ?? gifs['e2e-one']);
    } else if (url.pathname.startsWith('/giphy/gifs/')) {
      if (url.searchParams.get('api_key') !== 'e2e-giphy-key') {
        res.writeHead(403, { 'content-type': 'application/json' }).end('{"message":"invalid key"}');
        return;
      }
      const item = (id, title) => ({
        id,
        title,
        url: `https://giphy.com/gifs/${id}`,
        images: {
          original: { url: `${base}/files/a.gif?id=${id}`, width: '160', height: '120', size: String(gifs[id].length) },
          fixed_width: { url: `${base}/files/a.gif?id=${id}` },
        },
      });
      const q = url.searchParams.get('q') ?? '';
      res.writeHead(200, { 'content-type': 'application/json' }).end(
        JSON.stringify({
          data: [item('e2e-one', `${q || 'trending'} one`), item('e2e-two', `${q || 'trending'} two`)],
          pagination: { total_count: 2, count: 2, offset: 0 },
        }),
      );
    } else {
      res.writeHead(404).end();
    }
  }).listen(mockPort, '127.0.0.1');
  env.UNIGIF_SEARCH__GIPHY_BASE_URL = `${base}/giphy`;
  env.UNIGIF_FETCH__ALLOW_PRIVATE = 'true';
}

const server = spawn(bin, ['serve'], { env, stdio: 'inherit' });
server.on('exit', (code) => process.exit(code ?? 1));
for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, () => server.kill());
