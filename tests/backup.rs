//! `unigif backup` and the snapshot taken before a migration.

use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};
use unigif::{backup, db};

/// Open a snapshot read-only and run one scalar query against it.
async fn scalar(path: &Path, sql: &str) -> i64 {
    let opts =
        SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display())).unwrap().read_only(true);
    let mut c = SqliteConnection::connect_with(&opts).await.unwrap();
    sqlx::query_scalar(sql).fetch_one(&mut c).await.unwrap()
}

#[tokio::test]
async fn snapshot_is_a_consistent_copy_and_old_ones_are_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("data").join("unigif.db");
    let pool = db::open(&db_path).await.unwrap();
    unigif::auth::accounts::create_user(&pool, "owner", "correct horse battery").await.unwrap();

    let backups = dir.path().join("backups");
    let first = backup::run(&pool, &backups, 2).await.unwrap();
    assert_eq!(scalar(&first, "SELECT count(*) FROM users").await, 1);
    assert_eq!(
        scalar(&first, "SELECT count(*) FROM pragma_integrity_check WHERE integrity_check = 'ok'").await,
        1
    );
    // A snapshot is one self-contained file.
    assert!(!first.with_extension("db-wal").exists());

    // Later changes do not reach it; the next snapshot sees them.
    unigif::auth::accounts::create_user(&pool, "second", "correct horse battery").await.unwrap();
    assert_eq!(scalar(&first, "SELECT count(*) FROM users").await, 1);

    // Make room for three snapshots, the two older ones with earlier names.
    for n in ["unigif-20200101-000000000.db", "unigif-20200102-000000000.db"] {
        std::fs::copy(&first, backups.join(n)).unwrap();
    }
    let last = backup::run(&pool, &backups, 2).await.unwrap();
    assert_eq!(scalar(&last, "SELECT count(*) FROM users").await, 2);

    let mut names: Vec<_> =
        std::fs::read_dir(&backups).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.iter().all(|n| !n.contains("2020")), "the oldest ones go first: {names:?}");
    assert!(names.iter().all(|n| !n.ends_with(".partial")));
}

#[tokio::test]
async fn refuses_to_overwrite_and_zero_keep() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(&dir.path().join("unigif.db")).await.unwrap();
    let dest = dir.path().join("x.db");
    backup::snapshot(&pool, &dest).await.unwrap();
    assert!(backup::snapshot(&pool, &dest).await.is_err());
    assert!(backup::run(&pool, dir.path(), 0).await.is_err());
    // A path with a quote in it is handled, not injected.
    backup::snapshot(&pool, &dir.path().join("it's.db")).await.unwrap();
}

#[tokio::test]
async fn pending_migration_snapshots_the_existing_database_first() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let db_path = data.join("unigif.db");

    // A brand-new database is migrated without a snapshot.
    let pool = db::open(&db_path).await.unwrap();
    assert!(!data.join("backups").exists(), "nothing to protect in a new database");
    // Re-opening an up-to-date one does not snapshot either.
    drop(pool);
    let pool = db::open(&db_path).await.unwrap();
    assert!(!data.join("backups").exists());

    // Pretend the last migration has not run yet (an older version of the app wrote this file).
    let latest: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations").fetch_one(&pool).await.unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ?").bind(latest).execute(&pool).await.unwrap();
    sqlx::query("DROP TABLE settings").execute(&pool).await.unwrap(); // what 0002 created
    drop(pool);

    let pool = db::open(&db_path).await.unwrap();
    let snaps: Vec<_> = std::fs::read_dir(data.join("backups"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(snaps.len(), 1, "{snaps:?}");
    assert!(
        snaps[0].starts_with(&format!("pre-migration-v{}-", latest - 1))
            || snaps[0].starts_with("pre-migration-v")
    );
    // The copy is the state before migrating: no settings table; the live DB has it again.
    let snap = data.join("backups").join(&snaps[0]);
    assert_eq!(scalar(&snap, "SELECT count(*) FROM sqlite_master WHERE name = 'settings'").await, 0);
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name = 'settings'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(live, 1);
}
