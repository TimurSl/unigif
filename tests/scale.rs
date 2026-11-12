//! Scale test (plan section 13, Testing): 50 000 GIFs, list and search p95
//! under 50 ms. Slow to seed and only meaningful optimised, so it is ignored
//! by default. Run it with:
//!
//!     cargo test --release --test scale -- --ignored --nocapture
#![allow(clippy::field_reassign_with_default)]
mod common;

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use common::*;

const SEED: &str = include_str!("scale/seed.sql");
const RUNS: usize = 60;
const BUDGET: Duration = Duration::from_millis(50);

#[tokio::test(flavor = "multi_thread")]
#[ignore = "slow: seeds 50 000 GIFs; run with --release -- --ignored"]
async fn list_and_search_stay_fast_at_50k_gifs() {
    // Hundreds of requests a minute: lift the per-key rate limit.
    let h = Harness::with(|c| c.rate_limit.read_per_min = 1_000_000).await;
    let t = Instant::now();
    sqlx::raw_sql(SEED).execute(&h.app.db).await.unwrap();
    println!("seeded in {:.1} s", t.elapsed().as_secs_f64());
    let (_, st) = h.call("GET", "/api/v1/stats", None).await;
    assert_eq!(st["gifs"], 50_000);

    // A cursor from deep in the library, for the "page 100" case.
    let mut cursor = String::new();
    for _ in 0..20 {
        let (_, page) = h.call("GET", &format!("/api/v1/gifs?limit=100{cursor}"), None).await;
        cursor = format!("&cursor={}", page["next_cursor"].as_str().unwrap());
    }

    let cases = [
        ("library, first page", "/api/v1/gifs?limit=60".to_string()),
        ("library, page 21", format!("/api/v1/gifs?limit=60{cursor}")),
        ("oldest first", "/api/v1/gifs?limit=60&sort=old".to_string()),
        ("most used", "/api/v1/gifs?limit=60&sort=used".to_string()),
        ("unsorted", "/api/v1/gifs?limit=60&unsorted=true".to_string()),
        ("one hashtag", "/api/v1/gifs?limit=60&q=tag7%20".to_string()),
        ("two hashtags", "/api/v1/gifs?limit=60&q=tag7+tag52%20".to_string()),
        ("prefix while typing", "/api/v1/gifs?limit=60&q=tag1".to_string()),
        ("narrow prefix", "/api/v1/gifs?limit=60&q=tag199".to_string()),
        ("exclusion", "/api/v1/gifs?limit=60&q=tag7+-tag52".to_string()),
        ("no match", "/api/v1/gifs?limit=60&q=nothing%20".to_string()),
        ("hashtag list", "/api/v1/tags?sort=popular&limit=100".to_string()),
        ("stats", "/api/v1/stats".to_string()),
    ];
    // The same through a key that hides a hashtag: every query takes the visibility clause.
    let hiding = h.mint(unigif::auth::Scopes::preset("read_only").unwrap(), &["tag3"]).await;
    let hidden_cases = [
        ("hidden: library", "/api/v1/gifs?limit=60".to_string()),
        ("hidden: one hashtag", "/api/v1/gifs?limit=60&q=tag7%20".to_string()),
        ("hidden: hashtag list", "/api/v1/tags?sort=popular&limit=100".to_string()),
        ("hidden: stats", "/api/v1/stats".to_string()),
    ];
    let mut failures = vec![];
    let all = cases
        .iter()
        .map(|(n, u)| (n, u, h.token.clone()))
        .chain(hidden_cases.iter().map(|(n, u)| (n, u, hiding.clone())));
    for (name, uri, token) in all {
        let mut times = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let t = Instant::now();
            let (s, _) = json(h.send(h.req("GET", uri, Some(&token), None)).await).await;
            times.push(t.elapsed());
            assert_eq!(s, StatusCode::OK, "{name}");
        }
        times.sort();
        let p95 = times[RUNS * 95 / 100];
        println!("{name:<22} p50 {:>6.1} ms  p95 {:>6.1} ms", ms(times[RUNS / 2]), ms(p95));
        if p95 > BUDGET {
            failures.push(format!("{name}: p95 {:.1} ms", ms(p95)));
        }
    }
    assert!(failures.is_empty(), "over the {BUDGET:?} budget:\n{}", failures.join("\n"));
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
