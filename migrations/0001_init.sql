-- Unigif schema v1. Timestamps are Unix milliseconds.

CREATE TABLE users (
  id            INTEGER PRIMARY KEY,
  username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
  password_hash TEXT NOT NULL,
  prefs         TEXT NOT NULL DEFAULT '{}',
  created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
  token_hash   BLOB PRIMARY KEY,
  user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  user_agent   TEXT,
  ip           TEXT,
  created_at   INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  expires_at   INTEGER NOT NULL
);

CREATE TABLE api_keys (
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  secret_hash  BLOB NOT NULL,
  scopes       TEXT NOT NULL,
  deny_tags    TEXT NOT NULL DEFAULT '[]',
  rate_limit   INTEGER,
  created_by   INTEGER NOT NULL REFERENCES users(id),
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER,
  last_used_at INTEGER,
  revoked_at   INTEGER
);

CREATE TABLE gifs (
  id             TEXT PRIMARY KEY,
  status         TEXT NOT NULL CHECK (status IN ('processing', 'ready', 'failed')),
  sha256         BLOB NOT NULL UNIQUE,
  phash          INTEGER,
  width          INTEGER,
  height         INTEGER,
  duration_ms    INTEGER,
  has_alpha      INTEGER NOT NULL DEFAULT 0,
  original_mime  TEXT NOT NULL,
  original_kept  INTEGER NOT NULL DEFAULT 0,
  bytes_original INTEGER NOT NULL,
  bytes_stored   INTEGER NOT NULL DEFAULT 0,
  thumbhash      BLOB,
  description    TEXT,
  source         TEXT NOT NULL,
  source_id      TEXT,
  source_url     TEXT,
  added_by_key   TEXT REFERENCES api_keys(id),
  tag_count      INTEGER NOT NULL DEFAULT 0,
  use_count      INTEGER NOT NULL DEFAULT 0,
  last_used_at   INTEGER,
  created_at     INTEGER NOT NULL,
  sorted_at      INTEGER,
  deleted_at     INTEGER
);
CREATE UNIQUE INDEX gifs_source   ON gifs(source, source_id) WHERE source_id IS NOT NULL;
CREATE INDEX        gifs_library  ON gifs(created_at DESC, id) WHERE deleted_at IS NULL;
CREATE INDEX        gifs_unsorted ON gifs(created_at DESC, id) WHERE tag_count = 0 AND deleted_at IS NULL;

CREATE TABLE tags (
  id           INTEGER PRIMARY KEY,
  name         TEXT NOT NULL UNIQUE,
  gif_count    INTEGER NOT NULL DEFAULT 0,
  last_used_at INTEGER,
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
  name           TEXT NOT NULL,
  tier           TEXT NOT NULL CHECK (tier IN ('stored', 'cache')),
  bytes          INTEGER NOT NULL,
  width          INTEGER,
  height         INTEGER,
  created_at     INTEGER NOT NULL,
  last_access_at INTEGER,
  PRIMARY KEY (gif_id, name)
) WITHOUT ROWID;

CREATE TABLE jobs (
  id         INTEGER PRIMARY KEY,
  kind       TEXT NOT NULL,
  gif_id     TEXT,
  state      TEXT NOT NULL CHECK (state IN ('queued', 'running', 'done', 'failed')),
  attempts   INTEGER NOT NULL DEFAULT 0,
  error      TEXT,
  run_after  INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX jobs_pending ON jobs(run_after) WHERE state = 'queued';
