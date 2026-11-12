-- Scale fixture: 50 000 ready GIFs (rows only, no media files), 200 hashtags,
-- one GIF in ten unsorted, the rest with one or two hashtags, varied usage.
-- Shared by `cargo test --release --test scale -- --ignored` and the Playwright
-- `scale` project (E2E_SCALE=1). Run against a migrated, empty database.
-- Ids are 12 base58 characters: 'S' plus the number with 0 spelled 'z'.
BEGIN;

WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 50000)
INSERT INTO gifs (id, status, sha256, width, height, duration_ms, original_mime, bytes_original,
                  bytes_stored, source, use_count, last_used_at, created_at)
SELECT 'S' || replace(printf('%011d', i), '0', 'z'), 'ready', randomblob(32),
       CASE i % 3 WHEN 0 THEN 480 WHEN 1 THEN 320 ELSE 270 END,
       CASE i % 4 WHEN 0 THEN 270 WHEN 1 THEN 320 WHEN 2 THEN 480 ELSE 360 END,
       2000, 'image/gif', 400000, 90000, 'import',
       (i * 37) % 23, CASE WHEN (i * 37) % 23 > 0 THEN 1700000000000 + i * 1000 END,
       1700000000000 + i * 60000
FROM n;

WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 200)
INSERT INTO tags (id, name, gif_count, created_at) SELECT i, 'tag' || i, 0, 1700000000000 FROM n;

WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 50000)
INSERT INTO gif_tags (gif_id, tag_id, created_at)
SELECT 'S' || replace(printf('%011d', i), '0', 'z'), (i % 200) + 1, 1700000000000 FROM n WHERE i % 10 != 0
UNION ALL
SELECT 'S' || replace(printf('%011d', i), '0', 'z'), ((i * 7 + 3) % 200) + 1, 1700000000000 FROM n
WHERE i % 10 != 0 AND i % 3 != 0 AND ((i * 7 + 3) % 200) != (i % 200);

UPDATE gifs SET tag_count = (SELECT COUNT(*) FROM gif_tags WHERE gif_id = gifs.id);
UPDATE gifs SET sorted_at = created_at WHERE tag_count > 0;
UPDATE tags SET gif_count = (SELECT COUNT(*) FROM gif_tags WHERE tag_id = tags.id),
                last_used_at = 1700000000000;

INSERT INTO variants (gif_id, name, tier, bytes, width, height, created_at)
SELECT id, 'full.mp4', 'stored', 80000, width, height, created_at FROM gifs WHERE id LIKE 'S%'
UNION ALL
SELECT id, 'preview.mp4', 'stored', 9000, width * 200 / height, 200, created_at FROM gifs WHERE id LIKE 'S%'
UNION ALL
SELECT id, 'poster.webp', 'stored', 1000, width * 200 / height, 200, created_at FROM gifs WHERE id LIKE 'S%';

COMMIT;
ANALYZE;
