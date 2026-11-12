# unigif

A self-hosted, hashtag-first GIF library. Every GIF carries hashtags; new ones land in **Unsorted** and leave it the moment they get their first hashtag. GIFs are stored as small H.264 MP4s and served in whatever format a client needs, through a documented HTTP API with scoped keys.

The full design is in [`PLAN.md`](PLAN.md). **Status:** milestones M0 to M8 are implemented: storage core, auth and API, web UI, mobile layout and installable PWA, on-demand formats and the share page, the NixOS module, hashtag management and live updates (M7), and the remaining gaps (M8: export, private media, settings screens, scale test and more). What is left needs the owner: deploying to the real server, checks on a real iPhone, and live searches with real KLIPY and GIPHY keys.

## What works today

- **Quick add** (`POST /api/v1/gifs`): a raw body, multipart, or `{"url": …}` (fetched server-side with SSRF protection). Hashes, deduplicates (by content and by `source` + `source_id`), probes with ffprobe, enforces size / dimension / duration / frame limits, then transcodes in a background worker pool that survives restarts.
- **Stored variants:** `full.mp4` (H.264, no audio, ≤ 720 px long edge, `+faststart`; a plain remux when the source already qualifies, and for an H.264 source re-encoded only for its audio or frame rate, whichever of the re-encode and a packet copy is smaller), `preview.mp4`, `poster.webp`, plus a ThumbHash and a dHash. Transparent GIFs keep their original and are composited on a matte colour for the MP4.
- **Media** (`GET /m/{id}/{variant}`): public, unguessable-URL, with `Range`, `ETag` and immutable caching. With `media.public = false` media and share pages need a session or a `read` key instead (hidden hashtags apply, `Cache-Control: private`); links then no longer embed or unfurl elsewhere.
- **The API** (`/api/v1`, documented in [`api/openapi.yaml`](api/openapi.yaml), served at `/api/v1/openapi.json` with a rendered reference at `/api/docs`): list and search (`q=cat funny`, `-dog`, prefix on the last word, cursor pagination), edit and tag, trash and restore (purged after 30 days, or at once with `POST /gifs/trash/empty`), the unsorted queue (`/gifs/unsorted/next`), batch edits, hashtag list / suggest / rename / merge / delete, usage pings for `sort=used`, a "convert now" warm-up (`POST /gifs/{id}/convert`), clearing the conversion cache (`DELETE /cache`), stats, and a server-sent event stream (`gif.added`, `gif.ready`, `gif.failed`, `gif.tagged`, `gif.deleted`).
- **Auth:** the owner logs in with a password (Argon2id) and gets a `HttpOnly` session cookie; everything else uses scoped API keys (`read`, `upload`, `tag`, `delete`, `admin`) that can also hide hashtags (`deny_tags`), carry an expiry and a rate limit. Cookie writes are CSRF-protected, logins are throttled, keys are rate-limited with `RateLimit-*` headers.
- **Web UI** (embedded in the binary, served at `/`): sign in, a virtualised masonry **Library** with hashtag search (`cat funny`, `-dog`; `/` focuses the box) and a hashtag sidebar, an **Unsorted** queue, and the **tag sheet** for sorting with the keyboard only: type a hashtag, `Space` makes a chip, `Tab` accepts a suggestion, `Enter` saves and moves to the next GIF, `Alt+1…9` toggles a recent hashtag, `Alt+0` repeats the last GIF's hashtags, `Alt+←/→` goes back or skips, `Shift+Delete` trashes (with Undo), `Esc` closes. Upload by button, drag and drop, or pasting a file or link; a detail view per GIF with an editable description (the tile's alt text) and stored-against-original size. The Library sorts by Newest, Most used (Copy and Share count as uses) or Oldest. Select mode with Shift-click ranges and a rubber band, batch hashtags and trash, a Hashtags page (rename, merge, delete), a Trash page (restore, Empty trash), API keys, and Settings with password change, signed-in devices and Clear cache. Lists update live over the event stream.
- **Search** (the Search tab, `GET /api/v1/search`): look up GIFs at **KLIPY** and **GIPHY** (trending when the box is empty) and save any result to the library with one click, or select several and save them together, optionally with hashtags; without them it lands in Unsorted. The server makes the calls, so API keys never reach the browser. Enter them in **Settings → GIF search**, or in the config file. A result is added through the ordinary quick add by URL, so it is deduplicated (by content and by provider + id), size-limited and SSRF-checked like any other URL. **Tenor** is not offered: Google shut its developer API down on 30 June 2026.
- **CLI:** `serve`, `doctor` (including whether this ffmpeg can decode animated WebP), `backup [--dir D] [--keep N]`, `export <file.zip>`, `import <dir|file.zip> [--tags-from-folders]`, `gc` (one janitor pass now), `user add|passwd`, `key create|list|revoke`.

## Run it

Needs `ffmpeg` and `ffprobe` (with libx264 and libwebp) on `PATH`.

```bash
cd web && npm ci && npm run build && cd ..   # optional: builds the web UI that the binary embeds
cargo run -- doctor
cargo run -- user add timur                      # prompts for a password
cargo run -- key create --name laptop --preset full   # prints a key once
cargo run -- serve
```

