-- Server-wide settings edited from the web UI (third-party search API keys).
-- Values are stored as plain text, like the rest of the database: protect the data directory.

CREATE TABLE settings (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at INTEGER NOT NULL
) WITHOUT ROWID;
