# Unigif — implementation plan

A self-hosted, hashtag-first GIF library with a scoped-key API, MP4-based storage, a keyboard-driven sorting flow, a mobile UI, and first-class NixOS packaging. Built so that Telegram clients, bots, and your own social network can use it as a GIF provider.

Status: **M0, M1, M2 and M3 are implemented** (see section 13 for what that covers and where the code departs from this text). Stack, media visibility, tenancy, originals policy, and licence are confirmed by the owner (section 14). Items marked **(verify)** are facts I could not fully confirm during research and must be checked when that part is built.

---

## 1. Decisions at a glance

| Area | Decision | Why |
|---|---|---|
| Backend | Rust, single binary, `axum` on `tokio` | One self-contained binary is the easiest thing to package and harden on NixOS; ffmpeg is driven as a subprocess |
| Database | SQLite (WAL mode) through `sqlx` | One owner, one host; no extra service to run or back up. Queries are plain runtime `sqlx::query` calls covered by integration tests (see 13.1) |
| Media engine | `ffmpeg` / `ffprobe` (`ffmpeg-headless` on NixOS) | Does every conversion needed, both directions |
| Stored format | H.264 MP4, no audio, `+faststart` | Same format Telegram uses for GIFs, plays everywhere, typically several times smaller than GIF |
| Other formats | GIF / WebM / animated WebP generated on demand, kept in an evictable cache | Space is spent only on formats somebody actually asks for |
| Frontend | Svelte 5 + Vite SPA, embedded into the Rust binary | The tagging flow is keyboard- and state-heavy; an SPA keeps it instant. Still one deployable artifact |
| Mobile | Responsive PWA (installable, Android share target) | No app store, one codebase |
| Login | Username + password (Argon2id), server-side sessions | Simple, no third party |
| API auth | `Authorization: Bearer ugf_…` keys with scopes | Read-only keys for friends, upload-only key for the Telegram hook |
| Media URLs | Public by unguessable ID, immutable caching | Required for Telegram and social embeds, which fetch without headers. Listing and search always need a key |
| Tenancy | One library per instance | Friends consume your library; they do not get their own |
| Packaging | Nix flake: package, NixOS module, dev shell, VM test | Requested |
| Licence | MIT | Chosen by the owner |

---

## 2. Goals and non-goals

**Goals**

1. Every GIF carries hashtags. New GIFs land in **Unsorted** and leave it the moment they get their first hashtag.
2. Sorting one GIF takes a click, a few keystrokes, and Enter. Sorting fifty takes no mouse at all.
3. A stable, documented, versioned HTTP API with scoped keys, including a one-request "quick add".
4. Store MP4, serve whatever the consumer needs.
5. A plain dark UI with white as the only accent, usable on a phone.
6. Deployable on NixOS with `services.unigif.enable = true`.

**Non-goals for v1**

- Multiple libraries, public sign-up, social features.
- A public GIF search engine or trending feed across users.
- AI auto-tagging (the schema leaves room for it later).
- Native mobile apps.
- Any Telegram integration (bot, sync daemon, client mods). Deferred by the owner. v1 delivers the storage service and the API those will call; section 11 keeps short notes for later.

**Vocabulary**

- **GIF**: one library item, whatever its file format.
- **Variant**: one encoded file of a GIF (`full.mp4`, `preview.mp4`, `poster.webp`, `full.gif`, …).
- **Unsorted**: a GIF with zero hashtags. Derived, never set by hand.
- **Quick add**: a single authenticated request that turns bytes or a URL into a library item.

---

## 3. Similar apps and what to borrow

No existing project combines tag-first triage, a scoped API, and MP4 storage, which is the gap Unigif fills. These are the references worth opening side by side while building.

### 3.1 GIF providers (grid, search, API shape)