With Nix (the flake builds the web UI and wraps ffmpeg for you):

```bash
nix run          # throwaway instance in a temporary directory; log in as dev / dev-password
nix develop      # dev shell with Rust, Node, ffmpeg, sqlx-cli, nextest
nix build        # ./result/bin/unigif
```

## Deploy on NixOS

The flake exports `nixosModules.default` (`services.unigif`): a hardened systemd service, an optional nginx virtual host with ACME, a database snapshot timer, and a `unigif-ctl` helper. A pre-migration snapshot is taken automatically. See [`docs/deployment.md`](docs/deployment.md).

```nix
{
  imports = [ inputs.unigif.nixosModules.default ];
  services.unigif = {
    enable = true;
    publicUrl = "https://gif.example.com";
    nginx.enable = true;
    backup.enable = true;
    initialOwner = { username = "timur"; passwordFile = config.age.secrets.unigif-owner.path; };
  };
}
```

Add a GIF:

```bash
curl -X POST "http://127.0.0.1:8787/api/v1/gifs?tags=cat,funny&source=telegram&source_id=42" \
  -H "Authorization: Bearer ugf_…" \
  -H "Content-Type: image/gif" \
  --data-binary @cat.gif
```

Or let the server fetch it: `-H "Content-Type: application/json" -d '{"url": "https://example.com/cat.gif", "tags": ["cat"]}'`.

Sending the same bytes (or the same `source_id`) again returns `200` with `"duplicate": true` and merges any new tags. `?wait=<seconds>` (default 10) bounds how long the call waits for processing; otherwise you get `status: "processing"` and can poll `GET /api/v1/gifs/{id}`.

Every GIF is stored as MP4. For a client that needs a real GIF, the object lists `media.gif`, `media.preview_gif`, `media.webm` and `media.webp`: each is converted the first time its URL is fetched, then served from a size-bounded cache (`cache.maxSize`, least recently used files go first). `page_url` (`/g/{id}`) is a public page with OpenGraph tags, so a pasted link unfurls in chats; `/g/{id}.mp4`, `.gif`, `.webm` and `.webp` redirect to the file.

Look things up:

```bash
curl -H "Authorization: Bearer ugf_…" "http://127.0.0.1:8787/api/v1/gifs?q=cat+funny&sort=used&limit=30"
curl -H "Authorization: Bearer ugf_…" "http://127.0.0.1:8787/api/v1/tags?sort=popular&limit=20"
```

Keys go in the `Authorization` header (or `X-API-Key`); a key in a URL is refused. Presets for the key screen: **read_only** (friends, your social backend), **uploader** (a hook that can add but not browse), **picker** (`read upload`), **full**.

Bulk-import an existing folder:

```bash
cargo run -- import ~/gifs --tags-from-folders
```

Move a library to another instance: `unigif export library.zip` writes each GIF's best file (the kept original, else `full.mp4`) with a `manifest.json` of hashtags, descriptions, sources, dates and usage; `unigif import library.zip` on the other side restores all of it. Trashed GIFs are left out.

On an iPhone, an iOS Shortcut can send GIFs from the share sheet: see [`docs/ios-shortcut.md`](docs/ios-shortcut.md).

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

[auth]
sessionDays = 30
trustedProxies = ["127.0.0.1"]        # whose X-Forwarded-For is believed (login throttling)
[auth.initialOwner]                   # creates the first user when there is none
username = "timur"
passwordFile = "/run/secrets/unigif-owner-password"

[rateLimit]
readPerMin = 120                      # per key; a key's own limit replaces both
uploadPerMin = 30

[cors]
origins = ["*"]                       # browser origins for key-authenticated calls

[fetch]
allowPrivate = false                  # quick add by URL never reaches private addresses
timeoutSecs = 30

[trash]
retentionDays = 30

[cache]
maxSize = "2GiB"          # on-demand conversions; safe to delete the directory at any time

[search]                              # keys entered in Settings win over these
giphyApiKey = "…"
klipyApiKey = "…"
rating = "pg-13"                      # GIPHY content rating: g | pg | pg-13 | r

[media]
public = true             # false: media and share pages need a login or a read key
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
cd web && npm run check           # svelte-check
```

End-to-end tests (Playwright, driving a real server and ffmpeg in a throwaway data directory) need the UI built and embedded first:

```bash
cd web && npm run build && (cd .. && cargo build) && npx playwright install chromium && npm run e2e
```

Scale checks, on demand (50 000 GIF rows, no media):

```bash
cargo test --release --test scale -- --ignored --nocapture   # list and search p95 under 50 ms
cd web && E2E_SCALE=1 npm run e2e -- --project scale          # the grid scrolls with a bounded DOM
```

`nix flake check` runs the same gates as CI: the package build (with the cargo tests), rustfmt, clippy, svelte-check with the JavaScript budget, and the NixOS VM test.

The integration tests synthesise their fixtures with ffmpeg, then drive the real router, SQLite and worker pool.

## Licence

[MIT](LICENSE)
