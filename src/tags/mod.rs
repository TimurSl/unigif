//! Hashtag normalisation and the transactional helpers that keep
//! `tags.gif_count`, `gifs.tag_count` and `gifs.sorted_at` consistent.

use sqlx::{Sqlite, Transaction};
use unicode_normalization::UnicodeNormalization;

pub const MAX_TAG_LEN: usize = 64;
pub const MAX_TAGS_PER_GIF: usize = 32;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TagError {
    #[error("hashtag {0:?} is empty after normalisation")]
    Empty(String),
    #[error("hashtag {0:?} is longer than {MAX_TAG_LEN} characters")]
    TooLong(String),
    #[error("hashtag {0:?} may only contain letters, digits and underscores")]
    InvalidChar(String),
    #[error("a GIF can have at most {MAX_TAGS_PER_GIF} hashtags")]
    TooMany,
}

/// Normalise one tag: strip leading `#`, NFKC, lower-case, validate.
pub fn normalize(raw: &str) -> Result<String, TagError> {
    let trimmed = raw.trim().trim_start_matches('#');
    let norm: String = trimmed.nfkc().collect::<String>().to_lowercase().nfkc().collect();
    if norm.is_empty() {
        return Err(TagError::Empty(raw.to_string()));
    }
    if norm.chars().count() > MAX_TAG_LEN {
        return Err(TagError::TooLong(raw.to_string()));
    }
    if !norm.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(TagError::InvalidChar(raw.to_string()));
    }
    Ok(norm)
}

/// Normalise a list of tags, dropping duplicates and keeping first-seen order.
pub fn normalize_all<S: AsRef<str>>(raw: &[S]) -> Result<Vec<String>, TagError> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for r in raw {
        let n = normalize(r.as_ref())?;
        if !out.contains(&n) {
            out.push(n);
        }
    }
    if out.len() > MAX_TAGS_PER_GIF {
        return Err(TagError::TooMany);
    }
    Ok(out)
}

/// Split free text (`"#a #b, c"`) on whitespace and commas, then normalise.
pub fn parse_list(text: &str) -> Result<Vec<String>, TagError> {
    let parts: Vec<&str> =
        text.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).collect();
    normalize_all(&parts)
}