| App | Borrow |
|---|---|
| [GIPHY](https://giphy.com) | Masonry grid on a black background; the content supplies all the colour. Closest visual match to "dark with white" |
| [Tenor](https://tenor.com) | Search bar with related-term chips under it; each result exposes several sizes and formats (`mp4`, `tinymp4`, `gif`, …). The API was shut down on 30 June 2026, the site still works |
| [Klipy](https://klipy.com) | The current Tenor-style API. Each item is a set of sizes, each size a set of formats (gif, webp, jpg, mp4, webm). Unigif's `media` object follows the same idea so a provider adapter is trivial |
| Telegram GIF panel, Discord GIF picker | The consumers. Dense mosaic, muted autoplay, a "saved" tab. Unigif's preview variant is sized for exactly this |

### 3.2 Sorting and tagging UX

| App | Borrow |
|---|---|
| [Raindrop.io](https://raindrop.io) | The **Unsorted** inbox concept, and its tag autocomplete grouped into "recently used" and "all other" tags |
| [Eagle](https://eagle.cool) | Press `T` to open a tag input; panel sections for **Recent** and **Starred** tags; a marker that distinguishes "will create a new tag" from "existing tag" |
| [Hydrus Network](https://hydrusnetwork.github.io/hydrus/) | The archive/delete filter: a full-screen, keyboard-only triage loop with keys for keep, skip, go back, and delete. Ugly UI, excellent workflow. This is the model for "Proceed to next unsorted" |
| [Szurubooru](https://github.com/rr-/szurubooru), Danbooru | Booru-style tag input with usage counts in the autocomplete; tag rename and merge tools |

### 3.3 Visual language (dark, monochrome, simple)

| App | Borrow |
|---|---|
| [Linear](https://linear.app) | Monochrome dark surfaces separated by hairline borders, no shadows; keyboard-first; the Triage inbox |
| [Vercel Geist](https://vercel.com/geist) | A pure black-and-white token set: white primary button with black text, grey scale for everything else |
| [Cosmos](https://www.cosmos.so), [mymind](https://mymind.com), [Are.na](https://www.are.na) | Media collections with almost no chrome |
| [Immich](https://immich.app) | A self-hosted media app with a dark responsive web UI and a mobile layout with a bottom bar. Also a good NixOS module to read (`services.immich`) |

### 3.4 Self-hosted prior art

| Project | Notes |
|---|---|
| [Gifable](https://github.com/pietvanzoen/gifable) | Self-hosted GIF library manager. Closest in intent; small project. I could not open the repo during research **(verify)** |
| [Meme Search](https://github.com/neonwatty/meme-search) | Self-hosted meme search with AI descriptions. Reference for a later auto-tagging phase |
| [evoL/gif](https://github.com/evoL/gif) | CLI GIF library with tags and URLs, in Go |
| [PictShare](https://github.com/HaschekSolutions/pictshare) | Self-hosted image/GIF/MP4 host with URL-driven on-the-fly conversion |
| [Zipline](https://github.com/diced/zipline), [Chibisafe](https://github.com/chibisafe/chibisafe) | Upload dashboards with API tokens; reference for the key-management screen |

---

## 4. Architecture

```
                    ┌──────────────────────────── NixOS VPS ────────────────────────────┐
 Browser / PWA ───► │ nginx (TLS, ACME) ──► unigif (Rust, 127.0.0.1:8787)               │
 Telegram bot  ───► │                        ├─ /            embedded SPA               │
 Client mod    ───► │                        ├─ /api/v1/*    JSON API                   │
 Social network ──► │                        ├─ /m/{id}/*    media (Range, immutable)   │
                    │                        ├─ /g/{id}      share page (OpenGraph)     │
                    │                        ├─ worker pool ──► ffmpeg / ffprobe        │
                    │                        ├─ SQLite   /var/lib/unigif/unigif.db      │
                    │                        ├─ media    /var/lib/unigif/media/         │
                    │                        └─ cache    /var/cache/unigif/  (evictable)│
                    └───────────────────────────────────────────────────────────────────┘
```

One process. Background work (transcoding, cache eviction, trash purge) runs in an in-process worker pool (tokio tasks) backed by a `jobs` table, so a restart resumes unfinished work.

### 4.1 Repository layout

```
unigif/
├─ flake.nix  flake.lock
├─ LICENSE               # MIT
├─ Cargo.toml  Cargo.lock
├─ nix/
│  ├─ package.nix        # buildNpmPackage (web) + rustPlatform.buildRustPackage (server)
│  ├─ module.nix         # services.unigif
│  └─ vm-test.nix        # NixOS test: boot, upload, tag, fetch mp4 and gif
├─ src/
│  ├─ main.rs            # CLI: serve | user | key | import | export | gc | doctor
│  ├─ config.rs          # env + TOML, validation
│  ├─ db/                # pool setup, query modules
│  ├─ auth/              # passwords, sessions, API keys, scopes, rate limits
│  ├─ media/             # probe, transcode, variants, cache, disk layout
│  ├─ ingest/            # upload, URL fetch (SSRF-safe), dedupe, jobs
│  ├─ tags/              # normalisation, suggestions
│  ├─ api/               # router, handlers, extractors, problem+json errors
│  └─ web.rs             # embedded web/dist, SPA fallback
├─ migrations/           # SQL migrations, embedded with sqlx::migrate!
├─ tests/                # integration tests (API, media pipeline)
├─ api/openapi.yaml      # written first; source of truth for the API
├─ web/                  # Svelte 5 + Vite + TypeScript
├─ testdata/             # fixture GIFs and MP4s
└─ docs/
```

A single crate to start. Split into a workspace only if compile times or a second binary (for example a future Telegram daemon) make it worthwhile.

### 4.2 Libraries

- Server: `axum` and `tower-http` (routing, body limits, compression, CORS), `tokio` (runtime, `tokio::process` for ffmpeg), `sqlx` with bundled SQLite (queries and migrations; runtime-checked, no query macros), `serde`, `clap` (CLI), `figment` (TOML + environment config), `tracing` (structured logs).
- Auth and crypto: `argon2`, `sha2`, `rand`, `subtle` (constant-time comparison), `governor` (rate limiting).
- Media helpers: `thumbhash`, `infer` (file-type detection from content). All decoding and encoding stays in ffmpeg: the ThumbHash and dHash inputs are raw pixels that ffmpeg scales from the poster, so the `image` crate is not needed.
- Other: `reqwest` with a custom DNS resolver for the SSRF-safe URL fetch, `rust-embed` to embed the built web UI, `unicode-normalization` for hashtags.
- Web: Svelte 5, Vite, TypeScript, Lucide icons. No CSS framework: a single tokens file plus component styles. No web fonts fetched from third parties.

### 4.3 CLI

| Command | Purpose |
|---|---|
| `unigif serve` | Run the server |
| `unigif user add <name>` / `passwd <name>` | Create the owner, reset a password |
| `unigif key create --name … --scopes read` | Mint a key without the UI |
| `unigif import <dir> [--tags-from-folders]` | Bulk-import an existing GIF folder |
| `unigif export <file.zip>` | Media plus a JSON manifest, for portability |
| `unigif gc` | Run cache eviction and trash purge now |
| `unigif doctor` | Check that ffmpeg has the needed encoders, that paths are writable, and that the DB is healthy |

---

## 5. Data model

SQLite, timestamps as Unix milliseconds, foreign keys on.

```sql
CREATE TABLE users (
  id            INTEGER PRIMARY KEY,
  username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
  password_hash TEXT NOT NULL,                 -- Argon2id, PHC string
  prefs         TEXT NOT NULL DEFAULT '{}',    -- JSON: proceed_to_next, sort order, …
  created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
  token_hash   BLOB PRIMARY KEY,               -- sha256 of the cookie value
  user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  user_agent   TEXT,
  ip           TEXT,
  created_at   INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  expires_at   INTEGER NOT NULL
);

CREATE TABLE api_keys (
  id           TEXT PRIMARY KEY,               -- 8-char public key id, part of the token
  name         TEXT NOT NULL,                  -- "Alex (read-only)", "Telegram Desktop"
  secret_hash  BLOB NOT NULL,                  -- sha256 of the secret part
  scopes       TEXT NOT NULL,                  -- space-separated, e.g. "read upload"
  deny_tags    TEXT NOT NULL DEFAULT '[]',     -- GIFs carrying any of these are invisible to the key
  rate_limit   INTEGER,                        -- requests/minute override
  created_by   INTEGER NOT NULL REFERENCES users(id),
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER,
  last_used_at INTEGER,
  revoked_at   INTEGER
);

CREATE TABLE gifs (
  id             TEXT PRIMARY KEY,             -- 12-char base58, random
  status         TEXT NOT NULL,                -- processing | ready | failed
  sha256         BLOB NOT NULL UNIQUE,         -- of the uploaded bytes
  phash          INTEGER,                      -- 64-bit dHash of the poster, for later near-duplicate detection
  width          INTEGER,
  height         INTEGER,
  duration_ms    INTEGER,
  has_alpha      INTEGER NOT NULL DEFAULT 0,
  original_mime  TEXT NOT NULL,
  original_kept  INTEGER NOT NULL DEFAULT 0,
  bytes_original INTEGER NOT NULL,             -- kept even if the original is discarded, for savings stats
  bytes_stored   INTEGER NOT NULL DEFAULT 0,
  thumbhash      BLOB,                         -- ~25-byte blurred placeholder
  description    TEXT,                         -- optional alt text
  source         TEXT NOT NULL,                -- web | api | url | share | telegram | import
  source_id      TEXT,                         -- e.g. Telegram document id
  source_url     TEXT,
  added_by_key   TEXT REFERENCES api_keys(id),
  tag_count      INTEGER NOT NULL DEFAULT 0,   -- maintained in the same transaction as gif_tags
  use_count      INTEGER NOT NULL DEFAULT 0,
  last_used_at   INTEGER,
  created_at     INTEGER NOT NULL,
  sorted_at      INTEGER,                      -- when it first got a hashtag
  deleted_at     INTEGER                       -- trash
);
CREATE UNIQUE INDEX gifs_source   ON gifs(source, source_id) WHERE source_id IS NOT NULL;
CREATE INDEX        gifs_library  ON gifs(created_at DESC, id) WHERE deleted_at IS NULL;
CREATE INDEX        gifs_unsorted ON gifs(created_at DESC, id) WHERE tag_count = 0 AND deleted_at IS NULL;

CREATE TABLE tags (
  id           INTEGER PRIMARY KEY,
  name         TEXT NOT NULL UNIQUE,           -- normalised, without '#'
  gif_count    INTEGER NOT NULL DEFAULT 0,
  last_used_at INTEGER,                        -- drives "recent hashtags"
  created_at   INTEGER NOT NULL
);

CREATE TABLE gif_tags (
  gif_id     TEXT    NOT NULL REFERENCES gifs(id) ON DELETE CASCADE,
  tag_id     INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (gif_id, tag_id)
) WITHOUT ROWID;
CREATE INDEX gif_tags_by_tag ON gif_tags(tag_id, gif_id);

CREATE TABLE variants (
  gif_id         TEXT NOT NULL REFERENCES gifs(id) ON DELETE CASCADE,
  name           TEXT NOT NULL,                -- full.mp4, preview.mp4, poster.webp, original.gif, full.gif, …
  tier           TEXT NOT NULL,                -- stored | cache
  bytes          INTEGER NOT NULL,
  width          INTEGER,
  height         INTEGER,
  created_at     INTEGER NOT NULL,
  last_access_at INTEGER,
  PRIMARY KEY (gif_id, name)
) WITHOUT ROWID;

CREATE TABLE jobs (
  id         INTEGER PRIMARY KEY,
  kind       TEXT NOT NULL,                    -- transcode | purge | evict
  gif_id     TEXT,
  state      TEXT NOT NULL,                    -- queued | running | done | failed
  attempts   INTEGER NOT NULL DEFAULT 0,
  error      TEXT,
  run_after  INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);
```

### 5.1 Hashtag rules

- Stored without `#`, displayed with it.
- Normalised with Unicode NFKC and case folding, so `#Cat`, `#CAT`, and `#cat` are one tag. Cyrillic and other scripts work.
- Allowed: letters, digits, `_`. A space or comma ends a tag; use `_` for multi-word tags (`#funny_cat`).
- 1 to 64 characters per tag, at most 32 tags per GIF.
- A tag row is created the first time it is used and removed by a janitor when `gif_count` reaches 0.

### 5.2 Search semantics

- `q=cat funny` means GIFs that have both tags. The last token is matched as a prefix so results update while typing.
- `-tag` excludes. `unsorted=true` restricts to GIFs without tags.
- Order: `created_at desc` by default; `sort=used` orders by `use_count` then recency, which is what a GIF picker wants.
- With ~10⁵ GIFs this is an indexed join; no full-text engine is needed.

---

## 6. Media pipeline

### 6.1 Ingest

1. **Receive** into `tmp/` while hashing (SHA-256) and enforcing the size cap (default 50 MiB).
2. **Deduplicate**: if the hash or `(source, source_id)` already exists, return the existing item with `"duplicate": true`. If that item is in the trash, restore it.
3. **Probe** with `ffprobe`: container, codec, dimensions, frame count, duration, pixel format. Reject anything over the limits (default 4096×4096, 60 s, 3000 frames) before decoding it, to stop decompression bombs.
4. **Insert** the row with `status = processing` and enqueue a job. The item appears in Unsorted immediately with a placeholder.
5. **Transcode** (worker):
   - Input is GIF, WebP, APNG, or a video that does not meet the target: encode to H.264 MP4.
   - Input is already H.264 / yuv420p / no audio / within the size limit (this is every Telegram GIF): remux only (`-c copy -an -movflags +faststart`). No quality loss, done in milliseconds.
6. **Derive** `preview.mp4`, `poster.webp`, the ThumbHash, and the dHash.
7. **Verify** the outputs with `ffprobe`, move them into `media/`, set `status = ready`, then apply the originals policy.

Quick add waits up to `?wait=` seconds (default 10) for step 7, so most callers get a finished item back in one request. If it is not ready in time, the response is the same object with `status: "processing"`.

### 6.2 Stored variants

| Variant | Spec | Used by |
|---|---|---|
| `full.mp4` | H.264 High, yuv420p, CRF 23, preset `slow`, no audio, long edge ≤ 720 px, never upscaled, even dimensions, source frame timing preserved up to 30 fps, `+faststart` | Detail view, sending, API default |
| `preview.mp4` | Same codec, height ≤ 200 px, CRF 28 | Grids and pickers |
| `poster.webp` | One frame from ~10 % in, same size as the preview | Placeholders, reduced-motion mode, link previews |
| `original.*` | The uploaded bytes | Only when the originals policy keeps them |

All numbers are settings.

**Originals policy** (`media.keepOriginals`):

- `alpha` (default): discard the original once the MP4 is verified, unless the source has transparency. H.264 cannot carry alpha, so transparent GIFs keep their original and the MP4 is composited on a configurable matte colour (default black).
- `never`: always discard.
- `always`: keep everything (largest; lossless round-trip).

### 6.3 On-demand variants

| Request | Produced from | Notes |
|---|---|---|
| `full.gif` | `original.gif` if kept, else `full.mp4` | ≤ 480 px wide, 15 fps, two-pass palette |
| `preview.gif` | same | ≤ 220 px wide, 10 fps |
| `full.webm` | `full.mp4` | VP9 |
| `full.webp` | original if kept, else `full.mp4` | Animated WebP; keeps alpha when the original is available |

Rules:

- Only these names exist. There is no free-form `?w=` parameter, so nobody can fill the disk by requesting thousands of sizes.
- The first request generates the file while the client waits (a short GIF converts in about a second). Concurrent requests for the same file share one conversion (an in-flight map of shared futures). A global semaphore caps parallel ffmpeg processes (default 2). Hard timeout 30 s.
- Results go to the cache directory. A janitor evicts least-recently-accessed files when the cache exceeds `cache.maxSize` (default 2 GiB). Deleting the whole cache directory is always safe.

Reference commands (to be tuned against the fixtures):

```bash
# GIF -> MP4 (target size computed in code; shown here for a 480x270 result)
ffmpeg -nostdin -v error -i in.gif -an -c:v libx264 -preset slow -crf 23 \
  -pix_fmt yuv420p -vf "scale=480:270:flags=lanczos" -fps_mode vfr \
  -movflags +faststart full.mp4
```

```bash
# MP4 -> GIF with a per-clip palette
ffmpeg -nostdin -v error -i full.mp4 -vf \
  "fps=15,scale=480:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" \
  -loop 0 full.gif
```

ffmpeg always runs with `-nostdin`, a restricted protocol whitelist (`file` only), a wall-clock timeout, and inside the systemd sandbox described in section 10.

### 6.4 Disk layout

```
/var/lib/unigif/
  unigif.db  unigif.db-wal  unigif.db-shm
  media/7K/7Kq2mXw9Fb3T/full.mp4
  media/7K/7Kq2mXw9Fb3T/preview.mp4
  media/7K/7Kq2mXw9Fb3T/poster.webp
  media/7K/7Kq2mXw9Fb3T/original.gif        # only when kept
  tmp/                                      # in-flight uploads and finished-but-unmoved outputs, wiped at start
  work/{id}.src                             # source of a GIF that is queued or transcoding; survives restarts
  backups/                                  # nightly DB snapshots
/var/cache/unigif/
  7K/7Kq2mXw9Fb3T/full.gif                  # on-demand, evictable
```

Files under `media/` are written once and never modified, so incremental backups stay small.

### 6.5 Serving

- `GET /m/{id}/{variant}` with `Cache-Control: public, max-age=31536000, immutable`, `ETag`, and HTTP Range support (required for video on iOS Safari).
- `GET /g/{id}` is a tiny HTML page with OpenGraph and Twitter-card video tags, so a pasted link unfurls in chats. `/g/{id}.mp4` and `/g/{id}.gif` redirect to the media URL.
- Settings → Storage shows the total of `bytes_original` against `bytes_stored`: "1 240 GIFs · 310 MB stored · 2.9 GB saved".

---

## 7. Authentication and authorisation

### 7.1 Owner login

- Username and password. Argon2id hashes. No sign-up page.
- The first user is created by `unigif user add`, or automatically at first start from the NixOS option `initialOwner.passwordFile` when the users table is empty.
- Session: random 256-bit token in an `HttpOnly; Secure; SameSite=Lax` cookie, stored hashed, 30-day sliding expiry. Settings lists active sessions with a revoke button.
- CSRF: cookie-authenticated writes must send JSON (or the `X-Unigif-Request: 1` header for uploads) and pass an `Origin` check.
- Login throttling: 5 attempts per minute per IP, then exponential back-off. Client IP is read from the proxy header only when the peer is in `trustedProxies`.
- Later, optional: passkeys (WebAuthn), TOTP, OIDC.

### 7.2 API keys

Format: `ugf_<key-id:8>_<secret:32>`, base62. The `ugf_` prefix lets secret scanners recognise leaked keys. Only `sha256(secret)` is stored. The full key is shown once at creation.

| Scope | Allows |
|---|---|
| `read` | List, search, and fetch GIFs and tags |
| `upload` | Add GIFs (file, raw body, URL), with initial tags |
| `tag` | Change hashtags and descriptions; rename and merge tags |
| `delete` | Move to trash, restore |
| `admin` | Manage keys and settings. Never offered in presets |

Presets in the UI:

- **Read-only** (`read`): for friends and for your social network's backend.
- **Uploader** (`upload`): for the Telegram hook. It can add GIFs but cannot browse the library, so a key extracted from a client build leaks nothing.
- **Picker** (`read upload`): for a client mod that both shows the library and saves to it.
- **Full** (`read upload tag delete`).

Per-key options: expiry date, rate limit, and **hidden hashtags** (`deny_tags`). A friend's key with `deny_tags = ["private"]` never sees GIFs tagged `#private`, in lists, in search, or by ID.

Keys go in `Authorization: Bearer …` (or `X-API-Key`). Keys in query strings are rejected, because URLs end up in logs. `last_used_at` is updated at most once a minute.

Rate limits (token bucket per key, defaults): 120 requests/min for reads, 30/min for uploads. Responses carry `RateLimit-*` headers; over the limit returns `429` with `Retry-After`.

---

## 8. HTTP API (v1)

### 8.1 Conventions

- Base path `/api/v1`. JSON in and out, UTF-8, RFC 3339 timestamps in UTC.
- Errors use `application/problem+json`: `{ "type", "title", "status", "detail", "code" }` with stable `code` strings (`invalid_media`, `too_large`, `scope_required`, …).
- Lists are cursor-paginated: `?limit=50&cursor=…` returns `{ "items": […], "next_cursor": "…" | null }`.
- CORS: allowed for key-authenticated requests from configured origins (default `*`), never for cookie sessions.
- `api/openapi.yaml` is written before the handlers. The server serves it at `/api/v1/openapi.json` and a rendered reference at `/api/docs`.
- Breaking changes go to `/api/v2`; v1 only gains fields.

### 8.2 The GIF object

```json
{
  "id": "7Kq2mXw9Fb3T",
  "status": "ready",
  "tags": ["cat", "funny"],
  "description": null,
  "width": 480,
  "height": 270,
  "duration_ms": 2400,
  "has_alpha": false,
  "thumbhash": "1QcSHQRnh493V4dIh4eXh1h4kJUI",
  "source": { "type": "telegram", "id": "5368324170671202286" },
  "created_at": "2026-11-12T09:30:00Z",
  "sorted_at": "2026-11-12T09:31:12Z",
  "page_url": "https://gif.example.com/g/7Kq2mXw9Fb3T",
  "media": {
    "mp4":         { "url": "https://gif.example.com/m/7Kq2mXw9Fb3T/full.mp4",    "width": 480, "height": 270, "bytes": 183422 },
    "preview_mp4": { "url": "https://gif.example.com/m/7Kq2mXw9Fb3T/preview.mp4", "width": 356, "height": 200, "bytes": 41290 },
    "poster":      { "url": "https://gif.example.com/m/7Kq2mXw9Fb3T/poster.webp", "width": 356, "height": 200, "bytes": 6120 },
    "gif":         { "url": "https://gif.example.com/m/7Kq2mXw9Fb3T/full.gif",    "on_demand": true },
    "preview_gif": { "url": "https://gif.example.com/m/7Kq2mXw9Fb3T/preview.gif", "on_demand": true },
    "webm":        { "url": "https://gif.example.com/m/7Kq2mXw9Fb3T/full.webm",   "on_demand": true },
    "webp":        { "url": "https://gif.example.com/m/7Kq2mXw9Fb3T/full.webp",   "on_demand": true }
  }
}
```

A social network that needs real GIFs just uses `media.gif.url`; the file is generated the first time it is fetched.

### 8.3 Endpoints

| Method and path | Scope | Purpose |
|---|---|---|
| `POST /auth/login`, `POST /auth/logout` | none / session | Owner session |
| `GET /me` | any | Who am I: user or key name, scopes, limits |
| `GET /gifs` | `read` | List and search: `q`, `tag` (repeatable), `unsorted`, `sort`, `limit`, `cursor` |
| `GET /gifs/{id}` | `read` | One GIF |
| `POST /gifs` | `upload` | **Quick add**, three body types (8.4) |
| `PATCH /gifs/{id}` | `tag` | Set `tags` (full replace) and/or `description` |
| `POST /gifs/{id}/tags` | `tag` | `{ "add": […], "remove": […] }` |
| `DELETE /gifs/{id}` | `delete` | Move to trash (purged after 30 days) |
| `POST /gifs/{id}/restore` | `delete` | Take out of trash |
| `POST /gifs/{id}/use` | `read` | Usage ping when a GIF is sent; feeds `sort=used` |
| `GET /gifs/unsorted/next?after={id}` | `read` | Next unsorted item plus the remaining count |
| `POST /gifs/batch` | `tag` / `delete` | Apply tag changes or trash to many IDs |
| `GET /tags` | `read` | `q` (prefix), `sort=recent\|popular\|alpha`, `limit` |
| `GET /tags/suggest?selected=a,b` | `read` | Recent tags plus tags that often appear with the selected ones |
| `PATCH /tags/{name}` | `tag` | Rename |
| `POST /tags/{name}/merge` | `tag` | `{ "into": "other" }` |
| `DELETE /tags/{name}` | `tag` | Remove the tag from every GIF |
| `GET /keys`, `POST /keys`, `PATCH /keys/{id}`, `DELETE /keys/{id}` | `admin` | Key management |
| `GET /stats` | `read` | Counts, bytes stored and saved, unsorted count |
| `GET /events` | `read` | Server-sent events: `gif.added`, `gif.ready`, `gif.tagged`, `gif.deleted` |
| `GET /healthz`, `GET /readyz` | none | For the reverse proxy and monitoring |

### 8.4 Quick add

Three ways to call the same endpoint. All accept `tags`, `source`, `source_id`, `description`, and `wait`.

**Raw body** (what a client mod or script uses; nothing to assemble):

```bash
curl -X POST "https://gif.example.com/api/v1/gifs?source=telegram&source_id=5368324170671202286" \
  -H "Authorization: Bearer ugf_a1B2c3D4_…" \
  -H "Content-Type: video/mp4" \
  --data-binary @animation.mp4
```

**Multipart** (browser upload, share target): field `file`, plus `tags` and the other fields.

**By URL**:

```bash
curl -X POST "https://gif.example.com/api/v1/gifs" \
  -H "Authorization: Bearer ugf_a1B2c3D4_…" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/some.gif", "tags": ["cat"]}'
```

The server fetches the URL itself with SSRF protection: DNS is resolved first and private, loopback, and link-local ranges are refused, redirects are re-checked (maximum 3), and the size cap and timeout apply.

Responses:

| Status | Meaning |
|---|---|
| `201 Created` | New item. `status` is `ready`, or `processing` if the wait ran out |
| `200 OK` with `"duplicate": true` | Same bytes or same `source_id` already stored; returns the existing item. Tags sent with the request are merged in |
| `413`, `415`, `422` | Too large; unsupported type; not a decodable animation or over limits |
| `401`, `403`, `429` | Missing or bad key; missing scope; rate limited |

Idempotency by `source_id` matters: a Telegram hook may fire more than once for the same GIF, and it must be safe to retry on a flaky connection.

### 8.5 Provider use

Your social network's backend needs only:

- `GET /gifs?q=…&sort=used&limit=30` for search,
- `GET /gifs?sort=used` for a default "featured" page,
- `GET /tags?sort=popular&limit=20` for category chips,
- `POST /gifs/{id}/use` when a GIF is posted.

A thin Tenor/Klipy-shaped adapter (`/compat/…`) can be added later if a client library expects that response format.

---

## 9. UI design

### 9.1 Principles

1. **The GIFs are the colour.** The interface is black, greys, and white. Red appears only on destructive actions.
2. **White means "selected" or "primary".** A selected hashtag chip is white with black text; an unselected one is an outline. The primary button is white. Nothing else is.
3. **Borders, not shadows.** Surfaces are separated by 1 px hairlines.
4. **Keyboard first on desktop, thumb first on mobile.** Every sorting action has a key, and every mobile action is reachable in the lower half of the screen.
5. **No layout shift.** Every tile is sized from stored dimensions and painted with its ThumbHash before any media loads.

### 9.2 Tokens

| Token | Value | Use |
|---|---|---|
| `--bg` | `#0A0A0A` | Page |
| `--surface-1` | `#111111` | Sidebar, sheets, inputs |
| `--surface-2` | `#1A1A1A` | Hover, pressed |
| `--border` | `#262626` | Hairlines |
| `--border-strong` | `#3D3D3D` | Chip outline, input focus border |
| `--text` | `#FAFAFA` | Primary text |
| `--text-muted` | `#A1A1A1` | Secondary text, counts |
| `--text-faint` | `#6B6B6B` | Disabled and decorative only |
| `--accent` | `#FFFFFF` | Primary button, selected chip (text `#000000`) |
| `--danger` | `#FF4D4D` | Delete, errors |

- Type: system UI stack; hashtags in the system monospace stack, which gives `#tags` a distinct look for free. Sizes 13 / 14 / 16 / 20 / 28 px.
- Spacing on a 4 px grid. Radius 6 px on controls, 10 px on tiles and sheets.
- Focus: 2 px white ring with a 2 px offset, on every interactive element.
- Motion: 120 to 160 ms ease-out. With `prefers-reduced-motion`, no transitions and no autoplay: tiles show posters and play on hover or tap.
- Touch targets at least 44 × 44 px.

### 9.3 Layout

- **Desktop (≥ 1024 px)**: fixed 220 px sidebar, top bar with search and Upload, masonry grid with ~220 px columns.
- **Tablet (640 to 1023 px)**: the sidebar collapses to an icon rail.
- **Mobile (< 640 px)**: search on top, two-column grid, bottom tab bar.

```
┌────────────┬─────────────────────────────────────────────────────────┐
│ unigif     │  ⌕ Search hashtags…                 /        [ Upload ] │
│            ├─────────────────────────────────────────────────────────┤
│ Library    │  [#cat ×] [#funny ×]                    1 240 GIFs      │
│ Unsorted 41│  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐           │
│ Trash      │  │      │ │      │ │      │ │      │ │      │           │
│            │  │      │ └──────┘ │      │ └──────┘ │      │           │
│ HASHTAGS   │  └──────┘ ┌──────┐ └──────┘ ┌──────┐ └──────┘           │
│ #reaction 212 ┌──────┐ │      │ ┌──────┐ │      │ ┌──────┐           │
│ #cat     140│ │      │ │      │ │      │ │      │ │      │           │
│ #anime    97│ └──────┘ └──────┘ └──────┘ └──────┘ └──────┘           │
│ …          │                                                         │
│ Settings   │                                                         │
└────────────┴─────────────────────────────────────────────────────────┘
```

**Grid behaviour**

- Tiles play `preview.mp4` muted and looped, only while on screen (IntersectionObserver); off-screen videos are paused and unloaded.
- Hover (desktop) shows the hashtags and a copy-link button. Long-press (mobile) opens a menu: Copy link, Share, Edit hashtags, Delete.
- Click in **Library** opens the detail view. Click in **Unsorted** opens the tag sheet.
- The list is virtualised, so a library of tens of thousands stays smooth.
- Drag a file anywhere, or paste a file or URL anywhere, to upload.

### 9.4 The tag sheet (the core of the product)

Desktop: a centred modal, GIF on the left, tagging on the right.

```
┌──────────────────────────────────────────────────────────────────────┐
│ Unsorted · 3 of 41                                           Esc  ✕  │
├──────────────────────────────────┬───────────────────────────────────┤
│                                  │ RECENT                            │
│                                  │ [#cat] [#reaction] [#lol] [#no]   │
│                                  │ [#anime] [#yes] [#dog] [#work]    │
│         (GIF, looping)           │ ↻ Same as last:  #cat #lol        │
│                                  │                                   │
│                                  │ ┌───────────────────────────────┐ │
│                                  │ │ ▓#cat ×▓ ▓#funny ×▓ #typ▏     │ │
│                                  │ └───────────────────────────────┘ │
│                                  │   #typing            12           │
│                                  │   #typo               3           │
│                                  │   + create #typ                   │
│                                  │                                   │
│ ← Prev      Skip →       Delete  │ ☑ Proceed to next unsorted        │
│                                  │                      [ Save  ⏎ ]  │
└──────────────────────────────────┴───────────────────────────────────┘
```

**Flow**

1. The sheet opens with the cursor already in the input. The GIF loops at full quality.
2. **Recent** shows the 12 most recently used hashtags. Click or tap to toggle one. Tags applied in this session move to the front.
3. **Same as last** applies every hashtag of the previously saved GIF in one action. This is the fastest path when sorting a run of similar GIFs.
4. Typing filters existing hashtags (with usage counts). The last row offers to create the typed tag when it does not exist.
5. **Enter saves.** Then the sheet closes, or, if **Proceed to next unsorted** is ticked, the next unsorted GIF slides in with the cursor still in the input.

**Keys**

| Key | Action |
|---|---|
| typing | Filter suggestions. A leading `#` is optional |
| `Space` or `,` | Turn the typed text into a chip |
| `Tab`, `↓` / `↑` | Accept the top suggestion; move through suggestions |
| `Enter` | Commit any typed text, **save**, then close or go to the next GIF according to the checkbox |
| `Shift` + `Enter` | Save and do the opposite of the checkbox, once |
| `Backspace` in an empty input | Remove the last chip |
| `Alt` + `1`…`9` | Toggle the 1st to 9th recent hashtag |
| `Alt` + `0` | Same as last |
| `Alt` + `→` / `Alt` + `←` | Skip / go back, without saving |
| `Shift` + `Delete` | Move the GIF to trash (5-second Undo toast) |
| `Esc` | Close suggestions if open, otherwise close the sheet without saving |

**Rules**

- **Enter with no hashtags does not save.** The input shakes and a hint says "Add a hashtag, or Skip". A GIF without hashtags is unsorted by definition, so there is nothing to save.
- **Saving is optimistic.** The sheet moves on immediately and the request runs in the background. On failure a toast offers Retry and the GIF stays in Unsorted.
- **The next GIF is preloaded**, so moving on is instant.
- **Order** follows the Unsorted grid (newest first by default, switchable to oldest first). Skipped GIFs stay unsorted and are not shown again in the same run.
- **The checkbox is remembered** per user (server-side preference, mirrored locally). "Start sorting" in the Unsorted header opens the first GIF with the checkbox on.
- **Pasting** `#a #b #c` creates three chips.
- **End of queue**: "All sorted." with a Close button. The Unsorted badge disappears.
- The same sheet, without the queue controls, is what **Edit hashtags** opens for a GIF in the Library.

### 9.5 Mobile

```
┌──────────────────────────┐      ┌──────────────────────────┐
│ ⌕ Search hashtags…       │      │ Unsorted · 3 of 41     ✕ │
├──────────────────────────┤      ├──────────────────────────┤
│ Unsorted · 41  [ Sort ▶ ]│      │                          │
│ ┌──────────┐┌──────────┐ │      │      (GIF, looping)      │
│ │          ││          │ │      │                          │
│ │          │└──────────┘ │      ├──────────────────────────┤
│ └──────────┘┌──────────┐ │      │ [#cat][#reaction][#lol] →│
│ ┌──────────┐│          │ │      │ ↻ Same as last           │
│ │          ││          │ │      │ ┌──────────────────────┐ │
│ └──────────┘└──────────┘ │      │ │ ▓#cat ×▓ #fun▏       │ │
│                          │      │ └──────────────────────┘ │
├──────────────────────────┤      │ ◉ Next unsorted  [ Save ]│
│ Library  Unsorted  ＋  ⚙ │      ├──────────────────────────┤
│            ·41           │      │       (keyboard)         │
└──────────────────────────┘      └──────────────────────────┘
```

- **Bottom tab bar**: Library, Unsorted (with count), Add, Settings. Hashtag browsing lives in the search screen.
- **Tag sheet** is full-screen. The GIF takes at most 40 % of the height. The input row and the Save row stay pinned directly above the on-screen keyboard (via the `visualViewport` API).
- The keyboard's action key is labelled "Done" and acts as Enter. Autocapitalisation and autocorrect are off in the tag input.
- **Gestures**: swipe left to skip, right to go back, down to close.
- **Add** offers: pick files, paste a link, or (Android) receive a share.
- **Installable PWA**. On Android the manifest's `share_target` puts Unigif in the system share sheet, so a GIF can be shared into it from any app. iOS Safari does not support share targets; there the route is an iOS Shortcut that posts to the quick-add endpoint, plus the in-app file picker.
- Data use: on mobile at most six tiles play at once; the rest show posters.

### 9.6 Other screens

- **Login**: logo, username, password, one white button. Nothing else.
- **GIF detail**: the GIF, its hashtags (Edit opens the tag sheet), copy buttons for Link / MP4 / GIF / WebM, source, dimensions, stored size against original size, Delete.
- **Hashtags**: sortable table (name, count, last used) with rename, merge, delete.
- **Trash**: grid with Restore and "Empty trash".
- **Settings → API keys**:

```
┌───────────────────────────────────────────────────────────────────┐
│ API keys                                           [ New key ]    │
├───────────────────────────────────────────────────────────────────┤
│ Alex (friend)       read            last used 2 h ago    Revoke   │
│ Telegram Desktop    upload          last used 5 min ago  Revoke   │
│ My social backend   read            last used just now   Revoke   │
└───────────────────────────────────────────────────────────────────┘

New key
  Name            [ Alex (friend)            ]
  Access          (•) Read-only   ( ) Uploader   ( ) Picker   ( ) Full   ( ) Custom
  Hide hashtags   [ #private ×  …            ]
  Expires         [ Never ▾ ]
                                                   [ Create key ]

  ugf_a1B2c3D4_9fK…                      [ Copy ]
  This key is shown once. Store it now.
```

- **Settings → Storage**: GIF count, bytes stored, bytes saved, cache size with a "Clear cache" button.
- **Settings → Account**: change password, active sessions.

### 9.7 Budgets

- Initial JavaScript ≤ 150 kB gzipped.
- The grid is usable (placeholders painted) within one second on a mid-range phone.
- Saving in the tag sheet gives feedback in under 100 ms (optimistic).
- WCAG 2.1 AA contrast for all text; the hashtag input follows the ARIA combobox pattern.

---

## 10. NixOS

### 10.1 Flake outputs

| Output | Content |
|---|---|
| `packages.<system>.unigif` (also `default`) | The server binary with the web UI embedded. `web/` is built with `buildNpmPackage`, the server with `rustPlatform.buildRustPackage` from `Cargo.lock`; `ffmpeg-headless` is wrapped onto its `PATH` |
| `nixosModules.default` | `services.unigif` |
| `overlays.default` | Adds `pkgs.unigif` |
| `devShells.<system>.default` | Rust toolchain (cargo, rustc, clippy, rustfmt, rust-analyzer), `sqlx-cli`, `cargo-nextest`, Node, ffmpeg, sqlite, Playwright browsers |
| `checks.<system>.*` | `cargo test`, `clippy`, `rustfmt`, web tests, and the NixOS VM test |
| `formatter.<system>` | `nixfmt` |
| `apps.<system>.default` | `nix run` starts a local instance with a temporary data directory |

Systems: `x86_64-linux` and `aarch64-linux`.

Build notes:

- No database is needed at build time: queries are runtime-checked, so there is no `.sqlx/` metadata to keep in sync. The package's `checkPhase` runs the whole cargo test suite (real ffmpeg) instead.
- `nixpkgs` is pinned as a channel tarball (`https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz`) rather than `github:NixOS/nixpkgs/...`; it locks the same way.
- SQLite is compiled in (bundled), so the binary has no runtime library dependency beyond libc.
- Only nixpkgs is a flake input. If incremental CI builds become slow, switching the package to `crane` is a contained change.

### 10.2 Module options (`services.unigif`)

| Option | Default | Meaning |
|---|---|---|
| `enable` | `false` | |
| `package` | flake package | |
| `publicUrl` | (required) | e.g. `https://gif.example.com`; used to build absolute media URLs |
| `listen.address`, `listen.port` | `127.0.0.1`, `8787` | |
| `dataDir` | `/var/lib/unigif` | Database and media |
| `cacheDir` | `/var/cache/unigif` | On-demand variants |
| `user`, `group` | `unigif` | Static user, so file ownership is stable for backups |
| `settings` | `{}` | Free-form attribute set rendered to the config file (upload limits, CRF, originals policy, cache size, rate limits, CORS origins, trusted proxies) |
| `environmentFile` | `null` | Secrets as `UNIGIF_*` variables; works with agenix and sops-nix |
| `initialOwner.username`, `initialOwner.passwordFile` | `null` | Creates the first user when the database has none |
| `ffmpegPackage` | `pkgs.ffmpeg-headless` | |
| `nginx.enable`, `nginx.enableACME`, `nginx.forceSSL` | `false`, `true`, `true` | Creates the virtual host for `publicUrl`, sets `client_max_body_size` from the upload limit, disables request buffering for uploads |
| `openFirewall` | `false` | Only relevant without the reverse proxy |
| `backup.enable`, `backup.calendar`, `backup.keep` | `false`, `daily`, `7` | Timer that writes a consistent SQLite snapshot into `dataDir/backups` |

Target host configuration:

```nix
{
  inputs.unigif.url = "github:timursl/unigif";

  # in the host's modules:
  imports = [ inputs.unigif.nixosModules.default ];

  services.unigif = {
    enable = true;
    publicUrl = "https://gif.example.com";
    environmentFile = config.age.secrets.unigif-env.path;
    initialOwner = {
      username = "timur";
      passwordFile = config.age.secrets.unigif-owner-password.path;
    };
    settings = {
      upload.maxSize = "50MiB";
      media.keepOriginals = "alpha";
      cache.maxSize = "2GiB";
    };
    nginx.enable = true;
    backup.enable = true;
  };
}
```

### 10.3 systemd unit

- `StateDirectory`, `CacheDirectory`, `RuntimeDirectory`; `UMask=0077`.
- Hardening: `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, `PrivateDevices`, `ProtectKernelTunables`, `ProtectKernelModules`, `ProtectControlGroups`, `RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX`, `RestrictNamespaces`, `LockPersonality`, `SystemCallFilter=@system-service`, empty `CapabilityBoundingSet`.
- `MemoryMax` and `TasksMax` so a hostile file cannot make ffmpeg take the host down.
- Goal: `systemd-analyze security unigif` exposure score of 2.0 or lower, checked in the VM test.
- Migrations run automatically at start, inside a transaction, after a pre-migration DB snapshot.

### 10.4 VM test (`nix flake check`)

Boots a NixOS VM with the module and nginx, then:

1. logs in as the initial owner,
2. creates an `upload` key and a `read` key,
3. quick-adds a fixture GIF and checks that `full.mp4` is H.264 and smaller than the source,
4. checks the item is listed as unsorted, tags it, checks it left Unsorted,
5. fetches `full.gif` and checks it decodes,
6. checks the `read` key cannot upload (`403`) and that `deny_tags` hides a tagged item,
7. restarts the service and checks everything is still there.

### 10.5 Backups

- The database snapshot timer makes a consistent copy; `media/` is immutable.
- Back up `dataDir` with whatever the host already uses (restic, borg). `cacheDir` is never backed up.
- Restore is documented and exercised in the VM test: stop, replace `dataDir`, start.

---

## 11. Telegram integration (deferred, notes only)

Not scheduled. The owner will pick this up after the storage service is running. Nothing in M0 to M7 depends on it. These notes are kept so the research is not lost.

How Telegram works here, confirmed during research:

- Telegram "GIFs" are silent H.264 MP4 files, so adding one to Unigif is a remux, not a transcode.
- The saved list is read with `messages.getSavedGifs`, changed with `messages.saveGif`, and changes are announced by the `updateSavedGifs` update.
- Telegram caps saved GIFs (client defaults: 200, or 400 with Premium) and drops the oldest when the cap is passed. Unigif removes that ceiling.

Three possible routes, from least to most effort.

### Route A: a bot (no client changes)

- **Save**: forward or send a GIF to your private bot. It downloads the file and calls quick add with `source=telegram`. Access is limited to your Telegram user ID.
- **Send**: inline mode. Typing `@yourbot cat` in any chat on any Telegram client shows your Unigif results for `#cat`.
- Constraint: inline results given by URL are limited to 1 MB per the Bot API client documentation **(verify against the official Bot API page)**. So the bot uploads each MP4 to Telegram once (to a private storage chat), stores the returned `file_id` (in a small `external_refs` table added by migration at that point), and answers inline queries with cached-file results. This also makes results appear instantly.
- Bot downloads have a file-size limit on the hosted Bot API **(verify; commonly 20 MB)**, comfortably above normal GIF sizes.

### Route B: saved-GIFs sync daemon (the literal "Add to my GIFs" behaviour, on every client)

- A small MTProto client logged in as you listens for `updateSavedGifs`, calls `messages.getSavedGifs`, compares document IDs with `source_id` values already in Unigif, downloads the new ones, and quick-adds them.
- Result: pressing **Add to my GIFs** in the official app on any device puts the GIF into Unsorted within seconds. No mod needed.
- Optional: after a GIF is mirrored, unsave the oldest ones from Telegram so the cap is never hit.
- Cost: a Telegram user session lives on the server. It is stored in the state directory with mode 0600, the feature is off by default, and it gets its own systemd unit with its own sandbox.

### Route C: client mods

- **Telegram Desktop fork**: call quick add from the code path that toggles a saved GIF (`Telegram/SourceFiles/api/api_toggling_media.cpp` is where the save/unsave helpers live; the exact function must be confirmed against the current tree **(verify)**), and add a Unigif tab to the GIF panel that reads `GET /gifs?q=…` and uses `preview_mp4`.
- **Android**: exteraGram has a built-in Python plugin engine (Python 3.11, hooks on app methods, pure-Python dependencies only). A plugin hooks the save-GIF action and calls quick add. AyuGram-compatible plugin tooling exists in the community. Desktop plugin engines are still in development in forks, so a desktop plugin cannot be relied on yet **(verify state at the time)**.
- These clients ship the **Uploader** or **Picker** key, never a full one.

What any of these will need from v1, all already in the design: raw-body quick add, idempotency by `source_id`, the upload-only scope, small preview variants, and the event stream so the Unsorted badge updates the moment a GIF arrives.

---

## 12. Security checklist

- Uploads: size cap while streaming, type detection from content rather than the filename or header, probe before decode, dimension / duration / frame limits, ffmpeg timeout, restricted ffmpeg protocols, systemd sandbox and memory cap.
- URL fetch: private-range blocking after DNS resolution, redirect re-validation, size and time limits, no credentials forwarded.
- Paths: files are addressed only by generated IDs and a fixed list of variant names; no user-controlled path segments.
- Auth: Argon2id, hashed session tokens and key secrets, constant-time comparison, login throttling, CSRF defence, strict cookie flags.
- Keys: scopes checked in one middleware, `deny_tags` applied in the query layer so it cannot be bypassed by ID lookup, expiry and revocation honoured on every request.
- Headers: `Content-Security-Policy` (self only, media from self), `X-Content-Type-Options: nosniff`, `Referrer-Policy`, HSTS at the proxy. Media is served with an explicit content type and never as HTML.
- Logs: structured, no key material, no cookie values.
- Privacy: media URLs are unguessable (12 base58 characters, about 70 bits) but public. Setting `media.public = false` requires a key or session for media too, at the cost of embeds no longer working.

---

## 13. Milestones

Sizes are relative (S / M / L), not time estimates.

| # | Milestone | Size | Done when |
|---|---|---|---|
| M0 ✅ | **Skeleton**: MIT `LICENSE`, flake, dev shell, CI, Rust server serving an embedded empty SPA, `nix build` and `nix run` work | S | A fresh clone builds and starts with one command |
| M1 ✅ | **Storage core**: schema, migrations, ingest pipeline, stored variants, dedupe, media serving, `import` and `doctor` | L | `curl` quick add of a GIF yields `full.mp4`, `preview.mp4`, `poster.webp`; Range requests work; the same file twice returns `duplicate` |
| M2 ✅ | **Auth and API**: login, sessions, API keys with scopes and `deny_tags`, rate limits, all section 8 endpoints, OpenAPI served | L | Contract tests pass against `openapi.yaml`; a `read` key gets `403` on upload |
| M3 ✅ | **Web UI**: login, Library, search, Unsorted, tag sheet with every key in 9.4, upload by drag and paste, detail view | L | An end-to-end test sorts three GIFs using only the keyboard |
| M4 ✅ | **Mobile and PWA**: responsive layout, bottom bar, full-screen tag sheet, gestures, installable, Android share target | M | The same sorting test passes in a phone viewport; a shared GIF lands in Unsorted |
| M5 ✅ | **Conversion**: on-demand GIF / WebM / WebP, cache with eviction, share page with OpenGraph, storage stats | M | `full.gif` is generated once and served from cache after; the cache stays under its limit |
| M6 | **NixOS**: module, hardening, nginx integration, backup timer, VM test, deployment guide | M | The VM test in 10.4 passes; the service runs on the VPS |
| M7 | **Polish**: hashtag management (rename, merge), trash, batch select, co-occurrence suggestions, live updates over SSE, API key screen refinements | M | Renaming or merging a hashtag updates every GIF; a GIF added through the API appears in an open Unsorted tab without a reload |

### 13.1 Implementation notes (M0 to M5)

Built in this order, and verified by `cargo test` (46 unit tests and 68 integration tests, with real ffmpeg), `cargo clippy -D warnings`, a curl run against the real binary, `nix build`, and for M3 to M5 `svelte-check` plus the Playwright suite in `web/e2e`.

**M0, skeleton: done.** MIT `LICENSE`, `Cargo.toml`, flake (`packages`, `overlays`, `devShells`, `apps` for `nix run`, `formatter`, `checks`), `.github/workflows/ci.yml` (Rust, web with a 150 kB JS budget, Nix), Svelte 5 + Vite shell embedded with `rust-embed`, and a `build.rs` that creates `web/dist` so a plain `cargo build` works before the UI has been built.

**M1, storage core: done**, with these departures from the text above:

- **Interim auth** (M1 only): one static bearer token. Replaced in M2; see below.
- **Quick add by URL** returned `501` in M1; it landed with M2.
- **On-demand variants** (`full.gif`, `full.webm`, …) and `/g/{id}` are M5; those names answer `404` until then, and the GIF object only lists the three stored variants.
- **Pre-decode checks** are three, not two: ffprobe limits (dimensions, duration, frame count), then a first-frame decode. `ffprobe` reads headers only, so a file with a plausible header and garbage inside would otherwise pass and fail later in the worker.
- **Forced demuxers.** For GIF, MP4/MOV and Matroska/WebM the sniffed type picks ffmpeg's demuxer with `-f`, so a crafted file cannot select an exotic one (playlists, concat scripts).
- **Alpha detection** looks at the alpha plane (`alphaextract` + `signalstats`), because ffmpeg decodes every GIF as `bgra` whether or not it has transparency.
- **Sources of unfinished jobs** live in `work/`, not `tmp/` (which is wiped at start), so a restart resumes them. A failed item is dropped and replaced when the same bytes are uploaded again.
- **Animated WebP input** depends on the ffmpeg build: ffmpeg 6.x decodes only the first frame, newer releases decode the animation. `doctor` does not check this yet.
- Deferred to later milestones: pre-migration DB snapshots (M6), the "keep the smaller of re-encode and original" rule for video sources (M7 polish), `export`, `gc`, `user` and `key` subcommands.

**M2, auth and API: done**, with these departures from the text above:

- **No bootstrap token any more.** The first owner comes from `unigif user add <name>` or from `auth.initialOwner` (`username` + `passwordFile`, used only while the users table is empty). `unigif key create` mints keys without a browser; it also has `key list` and `key revoke`.
- **One authentication middleware.** `auth::middleware` resolves a bearer key or a session cookie, applies the CSRF rules and the rate limit, and attaches a `Principal`; handlers call `principal.require(scope)`. A malformed `Authorization` header is `401` rather than "anonymous"; a stale cookie is just anonymous. Keys in a query string (`api_key=…`, or any value starting `ugf_`) are `400`.
- **Rate limits** are an in-memory token bucket per (key, class): `read` for everything and `upload` for `POST /gifs`; a key's own `rate_limit` replaces both defaults. The owner's session is not limited. Failed logins are throttled per client IP (5 failures a minute, then a lock-out of 1, 2, 4 … minutes up to an hour, cleared by a success). Client IP comes from the TCP peer, or from `X-Forwarded-For` only when the peer is in `auth.trustedProxies`.
- **CSRF:** a cookie-authenticated write must be `application/json` or carry `X-Unigif-Request`, and an `Origin` header, when present, must match `Host` or `publicUrl`. CORS never allows credentials, so cookies cannot ride a cross-origin request; keys can, from `cors.origins`.
- **Hidden hashtags (`deny_tags`)** are one SQL fragment (`Visibility`) added to every query that returns or touches GIFs, so by-ID lookups, tag counts, stats, the unsorted queue and the event stream all honour it. A key with hidden hashtags may not use them when tagging or uploading, and may not rename, merge or delete hashtags library-wide (those reach GIFs it cannot see). A duplicate upload that matches a GIF the key cannot see returns only the tags the key sent and no media; the same goes for an uploader key without `read`.
- **Trash keeps hashtags attached** but removes the GIF from `tags.gif_count`; restoring (including by uploading the same bytes again) puts it back. Tag rows are deleted as soon as no GIF, trashed or not, carries them (there is no separate janitor for that). `sorted_at` is cleared when a GIF loses its last hashtag, so `sorted_at: null` always means unsorted. The hourly janitor purges trash older than `trash.retentionDays` (files included) and expired sessions.
- **URL fetch** uses its own DNS resolver that drops private, loopback, link-local, CGNAT, documentation and reserved addresses (v4 and v6, including v4-mapped, NAT64 and 6to4), checks literal-IP hosts up front, follows at most 3 redirects by hand and re-validates each hop, ignores proxy environment variables, refuses credentials in URLs, and applies the size cap while streaming plus a total deadline. `fetch.allowPrivate` exists for tests only.
- **Additions beyond the 8.3 table**, needed by the web UI in M3: `POST /auth/password`, `GET /auth/sessions`, `DELETE /auth/sessions/{id}`, `PATCH /me` (owner preferences such as `proceed_to_next`), `GET /gifs?trash=true` (needs the `delete` scope), `?status=` and `?include_revoked=`. `GET /gifs/unsorted/next` returns `{item, remaining, total}`; `after` only marks a position, so the GIF just sorted can be passed.
- **Contract tests** (`tests/contract.rs`) check the spec in both directions: every documented operation is routed and answers with a documented status, and real responses validate against the schemas in `api/openapi.yaml`. They found one missing documented status while being written.
- **Write transactions use `BEGIN IMMEDIATE`** (`db::begin_write`). SQLite's default deferred `BEGIN` fails a transaction that reads before it writes with `SQLITE_BUSY_SNAPSHOT` when another connection commits in between, and `busy_timeout` does not apply. A concurrency test (48 parallel edits) failed about 96% of the time before the change, and the M1 ingest path had the same exposure.
- Not done in M2: `page_url` and the on-demand `gif`/`webm`/`webp` entries of the GIF object (M5), `gif.ready` for failed items, pre-migration snapshots (M6).

**M3, web UI: done**, with these departures from the text above:

- **Scope.** Login, Library (search with chips, hashtag sidebar), Unsorted, the tag sheet with every key in 9.4, upload by button / drag / paste (file or link), and a detail view with Edit hashtags, Copy, Delete (with Undo). Hashtag management, Trash, batch select, live updates over SSE and the API-key screen stay in M7; the gestures, PWA manifest and share target stay in M4. A basic responsive layout (icon rail, bottom tab bar, full-screen sheet) came along because the CSS was cheap, but it is not yet tested on a phone viewport.
- **Stack as planned:** Svelte 5 runes, no router or UI library (a 40-line path router; `?gif=<id>` opens the detail overlay on top of the current list), `thumbhash` for placeholders. The initial JavaScript is about 31 kB gzipped against the 150 kB budget.
- **Grid.** A JS masonry placed from stored dimensions (no layout shift) that only renders tiles near the viewport; each tile plays `preview.mp4` only while on screen. Processing items are re-fetched every 1.5 s until ready, instead of waiting for the SSE stream.
- **Tag sheet.** Saving is optimistic and "Same as last" / Recent update at once, not after the round trip. Shortcuts listen on `window` so they survive focus landing on the backdrop; `Esc` stops propagation so closing the sheet does not also close the detail view under it. Space and comma commit a chip from the `input` event (not `keydown`), so mobile keyboards work. When an arrow key has highlighted a suggestion, `Enter` accepts it as a chip and a second `Enter` saves; otherwise `Enter` commits the typed text and saves. The position shown ("3 of 41") counts the run, so it does not drift as GIFs leave the queue.
- **CSRF.** The first e2e run found that a body-less `DELETE` was rejected: cookie writes need JSON or `X-Unigif-Request`. The client now sends that header on every call.
- **Tests.** `web/e2e` starts the real binary against a throwaway data directory with ffmpeg-made fixtures (`node e2e/serve.mjs`) and covers: upload three GIFs and sort them with the keyboard only, search, wrong password, suggestions / Backspace / Shift+Enter, and edit + delete + undo from the detail view. CI has a new `e2e` job.

**M4, mobile and PWA: done**, with these departures from the text above:

- **Layout.** Phone (< 640 px): bottom tab bar with Library, Unsorted (count badge), Add and Settings, all at least 44 px tall; tablet: icon rail; desktop unchanged. `--tabbar-h` in `tokens.css` is the one source for the bar height, including the home-indicator inset. Toasts move to the top on a phone so they never cover the tab bar or the pinned Save button. Under `pointer: coarse` every `.btn` is 44 px, and tile hashtags are always visible (there is no hover).
- **Add** opens a bottom sheet (a centred card on desktop): Choose files, or paste a link. **Settings** is a small page for now (account and sign out, storage numbers, install hint); API keys, hashtag management and Trash stay in M7.
- **Hashtag browsing on a phone** is a horizontal chip row under the search box (the top 40 by use), shown while no search is active.
- **Tag sheet.** Full screen, the GIF at most 40 % of the height, the Recent row scrolls sideways, suggestions flow under the input instead of floating over the Save row, and the footer (Proceed checkbox and Save) is pinned. The sheet is sized from `visualViewport` (`--vv-height`, `--vv-top`, kept current by `lib/viewport.ts`), so it ends exactly above the on-screen keyboard on iOS and on Chrome, whose default is to shrink only the visual viewport. While a sheet is open the document does not scroll.
- **Gestures** (`lib/gestures.ts`, a Svelte action on the GIF area): swipe left skips, right goes back, down closes. Touch and pen only, 60 px horizontal or 80 px down, and the axis must dominate 1.5 to 1, so a stray drag does nothing. Buttons for Prev, Skip and Delete remain.
- **Long-press on a tile** (450 ms, cancelled by a 10 px drag or a scroll) opens Copy link, Share (only where `navigator.share` exists), Edit hashtags and Delete with Undo. The sheet ignores clicks for its first 400 ms: it opens under the finger, and the click on lift-off would otherwise dismiss it at once. `lib/actions.ts` now holds the copy, share and trash-with-undo code that the tile, the detail view and the tag sheet used to repeat.
- **Playback cap.** On a phone at most six on-screen tiles animate (top to bottom, left to right); the others show their poster and load no video.
- **PWA.** `public/manifest.webmanifest` (standalone, maskable icon, an Unsorted shortcut) and PNG icons generated from `public/icon.svg` by `web/scripts/icons.mjs`; the server serves the manifest as `application/manifest+json`. The service worker (`public/sw.js`, registered in production builds only) does one thing: it answers the share target's `POST /share`. It parks the shared files and text in Cache Storage and redirects to `/share-received`; once the app is signed in it uploads them (links by URL, since the server fetches them with SSRF protection), clears the stash and lands in Unsorted. Nothing else is cached or intercepted, so the API, media and app files always come from the network. Shared text with no link says so in a toast. If a browser POSTs to `/share` without the worker, the server's SPA fallback just opens the app; the share is lost.
- **Install.** Chromium's install prompt is offered in Settings; on iOS Settings explains Add to Home Screen, and says Safari cannot receive shares (use Add, or a Shortcut that posts to quick add).
- **Tests.** The Playwright config has two projects, each with its own throwaway server so the libraries do not interact: `desktop` (`sorting.spec.ts`) and `phone` (`mobile.spec.ts`, Pixel 7 emulation with real touch input through the DevTools protocol). The phone suite covers the manifest and worker; adding files through the system picker; sorting three GIFs with taps, swipes, "Same as last", a recent chip and the keyboard's Done key; the layout rules above (full screen, 40 %, Save pinned, 44 px targets); swipe down; the long-press menu; a GIF shared through a real multipart form POST to `/share`; shared text without a link; and Settings with sign out. After a synthetic swipe or long press, Chromium swallows a tap that follows within a few hundred ms, so those helpers wait before returning; this is a test artefact, not app behaviour.
- Not done: tap-to-play for reduced-motion users on a phone (tiles show posters; the detail view plays), and a real-device check on iOS, which only the owner can do.

**M5, conversion: done**, with these departures from the text above:

- **Where it lives.** `media/convert.rs` is the ffmpeg part (one function per request, no state); `cache.rs` decides what is cached, shares concurrent requests and evicts. `GET /m/{id}/{variant}` is the same handler as before: a stored variant is opened directly, an on-demand one goes through `cache::ensure` first, so Range, `ETag`, `304` and the immutable cache header behave identically. The `Variant` enum is still the only list of names; `full.gif?w=9000` ignores the query and `full.png` is `404`.
- **One conversion per file.** An in-flight map of shared futures keyed by `id/name`. The conversion runs in a detached task, so a client that disconnects does not waste it, and the entry removes itself when the task ends, success or failure (a failure is a `500` with the detail in the log, and the next request tries again). The "global semaphore" is the existing ffmpeg one (`media.concurrency`, default 2) and the hard timeout is `media.timeoutSecs` (30). Output is written to `{name}.part` and renamed, so a half-written file is never served.
- **Source choice.** `full.gif`, `preview.gif` and `full.webp` read the kept original when it is an image (GIF, WebP, PNG), which is the only way transparency survives (palette with a reserved transparent entry, `alpha_threshold=128`; an alpha-carrying WebP); otherwise `full.mp4`. `full.webm` always reads `full.mp4`, as planned, so it has the matte, not alpha. A kept MP4 or MKV original is never used (it is no better than `full.mp4`).
- **Sizes.** Width is capped and never upscaled: `full.gif` 480 px at 15 fps, `preview.gif` 220 px at 10 fps, `full.webp` 480 px at up to `media.maxFps`, `full.webm` at the stored size. The frame rate is only ever capped, not raised, so a 10 fps GIF is not padded with duplicate frames. All of these are `media.*` settings (`gifWidth`, `gifFps`, `previewGifWidth`, `previewGifFps`, `webpWidth`, `webpQuality`, `webmCrf`).
- **Cache.** Files live under `cacheDir/{shard}/{id}/`. `variants` rows with `tier = 'cache'` only record size and last access. After every conversion, and hourly in the janitor, the cache is trimmed to `cache.maxSize` (default 2 GiB), oldest access first, never the file being served. Before deleting anything the rows are checked against the disk, so emptying the directory by hand is safe (the rows disappear and the files are rebuilt on demand). Purging a GIF from the trash removes its cache too, and a trashed GIF is not converted. A limit smaller than one file still lets that file be served once, and the next trim removes it.
- **Share page** (`/g/{id}`, public like media, `Cache-Control: max-age=60`, `X-Robots-Tag: noindex`). Open Graph and Twitter tags (`og:video` with type and size, `og:image` = the poster, `twitter:card = summary_large_image`) over a plain dark page with an autoplaying muted video and links to the four formats. URLs in the tags are absolute: from `publicUrl`, else from the request's `Host` and `X-Forwarded-Proto` (escaped). It deliberately shows **no hashtags and no description**: those are the owner's organisation, and the page is reachable by anyone who has the link. `/g/{id}.mp4|gif|webm|webp` answer `307` to the media file; a trashed or unknown id is `404`.
- **API.** The GIF object gains `page_url` and `media.gif`, `preview_gif`, `webm`, `webp` (`{url, on_demand: true}`; they carry no size or byte count because nothing exists yet). `/stats` gains `cache_files`, `cache_bytes` and `cache_limit`, filtered by the key's hidden hashtags like everything else. Settings shows "Converted copies" next to Stored and Saved; the detail view's Copy row is Link (the share page), MP4, GIF, WebM and WebP.
- **Tests.** `tests/convert.rs` (15): every format is the right codec and size and is converted exactly once; Range / ETag on a converted file; six concurrent requests get one identical file and one cache row; a zero-second budget gives a `500`, leaves no files and does not wedge the in-flight map; transparency survives in GIF (checked by decoding) and WebP (checked in the container); eviction order (A is evicted first, then C after B is used again) and the byte limit; the janitor trim; a wiped cache directory; purge and trash; the share page tags, escaping and redirects. One more Playwright test copies each format's link and fetches it without credentials.
- **Known limits.** ffmpeg 6.x cannot decode an animated WebP (see M1), so the WebP tests read the RIFF container (animation flag, alpha flag, canvas size, frame count) instead of decoding it. Not done: a per-GIF "convert now" warm-up, and `Cache-Control` for converted files stays `immutable` although a file rebuilt after eviction could differ by a few bytes (the `ETag` includes the length, so a changed file is a different validator).

M0 to M6 are the "storage accessor" you asked for first. Packaging is kept working from M0 onward; M6 finishes and hardens it rather than starting it.

**Later, not scheduled**: Telegram integration (section 11), a Tenor/Klipy-shaped adapter, and wiring Unigif into the social network as its GIF provider.

### Testing

- **Unit**: hashtag normalisation, scope checks, cursor encoding, target-size maths.
- **Media integration** (real ffmpeg, fixtures): transparent GIF, variable frame delays, odd dimensions, single-frame GIF, oversized canvas, truncated file, an MP4 that needs only a remux, an MP4 with audio.
- **API contract**: every endpoint validated against `openapi.yaml`.
- **End-to-end** (Playwright): the keyboard sorting flow on desktop and on a phone viewport, upload by paste, key creation shows the secret once.
- **Scale**: seed 50 000 GIFs; list and search p95 under 50 ms; grid scroll stays smooth.
- **NixOS VM test**: section 10.4.

---

## 14. Risks and open questions

**Risks**

| Risk | Mitigation |
|---|---|
| Transparent GIFs lose alpha in MP4 | Originals kept for those by default; GIF and WebP output are produced from the original |
| A GIF with thousands of unique colours or noise can be larger as MP4 than expected, or look worse | CRF is a setting; the pipeline keeps the smaller of "re-encode" and "original" when the original is already a video |
| Public media URLs | Unguessable IDs; optional private mode; per-key hidden hashtags for anything sensitive |
| ffmpeg on untrusted input | Limits before decode, timeout, sandbox, memory cap |
| Telegram details change (limits, client code layout, plugin engines) | Deferred; everything marked **(verify)** is re-checked when that work starts |
| SQLite write contention | One writer, short transactions, WAL; ingest work happens outside transactions |
| Rust compile times slow the edit-run loop | Single crate, `cargo check` and `nextest` in the dev shell, the web UI runs on the Vite dev server against a running backend |
| A SQL typo only shows up at run time (no compile-time query checking) | Every query path is exercised by the integration tests; `sqlx` query macros can be adopted later if it proves worth the offline-metadata upkeep |

**Confirmed by the owner (2026-11-12)**

1. **Stack**: no Go. Rust + SQLite + Svelte, as one binary. Rust was my pick among the remaining options because it keeps the single-binary, low-memory, easily sandboxed design unchanged.
2. **Media visibility**: public by unguessable URL.
3. **Tenancy**: one library per instance; friends get read keys to it.
4. **Originals**: discarded after conversion, except for transparent GIFs.
5. **Telegram**: deferred entirely; not part of this plan's schedule.
6. **Licence**: MIT.

---

## 15. Sources

- Tenor API shutdown and alternatives: [9to5Google](https://9to5google.com/2026/06/30/google-tenor-api-gif-updates/), [Shacknews](https://www.shacknews.com/article/149852/google-tenor-api-service-discontinued-twitter-discord-whatsapp-bluesky), [AlternativeTo](https://alternativeto.net/news/2026/7/google-discontinues-tenor-gif-api-for-developers-impacting-discord-x-whatsapp-and-bluesky/), [Klipy docs](https://klipy.com/docs), [Zuplo on the Klipy API](https://zuplo.com/learning-center/klipy-api)
- Telegram GIFs: [Working with GIFs](https://core.telegram.org/api/gifs), [messages.getSavedGifs](https://core.telegram.org/method/messages.getSavedGifs), [messages.saveGif](https://core.telegram.org/method/messages.saveGif), [Telegram Premium limits](https://core.telegram.org/api/premium), [Telegram limits](https://limits.tginfo.me)
- Telegram bots and clients: [InlineQueryResultMpeg4Gif (python-telegram-bot docs)](https://docs.python-telegram-bot.org/en/latest/telegram.inlinequeryresultmpeg4gif.html), [exteraGram plugin docs](https://plugins.exteragram.app/docs), [exDesktop wiki](https://github.com/metawka/exDesktop/wiki)
- Tagging UX: [Raindrop.io collections](https://help.raindrop.io/collections.md), [Raindrop.io tags](https://help.raindrop.io/tags.md), [Raindrop.io 5.0](https://blog.raindrop.io/raindrop-io-5-0), [Eagle tags](https://en.eagle.cool/article/492-tags), [Eagle 4.0 tag panel](https://eagle.cool/blog/post/eagle4-build6), [Hydrus getting started (archive/delete filter)](https://hydrusnetwork.github.io/hydrus/getting_started_files.html)
- Self-hosted prior art: [evoL/gif](https://github.com/evoL/gif), [Giffy](https://github.com/NotPiny/Giffy), [Meme Search](https://selfhostyourself.com/services/meme-search), [PictShare](https://selfhostyourself.com/services/pictshare)
- PWA share target: [Chrome docs](https://developer.chrome.com/docs/capabilities/web-apis/web-share-target), [MDN](https://developer.mozilla.org/en-US/docs/Web/Progressive_web_apps/Manifest/Reference/share_target), [WebKit bug 194593](https://bugs.webkit.org/show_bug.cgi?id=194593)
