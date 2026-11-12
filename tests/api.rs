#![allow(clippy::field_reassign_with_default)]
mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use http_body_util::BodyExt;
use serde_json::json;

/// Hashtags in API order; tags attached in the same request share a timestamp
/// and so come back in name order. Sorted here so tests state sets, not accidents.
fn tags_of(v: &serde_json::Value) -> Vec<String> {
    let mut t: Vec<String> =
        v["tags"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect();
    t.sort();
    t
}

#[tokio::test]
async fn list_and_search() {
    let h = Harness::new().await;
    let a = h.add_gif(1, "cat,funny").await;
    let b = h.add_gif(2, "cat,dog").await;
    let c = h.add_gif(3, "").await;
    let d = h.add_gif(4, "category").await;
    let id = |v: &serde_json::Value| v["id"].as_str().unwrap().to_string();

    // Newest first.
    let (s, all) = h.call("GET", "/api/v1/gifs", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(Harness::ids(&all), vec![id(&d), id(&c), id(&b), id(&a)]);
    assert!(all["next_cursor"].is_null());

    // The last word is a prefix while it is being typed…
    let (_, v) = h.call("GET", "/api/v1/gifs?q=cat", None).await;
    assert_eq!(Harness::ids(&v), vec![id(&d), id(&b), id(&a)]);
    // …and exact once it is followed by a space.
    let (_, v) = h.call("GET", "/api/v1/gifs?q=cat%20", None).await;
    assert_eq!(Harness::ids(&v), vec![id(&b), id(&a)]);
    // Several words: all hashtags are needed. `#` and case are forgiven.
    let (_, v) = h.call("GET", "/api/v1/gifs?q=%23Cat+fun", None).await;
    assert_eq!(Harness::ids(&v), vec![id(&a)]);
    // Exclusion.
    let (_, v) = h.call("GET", "/api/v1/gifs?q=cat+-dog", None).await;
    assert_eq!(Harness::ids(&v), vec![id(&a)]);
    // Repeated `tag` parameters are exact.
    let (_, v) = h.call("GET", "/api/v1/gifs?tag=cat&tag=dog", None).await;
    assert_eq!(Harness::ids(&v), vec![id(&b)]);
    let (_, v) = h.call("GET", "/api/v1/gifs?tag=cat&tag=nonexistent", None).await;
    assert!(Harness::ids(&v).is_empty());
    // A search word that cannot be a hashtag matches nothing instead of failing.
    let (s, v) = h.call("GET", "/api/v1/gifs?q=a-b", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(Harness::ids(&v).is_empty());
    // Unsorted.
    let (_, v) = h.call("GET", "/api/v1/gifs?unsorted=true", None).await;
    assert_eq!(Harness::ids(&v), vec![id(&c)]);
    // Oldest first.
    let (_, v) = h.call("GET", "/api/v1/gifs?sort=old", None).await;
    assert_eq!(Harness::ids(&v), vec![id(&a), id(&b), id(&c), id(&d)]);

    // Cursor pagination walks every item exactly once.
    let mut seen = vec![];
    let mut cursor = String::new();
    loop {
        let (s, page) = h.call("GET", &format!("/api/v1/gifs?limit=3{cursor}"), None).await;
        assert_eq!(s, StatusCode::OK, "{page}");
        seen.extend(Harness::ids(&page));
        match page["next_cursor"].as_str() {
            Some(c) => cursor = format!("&cursor={c}"),
            None => break,
        }
    }
    assert_eq!(seen, vec![id(&d), id(&c), id(&b), id(&a)]);

    // Bad input is a 400, not a 500.
    for q in ["sort=sideways", "limit=x", "cursor=%25%25", "unsorted=maybe", "status=odd"] {
        let (s, v) = h.call("GET", &format!("/api/v1/gifs?{q}"), None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{q}: {v}");
    }
}

#[tokio::test]
async fn used_sort_follows_usage_pings() {
    let h = Harness::new().await;
    let a = h.add_gif(1, "x").await;
    let b = h.add_gif(2, "x").await;
    let (a, b) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap());
    for _ in 0..2 {
        assert_eq!(h.call("POST", &format!("/api/v1/gifs/{a}/use"), None).await.0, StatusCode::NO_CONTENT);
    }
    assert_eq!(h.call("POST", &format!("/api/v1/gifs/{b}/use"), None).await.0, StatusCode::NO_CONTENT);
    let (_, v) = h.call("GET", "/api/v1/gifs?sort=used&limit=1", None).await;
    assert_eq!(Harness::ids(&v), vec![a.to_string()]);
    let (_, v2) = h
        .call(
            "GET",
            &format!("/api/v1/gifs?sort=used&limit=1&cursor={}", v["next_cursor"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(Harness::ids(&v2), vec![b.to_string()]);
}

#[tokio::test]
async fn patching_tags_moves_gifs_in_and_out_of_unsorted() {
    let h = Harness::new().await;
    let g = h.add_gif(1, "").await;
    let id = g["id"].as_str().unwrap();
    assert!(g["sorted_at"].is_null());

    let (s, v) =
        h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({"tags": ["Cat", "#funny"]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(tags_of(&v), vec!["cat", "funny"]);
    assert!(v["sorted_at"].is_string());
    let (_, stats) = h.call("GET", "/api/v1/stats", None).await;
    assert_eq!(
        (stats["gifs"].as_i64(), stats["unsorted"].as_i64(), stats["tags"].as_i64()),
        (Some(1), Some(0), Some(2))
    );

    // Full replace; free text is accepted too.
    let (_, v) = h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({"tags": "dog #funny"}))).await;
    assert_eq!(tags_of(&v), vec!["dog", "funny"]);
    let (_, t) = h.call("GET", "/api/v1/tags", None).await;
    let names: Vec<_> = t["items"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["dog", "funny"], "orphaned hashtag must disappear");

    // Back to unsorted.
    let (_, v) = h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({"tags": []}))).await;
    assert!(tags_of(&v).is_empty());
    assert!(v["sorted_at"].is_null());
    let (_, u) = h.call("GET", "/api/v1/gifs?unsorted=true", None).await;
    assert_eq!(Harness::ids(&u), vec![id.to_string()]);

    // Description alone, set and cleared.
    let (_, v) = h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({"description": "a cat"}))).await;
    assert_eq!(v["description"], "a cat");
    let (_, v) = h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({"description": null}))).await;
    assert!(v["description"].is_null());

    // Errors.
    let (s, _) = h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({"tags": ["two words"]}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let many: Vec<String> = (0..33).map(|i| format!("t{i}")).collect();
    let (s, _) = h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({"tags": many}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = h.call("PATCH", "/api/v1/gifs/AAAAAAAAAAAA", Some(json!({"tags": ["x"]}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn add_and_remove_tags_endpoint() {
    let h = Harness::new().await;
    let g = h.add_gif(1, "a,b").await;
    let id = g["id"].as_str().unwrap();
    let (s, v) = h
        .call(
            "POST",
            &format!("/api/v1/gifs/{id}/tags"),
            Some(json!({"add": ["c", "A"], "remove": ["b", "nothere"]})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(tags_of(&v), vec!["a", "c"]);
    // Removing the last ones makes it unsorted again.
    let (_, v) =
        h.call("POST", &format!("/api/v1/gifs/{id}/tags"), Some(json!({"remove": ["a", "c"]}))).await;
    assert!(tags_of(&v).is_empty() && v["sorted_at"].is_null());
    let (_, st) = h.call("GET", "/api/v1/stats", None).await;
    assert_eq!((st["unsorted"].as_i64(), st["tags"].as_i64()), (Some(1), Some(0)));
}

#[tokio::test]
async fn trash_restore_and_purge() {
    let h = Harness::with(|c| c.trash.retention_days = 30).await;
    let g = h.add_gif(1, "cat").await;
    let other = h.add_gif(2, "cat,dog").await;
    let id = g["id"].as_str().unwrap();

    assert_eq!(h.call("DELETE", &format!("/api/v1/gifs/{id}"), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(h.call("GET", &format!("/api/v1/gifs/{id}"), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        h.call("DELETE", &format!("/api/v1/gifs/{id}"), None).await.0,
        StatusCode::NOT_FOUND,
        "already trashed"
    );
    let (_, v) = h.call("GET", "/api/v1/gifs", None).await;
    assert_eq!(Harness::ids(&v), vec![other["id"].as_str().unwrap().to_string()]);
    let (_, trash) = h.call("GET", "/api/v1/gifs?trash=true", None).await;
    assert_eq!(Harness::ids(&trash), vec![id.to_string()]);
    // Tag counts ignore trashed GIFs.
    let (_, t) = h.call("GET", "/api/v1/tags?sort=popular", None).await;
    assert_eq!(t["items"][0]["name"], "cat");
    assert_eq!(t["items"][0]["gif_count"], 1);
    let (_, st) = h.call("GET", "/api/v1/stats", None).await;
    assert_eq!((st["gifs"].as_i64(), st["trash"].as_i64()), (Some(1), Some(1)));
    // Media of trashed items is gone from the public URL too.
    let url = g["media"]["mp4"]["url"].as_str().unwrap();
    assert_eq!(h.send(Request::get(url).body(Body::empty()).unwrap()).await.status(), StatusCode::NOT_FOUND);
    // A trashed GIF cannot be edited.
    assert_eq!(
        h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({"tags": ["x"]}))).await.0,
        StatusCode::NOT_FOUND
    );

    // Restore brings back the hashtags and counts.
    let (s, v) = h.call("POST", &format!("/api/v1/gifs/{id}/restore"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(tags_of(&v), vec!["cat"]);
    let (_, t) = h.call("GET", "/api/v1/tags?sort=popular", None).await;
    assert_eq!(t["items"][0]["gif_count"], 2);
    assert_eq!(
        h.call("POST", &format!("/api/v1/gifs/{id}/restore"), None).await.0,
        StatusCode::NOT_FOUND,
        "not in trash"
    );

    // Re-uploading trashed bytes restores them as well.
    h.call("DELETE", &format!("/api/v1/gifs/{id}"), None).await;
    let bytes = gif(h.dir.path(), "g1.gif", "96x64", 1);
    let (s, v) = h.add("?tags=again", bytes).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["duplicate"], true);
    assert_eq!(tags_of(&v), vec!["again", "cat"]);
    let (_, t) = h.call("GET", "/api/v1/tags?sort=popular&limit=1", None).await;
    assert_eq!(t["items"][0]["gif_count"], 2);

    // Purge: nothing is old enough yet, then everything in the trash is.
    h.call("DELETE", &format!("/api/v1/gifs/{id}"), None).await;
    assert_eq!(unigif::janitor::purge_trash(&h.app, unigif::db::now_ms()).await.unwrap(), 0);
    let dir = unigif::media::gif_dir(&h.app.cfg.media_dir(), id);
    assert!(dir.exists());
    let later = unigif::db::now_ms() + 31 * 86_400_000;
    assert_eq!(unigif::janitor::purge_trash(&h.app, later).await.unwrap(), 1);
    assert!(!dir.exists(), "files must be deleted with the row");
    assert!(unigif::db::gifs::get(&h.app.db, id).await.unwrap().is_none());
    // The other GIF and its hashtags are untouched.
    let (_, t) = h.call("GET", "/api/v1/tags", None).await;
    assert_eq!(t["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn unsorted_queue_walks_forward_from_any_position() {
    let h = Harness::new().await;
    let a = h.add_gif(1, "").await;
    let b = h.add_gif(2, "").await;
    let c = h.add_gif(3, "").await;
    let id = |v: &serde_json::Value| v["id"].as_str().unwrap().to_string();

    let (s, v) = h.call("GET", "/api/v1/gifs/unsorted/next", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["item"]["id"], json!(id(&c)));
    assert_eq!((v["remaining"].as_i64(), v["total"].as_i64()), (Some(2), Some(3)));

    // Sort the one just shown; `after` no longer being unsorted does not matter.
    h.call("PATCH", &format!("/api/v1/gifs/{}", id(&c)), Some(json!({"tags": ["x"]}))).await;
    let (_, v) = h.call("GET", &format!("/api/v1/gifs/unsorted/next?after={}", id(&c)), None).await;
    assert_eq!(v["item"]["id"], json!(id(&b)));
    assert_eq!((v["remaining"].as_i64(), v["total"].as_i64()), (Some(1), Some(2)));

    let (_, v) = h.call("GET", &format!("/api/v1/gifs/unsorted/next?after={}", id(&b)), None).await;
    assert_eq!(v["item"]["id"], json!(id(&a)));
    assert_eq!(v["remaining"], 0);

    // End of queue.
    let (_, v) = h.call("GET", &format!("/api/v1/gifs/unsorted/next?after={}", id(&a)), None).await;
    assert!(v["item"].is_null());
    assert_eq!(v["remaining"], 0);

    // Oldest first; an unknown `after` just starts from the beginning.
    let (_, v) = h.call("GET", "/api/v1/gifs/unsorted/next?sort=old&after=AAAAAAAAAAAA", None).await;
    assert_eq!(v["item"]["id"], json!(id(&a)));
}

#[tokio::test]
async fn batch_edits_and_trash() {
    let h = Harness::new().await;
    let a = h.add_gif(1, "x").await;
    let b = h.add_gif(2, "").await;
    let (a, b) = (a["id"].as_str().unwrap().to_string(), b["id"].as_str().unwrap().to_string());

    let (s, v) = h
        .call(
            "POST",
            "/api/v1/gifs/batch",
            Some(json!({"ids": [a, b, b, "AAAAAAAAAAAA", "bad"], "add": ["new"], "remove": ["x"]})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["updated"], 2);
    assert_eq!(v["not_found"], json!(["AAAAAAAAAAAA", "bad"]));
    let (_, g) = h.call("GET", &format!("/api/v1/gifs/{a}"), None).await;
    assert_eq!(tags_of(&g), vec!["new"]);
    let (_, g) = h.call("GET", &format!("/api/v1/gifs/{b}"), None).await;
    assert_eq!(tags_of(&g), vec!["new"]);

    let (s, v) = h.call("POST", "/api/v1/gifs/batch", Some(json!({"ids": [a, b], "trash": true}))).await;
    assert_eq!((s, v["updated"].as_i64()), (StatusCode::OK, Some(2)));
    let (_, st) = h.call("GET", "/api/v1/stats", None).await;
    assert_eq!((st["gifs"].as_i64(), st["trash"].as_i64()), (Some(0), Some(2)));

    for body in [
        json!({"ids": []}),
        json!({"ids": [a]}),
        json!({"ids": [], "trash": true}),
        json!({"ids": [a], "add": ["bad tag"]}),
    ] {
        assert_eq!(h.call("POST", "/api/v1/gifs/batch", Some(body)).await.0, StatusCode::BAD_REQUEST);
    }
    // Scopes follow the operation.
    let tag_only = h.mint(unigif::auth::Scopes::from_slice(&[unigif::auth::Scope::Tag]), &[]).await;
    let resp = h
        .send(h.req("POST", "/api/v1/gifs/batch", Some(&tag_only), Some(json!({"ids": [a], "trash": true}))))
        .await;
    assert_eq!(json(resp).await.1["code"], "scope_required");
}

#[tokio::test]
async fn tag_listing_rename_merge_delete() {
    let h = Harness::new().await;
    let a = h.add_gif(1, "cat,pet").await;
    let b = h.add_gif(2, "kitty,pet").await;
    let c = h.add_gif(3, "cat,kitty").await;
    let (a, b, c) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap(), c["id"].as_str().unwrap());

    let (_, t) = h.call("GET", "/api/v1/tags?sort=popular", None).await;
    let simple = |t: &serde_json::Value| -> Vec<(String, i64)> {
        t["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| (x["name"].as_str().unwrap().into(), x["gif_count"].as_i64().unwrap()))
            .collect()
    };
    assert_eq!(simple(&t), vec![("cat".into(), 2), ("kitty".into(), 2), ("pet".into(), 2)]);
    let (_, t) = h.call("GET", "/api/v1/tags?q=ki", None).await;
    assert_eq!(simple(&t), vec![("kitty".into(), 2)]);
    let (_, t) = h.call("GET", "/api/v1/tags?limit=2", None).await;
    assert_eq!(t["items"].as_array().unwrap().len(), 2);
    let (_, t2) = h
        .call("GET", &format!("/api/v1/tags?limit=2&cursor={}", t["next_cursor"].as_str().unwrap()), None)
        .await;
    assert_eq!(simple(&t2), vec![("pet".into(), 2)]);
    assert!(t2["next_cursor"].is_null());

    // Rename; the new name must be free.
    let (s, v) = h.call("PATCH", "/api/v1/tags/pet", Some(json!({"name": "Animals"}))).await;
    assert_eq!((s, v["name"].as_str()), (StatusCode::OK, Some("animals")));
    let (s, v) = h.call("PATCH", "/api/v1/tags/animals", Some(json!({"name": "cat"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("conflict")));
    assert_eq!(
        h.call("PATCH", "/api/v1/tags/nope", Some(json!({"name": "x"}))).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, g) = h.call("GET", &format!("/api/v1/gifs/{a}"), None).await;
    assert_eq!(tags_of(&g), vec!["animals", "cat"]);

    // Merge: GIF c carries both, so it must not end up with a duplicate.
    let (s, v) = h.call("POST", "/api/v1/tags/kitty/merge", Some(json!({"into": "cat"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["name"].as_str(), v["gif_count"].as_i64()), (Some("cat"), Some(3)));
    let (_, g) = h.call("GET", &format!("/api/v1/gifs/{c}"), None).await;
    assert_eq!(tags_of(&g), vec!["cat"]);
    let (_, g) = h.call("GET", &format!("/api/v1/gifs/{b}"), None).await;
    assert_eq!(tags_of(&g), vec!["animals", "cat"]);
    assert_eq!(h.call("GET", "/api/v1/gifs?tag=kitty", None).await.1["items"].as_array().unwrap().len(), 0);

    // Suggestions: recent plus co-occurring.
    let (_, sg) = h.call("GET", "/api/v1/tags/suggest?selected=cat", None).await;
    assert_eq!(sg["related"][0]["name"], "animals");
    assert_eq!(sg["related"][0]["gif_count"], 2);
    assert!(
        sg["recent"].as_array().unwrap().iter().all(|t| t["name"] != "cat"),
        "selected tags are not suggested"
    );
    let (_, sg) = h.call("GET", "/api/v1/tags/suggest", None).await;
    assert!(sg["related"].as_array().unwrap().is_empty());

    // Delete a hashtag: GIFs that carried only it become unsorted.
    assert_eq!(h.call("DELETE", "/api/v1/tags/cat", None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(h.call("DELETE", "/api/v1/tags/cat", None).await.0, StatusCode::NOT_FOUND);
    let (_, u) = h.call("GET", "/api/v1/gifs?unsorted=true", None).await;
    assert_eq!(Harness::ids(&u), vec![c.to_string()]);
    let (_, g) = h.call("GET", &format!("/api/v1/gifs/{c}"), None).await;
    assert!(g["sorted_at"].is_null());
}

#[tokio::test]
async fn quick_add_by_url() {
    let h = Harness::new().await;
    let bytes = gif(h.dir.path(), "u.gif", "96x64", 7);
    let base = serve_files(bytes).await;

    let (s, v) = h
        .call(
            "POST",
            "/api/v1/gifs",
            Some(json!({"url": format!("{base}/a.gif"), "tags": ["Fetched"], "wait": 30})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "ready");
    assert_eq!(tags_of(&v), vec!["fetched"]);
    assert_eq!(v["source"]["type"], "url");
    let row = unigif::db::gifs::get(&h.app.db, v["id"].as_str().unwrap()).await.unwrap().unwrap();
    assert_eq!(row.source_url.as_deref(), Some(format!("{base}/a.gif").as_str()));

    // Same bytes through a redirect: a duplicate, tags merged.
    let (s, v2) =
        h.call("POST", "/api/v1/gifs", Some(json!({"url": format!("{base}/redir"), "tags": "more"}))).await;
    assert_eq!(s, StatusCode::OK, "{v2}");
    assert_eq!(v2["duplicate"], true);
    assert_eq!(v2["id"], v["id"]);

    // Failures.
    let cases = [
        ("/missing", StatusCode::BAD_GATEWAY),
        ("/loop", StatusCode::BAD_GATEWAY),
        ("/text", StatusCode::UNSUPPORTED_MEDIA_TYPE),
    ];
    for (path, want) in cases {
        let (s, v) = h.call("POST", "/api/v1/gifs", Some(json!({"url": format!("{base}{path}")}))).await;
        assert_eq!(s, want, "{path}: {v}");
    }
    for bad in
        ["not a url", "ftp://example.com/a.gif", "file:///etc/passwd", "http://user:pw@example.com/a.gif"]
    {
        let (s, v) = h.call("POST", "/api/v1/gifs", Some(json!({"url": bad}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
}

#[tokio::test]
async fn url_fetch_refuses_private_addresses_by_default() {
    let h = Harness::with(|c| c.fetch.allow_private = false).await;
    let base = serve_files(gif(h.dir.path(), "u.gif", "96x64", 7)).await; // 127.0.0.1:port
    for url in [
        format!("{base}/a.gif"),
        base.replace("127.0.0.1", "localhost") + "/a.gif",
        "http://169.254.169.254/latest/meta-data/".to_string(),
        "http://[::1]/a.gif".to_string(),
        "http://10.0.0.1/a.gif".to_string(),
    ] {
        let (s, v) = h.call("POST", "/api/v1/gifs", Some(json!({"url": url}))).await;
        assert!(s == StatusCode::BAD_REQUEST || s == StatusCode::BAD_GATEWAY, "{url}: {s} {v}");
    }
    assert_eq!(h.app.cfg.tmp_dir().read_dir().unwrap().count(), 0, "no partial downloads left behind");
}

#[tokio::test]
async fn events_stream_reports_changes() {
    let h = Harness::new().await;
    let resp = h.send(h.req("GET", "/api/v1/events", Some(&h.token), None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    let mut body = resp.into_body();

    async fn next_events(body: &mut Body, want: usize) -> String {
        let mut text = String::new();
        while text.matches("\n\n").count() < want {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(20), body.frame())
                .await
                .expect("timed out waiting for an event")
                .expect("stream ended")
                .unwrap();
            if let Some(d) = frame.data_ref() {
                text.push_str(&String::from_utf8_lossy(d));
            }
        }
        text
    }

    let g = h.add_gif(1, "").await; // gif.added then gif.ready
    let id = g["id"].as_str().unwrap().to_string();
    h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({"tags": ["x"]}))).await;
    h.call("DELETE", &format!("/api/v1/gifs/{id}"), None).await;
    let text = next_events(&mut body, 4).await;
    let kinds: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("event: ")).collect();
    assert_eq!(kinds, vec!["gif.added", "gif.ready", "gif.tagged", "gif.deleted"], "{text}");
    assert!(text.contains(&format!("data: {{\"id\":\"{id}\"}}")));
}

/// Many writers at once must all succeed: a deferred SQLite transaction that
/// reads before it writes fails outright (`SQLITE_BUSY_SNAPSHOT`) when another
/// connection commits in between.
#[tokio::test]
async fn concurrent_edits_all_succeed() {
    let h = Harness::new().await;
    let mut ids = vec![];
    for seed in 1..=4 {
        ids.push(h.add_gif(seed, "base").await["id"].as_str().unwrap().to_string());
    }
    let h = std::sync::Arc::new(h);
    let mut tasks = vec![];
    for round in 0..12 {
        for id in &ids {
            let (h, id) = (h.clone(), id.clone());
            tasks.push(tokio::spawn(async move {
                let tags = vec![format!("t{}", round % 4), format!("r{round}"), "shared".to_string()];
                h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({"tags": tags}))).await.0
            }));
        }
    }
    let statuses: Vec<StatusCode> =
        futures_util::future::join_all(tasks).await.into_iter().map(|r| r.unwrap()).collect();
    let failed: Vec<_> = statuses.iter().filter(|s| **s != StatusCode::OK).collect();
    assert!(failed.is_empty(), "{} of {} concurrent edits failed: {failed:?}", failed.len(), statuses.len());

    // Counters still agree with the join table afterwards.
    let (_, t) = h.call("GET", "/api/v1/tags?limit=1000", None).await;
    for tag in t["items"].as_array().unwrap() {
        let name = tag["name"].as_str().unwrap();
        let (_, l) = h.call("GET", &format!("/api/v1/gifs?tag={name}&limit=100"), None).await;
        assert_eq!(
            tag["gif_count"].as_u64().unwrap() as usize,
            l["items"].as_array().unwrap().len(),
            "{name}"
        );
    }
}