/// Add tags to a GIF inside a transaction. Already-present tags are left
/// alone. Returns the number of tags that were newly attached.
pub async fn add_tags(
    tx: &mut Transaction<'_, Sqlite>,
    gif_id: &str,
    names: &[String],
    now: i64,
) -> Result<usize, AddTagsError> {
    let current: i64 = sqlx::query_scalar("SELECT tag_count FROM gifs WHERE id = ?")
        .bind(gif_id)
        .fetch_one(&mut **tx)
        .await?;
    let mut added = 0usize;
    for name in names {
        let tag_id: i64 = sqlx::query_scalar(
            "INSERT INTO tags (name, gif_count, last_used_at, created_at) VALUES (?, 0, ?, ?)
             ON CONFLICT(name) DO UPDATE SET last_used_at = excluded.last_used_at
             RETURNING id",
        )
        .bind(name)
        .bind(now)
        .bind(now)
        .fetch_one(&mut **tx)
        .await?;
        let res = sqlx::query("INSERT OR IGNORE INTO gif_tags (gif_id, tag_id, created_at) VALUES (?, ?, ?)")
            .bind(gif_id)
            .bind(tag_id)
            .bind(now)
            .execute(&mut **tx)
            .await?;
        if res.rows_affected() == 1 {
            if current as usize + added + 1 > MAX_TAGS_PER_GIF {
                return Err(AddTagsError::Tag(TagError::TooMany));
            }
            sqlx::query("UPDATE tags SET gif_count = gif_count + 1 WHERE id = ?")
                .bind(tag_id)
                .execute(&mut **tx)
                .await?;
            added += 1;
        }
    }
    if added > 0 {
        sqlx::query(
            "UPDATE gifs SET tag_count = tag_count + ?, sorted_at = COALESCE(sorted_at, ?) WHERE id = ?",
        )
        .bind(added as i64)
        .bind(now)
        .bind(gif_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(added)
}

/// Remove tags from a (non-trashed) GIF. Returns how many were detached.
/// A GIF left without hashtags goes back to Unsorted (`sorted_at` cleared).
pub async fn remove_tags(
    tx: &mut Transaction<'_, Sqlite>,
    gif_id: &str,
    names: &[String],
) -> Result<usize, sqlx::Error> {
    let mut removed = 0usize;
    let mut touched = Vec::new();
    for name in names {
        let Some(tag_id) = sqlx::query_scalar::<_, i64>("SELECT id FROM tags WHERE name = ?")
            .bind(name)
            .fetch_optional(&mut **tx)
            .await?
        else {
            continue;
        };
        let res = sqlx::query("DELETE FROM gif_tags WHERE gif_id = ? AND tag_id = ?")
            .bind(gif_id)
            .bind(tag_id)
            .execute(&mut **tx)
            .await?;
        if res.rows_affected() == 1 {
            sqlx::query("UPDATE tags SET gif_count = gif_count - 1 WHERE id = ?")
                .bind(tag_id)
                .execute(&mut **tx)
                .await?;
            removed += 1;
            touched.push(tag_id);
        }
    }
    if removed > 0 {
        sqlx::query(
            "UPDATE gifs SET tag_count = tag_count - ?,
                    sorted_at = CASE WHEN tag_count - ? = 0 THEN NULL ELSE sorted_at END
             WHERE id = ?",
        )
        .bind(removed as i64)
        .bind(removed as i64)
        .bind(gif_id)
        .execute(&mut **tx)
        .await?;
        for id in touched {
            drop_if_orphaned(tx, id).await?;
        }
    }
    Ok(removed)
}

/// Make the GIF's hashtags exactly `names`.
pub async fn replace_tags(
    tx: &mut Transaction<'_, Sqlite>,
    gif_id: &str,
    names: &[String],
    now: i64,
) -> Result<(), AddTagsError> {
    let current: Vec<String> = sqlx::query_scalar(
        "SELECT t.name FROM gif_tags gt JOIN tags t ON t.id = gt.tag_id WHERE gt.gif_id = ?",
    )
    .bind(gif_id)
    .fetch_all(&mut **tx)
    .await?;
    let stale: Vec<String> = current.iter().filter(|c| !names.contains(c)).cloned().collect();
    remove_tags(tx, gif_id, &stale).await?;
    add_tags(tx, gif_id, names, now).await?;
    Ok(())
}

/// A tag nobody carries (not even a trashed GIF) has no reason to exist.
async fn drop_if_orphaned(tx: &mut Transaction<'_, Sqlite>, tag_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM tags WHERE id = ? AND NOT EXISTS (SELECT 1 FROM gif_tags WHERE tag_id = ?)")
        .bind(tag_id)
        .bind(tag_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Recompute `gif_count` (live GIFs only) from the join table.
async fn recount_tag(tx: &mut Transaction<'_, Sqlite>, tag_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE tags SET gif_count = (SELECT COUNT(*) FROM gif_tags gt JOIN gifs g ON g.id = gt.gif_id
                                      WHERE gt.tag_id = tags.id AND g.deleted_at IS NULL)
         WHERE id = ?",
    )
    .bind(tag_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Recompute `tag_count` and `sorted_at` for GIFs whose tag set changed under them.
async fn recount_gifs(
    tx: &mut Transaction<'_, Sqlite>,
    gif_ids: &[String],
    now: i64,
) -> Result<(), sqlx::Error> {
    for id in gif_ids {
        sqlx::query(
            "UPDATE gifs SET tag_count = (SELECT COUNT(*) FROM gif_tags WHERE gif_id = gifs.id)
             WHERE id = ?",
        )
        .bind(id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE gifs SET sorted_at = CASE WHEN tag_count = 0 THEN NULL ELSE COALESCE(sorted_at, ?) END
             WHERE id = ?",
        )
        .bind(now)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum TagOpError {
    #[error("no such hashtag")]
    NotFound,
    #[error("a hashtag with that name already exists; merge instead")]
    Exists,
    #[error(transparent)]
    Invalid(#[from] TagError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

async fn tag_id(tx: &mut Transaction<'_, Sqlite>, name: &str) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM tags WHERE name = ?").bind(name).fetch_optional(&mut **tx).await
}

pub async fn rename(tx: &mut Transaction<'_, Sqlite>, from: &str, to: &str) -> Result<(), TagOpError> {
    let to = normalize(to)?;
    let id = tag_id(tx, from).await?.ok_or(TagOpError::NotFound)?;
    if to == from {
        return Ok(());
    }
    if tag_id(tx, &to).await?.is_some() {
        return Err(TagOpError::Exists);
    }
    sqlx::query("UPDATE tags SET name = ? WHERE id = ?").bind(&to).bind(id).execute(&mut **tx).await?;
    Ok(())
}

/// Move every GIF of `from` onto `into` and delete `from`. Returns the ids of
/// the GIFs that were touched.
pub async fn merge(
    tx: &mut Transaction<'_, Sqlite>,
    from: &str,
    into: &str,
    now: i64,
) -> Result<Vec<String>, TagOpError> {
    let into = normalize(into)?;
    let src = tag_id(tx, from).await?.ok_or(TagOpError::NotFound)?;
    let dst = tag_id(tx, &into).await?.ok_or(TagOpError::NotFound)?;
    if src == dst {
        return Ok(vec![]);
    }
    let gifs: Vec<String> = sqlx::query_scalar("SELECT gif_id FROM gif_tags WHERE tag_id = ?")
        .bind(src)
        .fetch_all(&mut **tx)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO gif_tags (gif_id, tag_id, created_at) SELECT gif_id, ?, created_at FROM gif_tags WHERE tag_id = ?")
        .bind(dst)
        .bind(src)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM tags WHERE id = ?").bind(src).execute(&mut **tx).await?; // cascades gif_tags
    sqlx::query("UPDATE tags SET last_used_at = MAX(COALESCE(last_used_at, 0), ?) WHERE id = ?")
        .bind(now)
        .bind(dst)
        .execute(&mut **tx)
        .await?;
    recount_tag(tx, dst).await?;
    recount_gifs(tx, &gifs, now).await?;
    Ok(gifs)
}

/// Remove a hashtag from every GIF. Returns the ids of the GIFs it was on.
pub async fn delete(
    tx: &mut Transaction<'_, Sqlite>,
    name: &str,
    now: i64,
) -> Result<Vec<String>, TagOpError> {
    let id = tag_id(tx, name).await?.ok_or(TagOpError::NotFound)?;
    let gifs: Vec<String> = sqlx::query_scalar("SELECT gif_id FROM gif_tags WHERE tag_id = ?")
        .bind(id)
        .fetch_all(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM tags WHERE id = ?").bind(id).execute(&mut **tx).await?;
    recount_gifs(tx, &gifs, now).await?;
    Ok(gifs)
}

#[derive(Debug, thiserror::Error)]
pub enum AddTagsError {
    #[error(transparent)]
    Tag(#[from] TagError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_case_and_hash() {
        assert_eq!(normalize("#Cat").unwrap(), "cat");
        assert_eq!(normalize("CAT").unwrap(), "cat");
        assert_eq!(normalize("  ##cat ").unwrap(), "cat");
    }

    #[test]
    fn supports_other_scripts() {
        assert_eq!(normalize("#Кошка").unwrap(), "кошка");
        assert_eq!(normalize("猫").unwrap(), "猫");
    }

    #[test]
    fn nfkc_folds_compatibility_forms() {
        // fullwidth letters fold to ASCII
        assert_eq!(normalize("Ｃａｔ").unwrap(), "cat");
    }

    #[test]
    fn rejects_bad_tags() {
        assert!(matches!(normalize("#"), Err(TagError::Empty(_))));
        assert!(matches!(normalize("two words"), Err(TagError::InvalidChar(_))));
        assert!(matches!(normalize("a-b"), Err(TagError::InvalidChar(_))));
        assert!(matches!(normalize(&"a".repeat(65)), Err(TagError::TooLong(_))));
        assert!(normalize(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn allows_underscores() {
        assert_eq!(normalize("#funny_cat").unwrap(), "funny_cat");
    }

    #[test]
    fn dedupes_and_limits() {
        assert_eq!(normalize_all(&["#Cat", "cat", "dog"]).unwrap(), vec!["cat", "dog"]);
        let many: Vec<String> = (0..33).map(|i| format!("t{i}")).collect();
        assert_eq!(normalize_all(&many), Err(TagError::TooMany));
    }

    #[test]
    fn parses_pasted_text() {
        assert_eq!(parse_list("#a #b, c\n#A").unwrap(), vec!["a", "b", "c"]);
        assert!(parse_list("").unwrap().is_empty());
    }
}
