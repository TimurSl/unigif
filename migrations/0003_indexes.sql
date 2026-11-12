-- Indexes found by the 50 000-GIF scale test (tests/scale.rs).
-- The trash count and listing.
CREATE INDEX gifs_trash ON gifs(deleted_at DESC, id) WHERE deleted_at IS NOT NULL;
-- "Most used" order.
CREATE INDEX gifs_used ON gifs(use_count DESC, created_at DESC, id) WHERE deleted_at IS NULL;
-- Cache rows (size totals, least-recently-used eviction) among all the stored ones.
CREATE INDEX variants_cache ON variants(last_access_at) WHERE tier = 'cache';
