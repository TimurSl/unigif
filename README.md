# unigif

A self-hosted, hashtag-first GIF library. Every GIF carries hashtags; new ones land in **Unsorted** and leave it the moment they get their first hashtag. GIFs are stored as small H.264 MP4s and served in whatever format a client needs, through a documented HTTP API with scoped keys.

The full design is in [`PLAN.md`](PLAN.md). **Status:** milestones M0 (skeleton) and M1 (storage core) are implemented; auth, the web UI and the rest follow the plan's milestone order.

## What works today

- `POST /api/v1/gifs`: quick add (raw body or multipart). Hashes, deduplicates (by content and by `source` + `source_id`), probes with ffprobe, enforces size / dimension / duration / frame limits, then transcodes in a background worker pool that survives restarts.
- Stored variants: `full.mp4` (H.264, no audio, ≤ 720 px long edge, `+faststart`; a plain remux when the source already qualifies), `preview.mp4`, `poster.webp`, plus a ThumbHash and a dHash. Transparent GIFs keep their original and are composited on a matte colour for the MP4.
- `GET /m/{id}/{variant}`: public, unguessable-URL media with `Range`, `ETag` and immutable caching.
- `GET /api/v1/gifs/{id}`, `/healthz`, `/readyz`, and the embedded SPA shell.
- CLI: `serve`, `doctor`, `import <dir> [--tags-from-folders]`.
- **Interim auth:** one static bearer token (`auth.bootstrapToken`). Without it every `/api/v1` request is refused. It is replaced by logins and scoped API keys in M2.

## Run it

Needs `ffmpeg` and `ffprobe` (with libx264 and libwebp) on `PATH`.

```bash
cd web && npm ci && npm run build && cd ..   # optional: embeds the web UI shell
cargo run -- doctor
UNIGIF_AUTH__BOOTSTRAP_TOKEN=dev cargo run -- serve
```

With Nix (the flake builds the web UI and wraps ffmpeg for you):

```bash
nix run          # throwaway instance in a temporary directory, token "dev"
nix develop      # dev shell with Rust, Node, ffmpeg, sqlx-cli, nextest
nix build        # ./result/bin/unigif
```

Add a GIF:

```bash
curl -X POST "http://127.0.0.1:8787/api/v1/gifs?tags=cat,funny&source=telegram&source_id=42" \
  -H "Authorization: Bearer dev" \
  -H "Content-Type: image/gif" \
  --data-binary @cat.gif
```

Sending the same bytes (or the same `source_id`) again returns `200` with `"duplicate": true` and merges any new tags. `?wait=<seconds>` (default 10) bounds how long the call waits for processing; otherwise you get `status: "processing"` and can poll `GET /api/v1/gifs/{id}`.

Bulk-import an existing folder:

```bash
cargo run -- import ~/gifs --tags-from-folders
```

## Configuration

Defaults, then an optional TOML file (`--config file.toml` or `UNIGIF_CONFIG`), then `UNIGIF_*` environment variables, where `__` separates nesting and `_` marks a camelCase boundary: `upload.maxSize` is `UNIGIF_UPLOAD__MAX_SIZE`.

```toml
publicUrl = "https://gif.example.com"   # used to build absolute media URLs
listen = "127.0.0.1:8787"
dataDir = "/var/lib/unigif"
cacheDir = "/var/cache/unigif"

[upload]
maxSize = "50MiB"
waitSecs = 10

[media]
keepOriginals = "alpha"   # alpha | never | always
crf = 23
longEdge = 720
concurrency = 2           # parallel ffmpeg processes
```

All keys and their defaults are in `src/config.rs`.

## Development

```bash
cargo test                        # unit + integration tests (real ffmpeg)
cargo clippy --all-targets -- -D warnings
cd web && npm run dev             # Vite dev server, proxies /api and /m to :8787
```

The integration tests synthesise their fixtures with ffmpeg, then drive the real router, SQLite and worker pool.

## Licence

[MIT](LICENSE)
