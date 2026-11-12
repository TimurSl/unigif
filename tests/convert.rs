//! M5: on-demand conversions, the cache, the share page.
#![allow(clippy::field_reassign_with_default)]
mod common;

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use common::*;
use unigif::config::ByteSize;

/// A wide fixture so the width caps matter (640 > 480 > 220).
async fn wide(h: &Harness, seed: u32) -> serde_json::Value {
    let bytes = gif(h.dir.path(), &format!("w{seed}.gif"), "640x360", seed);
    let (s, v) = h.add("?wait=60", bytes).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "ready", "{v}");
    v
}

async fn fetch(h: &Harness, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let resp = h.send(Request::get(uri).body(Body::empty()).unwrap()).await;
    (resp.status(), resp.headers().clone(), bytes(resp).await)
}

fn cache_file(h: &Harness, id: &str, name: &str) -> std::path::PathBuf {
    h.app.cfg.cache_gif_dir(id).join(name)
}

/// What the RIFF container of an animated WebP says. ffmpeg 6.x cannot decode
/// animated WebP (see the plan's note on M1), so the test reads the container.
#[derive(Debug, PartialEq, Eq)]
struct Webp {
    width: i64,
    height: i64,
    animated: bool,
    alpha: bool,
    frames: usize,
}

fn parse_webp(d: &[u8]) -> Webp {
    assert!(d.len() > 30 && &d[..4] == b"RIFF" && &d[8..12] == b"WEBP", "not a RIFF/WEBP file");
    assert_eq!(&d[12..16], b"VP8X", "an animated WebP starts with a VP8X chunk");
    let flags = d[20];
    let le24 = |i: usize| d[i] as i64 | (d[i + 1] as i64) << 8 | (d[i + 2] as i64) << 16;
    let (width, height) = (le24(24) + 1, le24(27) + 1);
    // Walk the chunks and count frames.
    let (mut pos, mut frames) = (12, 0);
    while pos + 8 <= d.len() {
        let size = u32::from_le_bytes(d[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if &d[pos..pos + 4] == b"ANMF" {
            frames += 1;
        }
        pos += 8 + size + (size & 1);
    }
    Webp { width, height, animated: flags & 0x02 != 0, alpha: flags & 0x10 != 0, frames }
}

/// Write `data` to disk and ffprobe it: (codec, width, height).
fn identify(h: &Harness, name: &str, data: &[u8]) -> (String, i64, i64) {
    if name.ends_with(".webp") {
        let w = parse_webp(data);
        assert!(w.animated && w.frames > 1, "{w:?}");
        return ("webp".into(), w.width, w.height);
    }
    let path = h.dir.path().join(name);
    std::fs::write(&path, data).unwrap();
    let p = probe(&path);
    let v = video_stream(&p);
    (
        v["codec_name"].as_str().unwrap().to_string(),
        v["width"].as_i64().unwrap(),
        v["height"].as_i64().unwrap(),
    )
}

/// Does any pixel of the first frame of `path` use transparency?
fn has_transparency(path: &std::path::Path) -> bool {
    let out = std::process::Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "format=rgba,alphaextract,signalstats,metadata=mode=print:key=lavfi.signalstats.YMIN:file=-",
            "-f",
            "null",
            "-",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once("YMIN=")?.1.trim().parse::<f64>().ok())
        .any(|m| m < 255.0)
}

#[tokio::test]
async fn gif_object_lists_every_format() {
    let h = Harness::with(|c| c.public_url = "https://gif.example.com".into()).await;
    let v = wide(&h, 1).await;
    let id = v["id"].as_str().unwrap();
    assert_eq!(v["page_url"], format!("https://gif.example.com/g/{id}"));
    for (key, file) in
        [("gif", "full.gif"), ("preview_gif", "preview.gif"), ("webm", "full.webm"), ("webp", "full.webp")]
    {
        assert_eq!(v["media"][key]["url"], format!("https://gif.example.com/m/{id}/{file}"));
        assert_eq!(v["media"][key]["on_demand"], true);
    }
}

#[tokio::test]
async fn every_format_is_converted_once_then_served_from_cache() {
    let h = Harness::new().await;
    let v = wide(&h, 1).await;
    let id = v["id"].as_str().unwrap();

    // name, content type, expected codec, expected size
    let cases = [
        ("full.gif", "image/gif", "gif", (480, 270)),
        ("preview.gif", "image/gif", "gif", (220, 124)),
        ("full.webm", "video/webm", "vp9", (640, 360)),
        ("full.webp", "image/webp", "webp", (480, 270)),
    ];
    for (name, ctype, codec, size) in cases {
        assert!(!cache_file(&h, id, name).exists(), "{name} must not exist before the first request");
        let (s, hd, body) = fetch(&h, &format!("/m/{id}/{name}")).await;
        assert_eq!(s, StatusCode::OK, "{name}");
        assert_eq!(hd[header::CONTENT_TYPE], ctype, "{name}");
        assert_eq!(hd[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
        assert_eq!(hd[header::ACCEPT_RANGES], "bytes");
        let (c, w, hgt) = identify(&h, name, &body);
        assert_eq!((c.as_str(), (w, hgt)), (codec, size), "{name}");

        let on_disk = cache_file(&h, id, name);
        assert_eq!(std::fs::read(&on_disk).unwrap(), body, "{name} is cached verbatim");
        let before = std::fs::metadata(&on_disk).unwrap().modified().unwrap();

        // Second request: same bytes, no new conversion (the file is untouched).
        let (s2, hd2, body2) = fetch(&h, &format!("/m/{id}/{name}")).await;
        assert_eq!(s2, StatusCode::OK);
        assert_eq!(body2, body, "{name}");
        assert_eq!(hd2[header::ETAG], hd[header::ETAG]);
        assert_eq!(std::fs::metadata(&on_disk).unwrap().modified().unwrap(), before, "{name} regenerated");
    }

    // Four rows, tier 'cache', with the real sizes.
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT name, bytes FROM variants WHERE gif_id = ? AND tier = 'cache' ORDER BY name")
            .bind(id)
            .fetch_all(&h.app.db)
            .await
            .unwrap();
    assert_eq!(rows.len(), 4, "{rows:?}");
    for (name, bytes) in rows {
        assert_eq!(std::fs::metadata(cache_file(&h, id, &name)).unwrap().len() as i64, bytes);
    }
    // No stray partial files.
    let leftovers: Vec<_> = std::fs::read_dir(h.app.cfg.cache_gif_dir(id))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // Stored bytes and "saved" numbers are not affected by the cache.
    let (_, st) = h.call("GET", "/api/v1/stats", None).await;
    assert_eq!(st["cache_files"], 4);
    assert!(st["cache_bytes"].as_i64().unwrap() > 0);
    assert_eq!(st["cache_limit"], 2u64 * 1024 * 1024 * 1024);
    let (_, g) = h.call("GET", &format!("/api/v1/gifs/{id}"), None).await;
    assert_eq!(g["media"]["mp4"]["bytes"], v["media"]["mp4"]["bytes"]);
}

#[tokio::test]
async fn ranges_and_etags_work_on_converted_files() {
    let h = Harness::new().await;
    let v = wide(&h, 2).await;
    let id = v["id"].as_str().unwrap();
    let url = format!("/m/{id}/full.webm");
    let (_, hd, all) = fetch(&h, &url).await;
    let resp =
        h.send(Request::get(&url).header(header::RANGE, "bytes=0-9").body(Body::empty()).unwrap()).await;
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(bytes(resp).await, all[..10]);
    let resp = h
        .send(
            Request::get(&url)
                .header(header::IF_NONE_MATCH, hd[header::ETAG].to_str().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    // WebM starts with the EBML magic number.
    assert_eq!(&all[..4], &[0x1a, 0x45, 0xdf, 0xa3]);
}

#[tokio::test]
async fn concurrent_requests_share_one_conversion() {
    let h = Harness::new().await;
    let v = wide(&h, 3).await;
    let id = v["id"].as_str().unwrap().to_string();
    let url = format!("/m/{id}/full.gif");

    let results = futures_util::future::join_all((0..6).map(|_| fetch(&h, &url))).await;
    let first = &results[0].2;
    assert!(!first.is_empty());
    for (s, _, body) in &results {
        assert_eq!(*s, StatusCode::OK);
        assert_eq!(body, first, "every waiter gets the same file");
    }
    assert!(h.app.cfg.cache_gif_dir(&id).read_dir().unwrap().count() == 1, "exactly one file, no partials");
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM variants WHERE gif_id = ? AND tier = 'cache'")
        .bind(&id)
        .fetch_one(&h.app.db)
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn names_outside_the_fixed_list_do_not_exist() {
    let h = Harness::new().await;
    let v = wide(&h, 1).await;
    let id = v["id"].as_str().unwrap();
    for bad in ["full.png", "huge.gif", "full.GIF", "preview.webm", "original.gif"] {
        let (s, _, _) = fetch(&h, &format!("/m/{id}/{bad}")).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{bad}");
    }
    // A query string cannot ask for another size: it is ignored.
    let (s, _, body) = fetch(&h, &format!("/m/{id}/full.gif?w=9000")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(identify(&h, "q.gif", &body).1, 480);
    // And nothing outside the four names ended up in the cache.
    let mut names: Vec<String> = h
        .app
        .cfg
        .cache_gif_dir(id)
        .read_dir()
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert_eq!(names, ["full.gif"]);
}

#[tokio::test]
async fn trashed_gifs_are_not_converted_and_purging_empties_their_cache() {
    let h = Harness::new().await;
    let v = wide(&h, 1).await;
    let id = v["id"].as_str().unwrap().to_string();
    assert_eq!(fetch(&h, &format!("/m/{id}/full.gif")).await.0, StatusCode::OK);
    assert!(cache_file(&h, &id, "full.gif").exists());

    h.call("DELETE", &format!("/api/v1/gifs/{id}"), None).await;
    assert_eq!(fetch(&h, &format!("/m/{id}/full.gif")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(fetch(&h, &format!("/m/{id}/full.webm")).await.0, StatusCode::NOT_FOUND);
    assert!(!cache_file(&h, &id, "full.webm").exists(), "nothing is converted for a trashed GIF");

    let later = unigif::db::now_ms() + 31 * 86_400_000;
    assert_eq!(unigif::janitor::purge_trash(&h.app, later).await.unwrap(), 1);
    assert!(!h.app.cfg.cache_gif_dir(&id).exists());
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM variants WHERE tier = 'cache'")
        .fetch_one(&h.app.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn a_failed_conversion_is_an_error_not_a_hang_and_leaves_no_files() {
    // A zero-second ffmpeg budget makes every on-demand conversion time out.
    let h = Harness::with(|c| c.media.timeout_secs = 0).await;
    let v = wide(&h, 1).await;
    let id = v["id"].as_str().unwrap();
    for _ in 0..2 {
        // The second round proves the failed attempt did not stay in the in-flight map.
        let resp = h.send(Request::get(format!("/m/{id}/full.gif")).body(Body::empty()).unwrap()).await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let (_, body) = json(resp).await;
        assert_eq!(body["code"], "internal");
        assert!(!body.to_string().contains("ffmpeg"), "internals stay in the log: {body}");
    }
    assert!(h.app.cfg.cache_gif_dir(id).read_dir().map(|mut d| d.next().is_none()).unwrap_or(true));
    // Stored variants are unaffected.
    assert_eq!(fetch(&h, &format!("/m/{id}/full.mp4")).await.0, StatusCode::OK);
}

#[tokio::test]
async fn transparent_gifs_convert_from_the_original_and_keep_alpha() {
    let h = Harness::new().await;
    let path = h.dir.path().join("alpha.gif");
    transparent_gif_fixture(&path);
    let (s, v) = h.add("?wait=60", std::fs::read(&path).unwrap()).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["has_alpha"], true);
    let id = v["id"].as_str().unwrap();

    // The stored MP4 is on the matte, so the original is the only source of alpha.
    let (s, _, body) = fetch(&h, &format!("/m/{id}/full.gif")).await;
    assert_eq!(s, StatusCode::OK);
    let out = h.dir.path().join("alpha-out.gif");
    std::fs::write(&out, body).unwrap();
    assert!(has_transparency(&out), "full.gif lost its transparency");
    let (s, _, body) = fetch(&h, &format!("/m/{id}/full.webp")).await;
    assert_eq!(s, StatusCode::OK);
    let w = parse_webp(&body);
    assert!(w.alpha && w.animated, "full.webp lost its transparency: {w:?}");
    // WebM comes from the MP4 and has none.
    let (s, _, body) = fetch(&h, &format!("/m/{id}/full.webm")).await;
    assert_eq!(s, StatusCode::OK);
    let out = h.dir.path().join("alpha-out.webm");
    std::fs::write(&out, body).unwrap();
    assert!(!has_transparency(&out));
}

#[tokio::test]
async fn gifs_without_a_kept_original_convert_from_the_mp4() {
    let h = Harness::new().await;
    let v = wide(&h, 5).await; // opaque: the original is not kept
    let id = v["id"].as_str().unwrap();
    assert!(!media_path(&h, id, "original.gif").exists());
    let (s, _, body) = fetch(&h, &format!("/m/{id}/full.gif")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(&body[..4], b"GIF8");
    // Animated, not a single frame: the duration survives the round trip.
    let path = h.dir.path().join("rt.gif");
    std::fs::write(&path, body).unwrap();
    let dur: f64 = probe(&path)["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((dur - 1.5).abs() < 0.4, "duration {dur}");
}

#[tokio::test]
async fn the_cache_stays_under_its_limit_and_evicts_least_recently_used_first() {
    // Measure the three files with an unconstrained cache first.
    let probe_h = Harness::new().await;
    let mut sizes = vec![];
    for seed in 1..=3 {
        let v = wide(&probe_h, seed).await;
        let id = v["id"].as_str().unwrap();
        sizes.push(fetch(&probe_h, &format!("/m/{id}/full.webp")).await.2.len() as u64);
    }
    let [a, b, c] = [sizes[0], sizes[1], sizes[2]];
    let limit = (a + b).max(b + c);
    assert!(limit < a + b + c);

    let h = Harness::with(|cfg| cfg.cache.max_size = ByteSize(limit)).await;
    let mut ids = vec![];
    for seed in 1..=3 {
        let v = wide(&h, seed).await;
        let id = v["id"].as_str().unwrap().to_string();
        assert_eq!(fetch(&h, &format!("/m/{id}/full.webp")).await.0, StatusCode::OK);
        // Distinct last-access times even on a coarse clock.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        ids.push(id);
    }
    let on_disk = |i: usize| cache_file(&h, &ids[i], "full.webp").exists();
    // A (the oldest) went; B and C stayed.
    assert_eq!([on_disk(0), on_disk(1), on_disk(2)], [false, true, true]);

    // Use B, then ask for A again: C is now the least recently used.
    assert_eq!(fetch(&h, &format!("/m/{}/full.webp", ids[1])).await.0, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    assert_eq!(fetch(&h, &format!("/m/{}/full.webp", ids[0])).await.0, StatusCode::OK);
    assert_eq!([on_disk(0), on_disk(1), on_disk(2)], [true, true, false]);

    let (_, bytes_now): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(bytes), 0) FROM variants WHERE tier = 'cache'")
            .fetch_one(&h.app.db)
            .await
            .unwrap();
    assert!(bytes_now as u64 <= limit, "{bytes_now} > {limit}");
    // The evicted GIF converts again on demand.
    assert_eq!(fetch(&h, &format!("/m/{}/full.webp", ids[2])).await.0, StatusCode::OK);
}

#[tokio::test]
async fn the_janitor_trims_and_reconciles_the_cache() {
    // A limit of one byte: every file is over it as soon as it is not the one being served.
    let h = Harness::with(|cfg| cfg.cache.max_size = ByteSize(1)).await;
    let v = wide(&h, 1).await;
    let id = v["id"].as_str().unwrap().to_string();
    assert_eq!(fetch(&h, &format!("/m/{id}/full.gif")).await.0, StatusCode::OK);
    assert!(cache_file(&h, &id, "full.gif").exists(), "the file being served is never evicted");
    unigif::janitor::run_once(&h.app).await.unwrap();
    assert!(!cache_file(&h, &id, "full.gif").exists(), "the janitor applies the limit");

    // Deleting the cache directory by hand is safe: rows are reconciled and the file comes back.
    let h = Harness::new().await;
    let v = wide(&h, 1).await;
    let id = v["id"].as_str().unwrap().to_string();
    assert_eq!(fetch(&h, &format!("/m/{id}/full.gif")).await.0, StatusCode::OK);
    std::fs::remove_dir_all(&h.app.cfg.cache_dir).unwrap();
    std::fs::create_dir_all(&h.app.cfg.cache_dir).unwrap();
    assert_eq!(fetch(&h, &format!("/m/{id}/full.gif")).await.0, StatusCode::OK);
    assert!(cache_file(&h, &id, "full.gif").exists());
}

// ---------------------------------------------------------------------------
// Share page
// ---------------------------------------------------------------------------

#[tokio::test]
async fn share_page_has_open_graph_tags_and_no_private_data() {
    let h = Harness::with(|c| c.public_url = "https://gif.example.com".into()).await;
    let bytes = gif(h.dir.path(), "s.gif", "640x360", 1);
    let (_, v) = h.add("?wait=60&tags=secrettag&description=private%20note", bytes).await;
    let id = v["id"].as_str().unwrap();

    let (s, hd, body) = fetch(&h, &format!("/g/{id}")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(hd[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"));
    assert!(hd.contains_key(header::CONTENT_SECURITY_POLICY));
    assert_eq!(hd["x-robots-tag"], "noindex");
    let html = String::from_utf8(body).unwrap();
    for needle in [
        format!(r#"property="og:video" content="https://gif.example.com/m/{id}/full.mp4""#),
        format!(r#"property="og:video:secure_url" content="https://gif.example.com/m/{id}/full.mp4""#),
        r#"property="og:video:type" content="video/mp4""#.to_string(),
        r#"property="og:video:width" content="640""#.to_string(),
        r#"property="og:video:height" content="360""#.to_string(),
        format!(r#"property="og:image" content="https://gif.example.com/m/{id}/poster.webp""#),
        format!(r#"property="og:url" content="https://gif.example.com/g/{id}""#),
        r#"name="twitter:card" content="summary_large_image""#.to_string(),
        format!(r#"<video src="/m/{id}/full.mp4""#),
    ] {
        assert!(html.contains(&needle), "missing {needle} in:\n{html}");
    }
    assert!(!html.contains("secrettag") && !html.contains("private note"), "the page leaks organisation");
    assert!(!html.contains("<script"), "the strict CSP allows no scripts");
}

#[tokio::test]
async fn share_page_urls_follow_the_request_when_no_public_url_is_set() {
    let h = Harness::new().await;
    let v = wide(&h, 1).await;
    let id = v["id"].as_str().unwrap();
    let resp = h
        .send(
            Request::get(format!("/g/{id}"))
                .header(header::HOST, "gifs.test:8787\"><script>")
                .header("x-forwarded-proto", "https")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let html = String::from_utf8(bytes(resp).await).unwrap();
    assert!(
        html.contains("https://gifs.test:8787&quot;&gt;&lt;script&gt;/m/"),
        "host must be escaped:\n{html}"
    );
    assert!(!html.contains("<script>"));
}

#[tokio::test]
async fn share_links_redirect_to_the_files() {
    let h = Harness::new().await;
    let v = wide(&h, 1).await;
    let id = v["id"].as_str().unwrap();
    for (ext, file) in
        [("mp4", "full.mp4"), ("gif", "full.gif"), ("webm", "full.webm"), ("webp", "full.webp")]
    {
        let (s, hd, _) = fetch(&h, &format!("/g/{id}.{ext}")).await;
        assert_eq!(s, StatusCode::TEMPORARY_REDIRECT, "{ext}");
        assert_eq!(hd[header::LOCATION], format!("/m/{id}/{file}"));
    }
    // The redirect leads to a working file.
    assert_eq!(fetch(&h, &format!("/m/{id}/full.gif")).await.0, StatusCode::OK);

    for bad in [
        format!("/g/{id}.png"),
        format!("/g/{id}.mp4.gif"),
        "/g/AAAAAAAAAAAA".to_string(),
        "/g/short".to_string(),
        format!("/g/{id}."),
    ] {
        assert_eq!(fetch(&h, &bad).await.0, StatusCode::NOT_FOUND, "{bad}");
    }
}

#[tokio::test]
async fn trashed_gifs_have_no_share_page() {
    let h = Harness::new().await;
    let v = wide(&h, 1).await;
    let id = v["id"].as_str().unwrap();
    assert_eq!(fetch(&h, &format!("/g/{id}")).await.0, StatusCode::OK);
    h.call("DELETE", &format!("/api/v1/gifs/{id}"), None).await;
    assert_eq!(fetch(&h, &format!("/g/{id}")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(fetch(&h, &format!("/g/{id}.gif")).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn emptying_the_trash_purges_files_and_cache_and_honours_hidden_hashtags() {
    let h = Harness::new().await;
    let a = wide(&h, 1).await["id"].as_str().unwrap().to_string();
    let b = h.add_gif(2, "secret").await["id"].as_str().unwrap().to_string();
    let keep = h.add_gif(3, "cat").await["id"].as_str().unwrap().to_string();
    assert_eq!(fetch(&h, &format!("/m/{a}/full.gif")).await.0, StatusCode::OK);
    for id in [&a, &b] {
        assert_eq!(h.call("DELETE", &format!("/api/v1/gifs/{id}"), None).await.0, StatusCode::NO_CONTENT);
    }

    // Needs the delete scope.
    let reader = h.mint(unigif::auth::Scopes::from_slice(&[unigif::auth::Scope::Read]), &[]).await;
    let resp = h.send(h.req("POST", "/api/v1/gifs/trash/empty", Some(&reader), None)).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // A key that cannot see #secret empties only what it can see.
    let limited = h.mint(unigif::auth::Scopes::all(), &["secret"]).await;
    let (s, v) = json(h.send(h.req("POST", "/api/v1/gifs/trash/empty", Some(&limited), None)).await).await;
    assert_eq!((s, v["purged"].as_i64()), (StatusCode::OK, Some(1)));
    assert!(unigif::db::gifs::get(&h.app.db, &a).await.unwrap().is_none());
    assert!(!media_path(&h, &a, "full.mp4").exists(), "stored files go");
    assert!(!h.app.cfg.cache_gif_dir(&a).exists(), "cached conversions go");
    assert!(unigif::db::gifs::get(&h.app.db, &b).await.unwrap().is_some(), "hidden GIF untouched");

    // The owner empties the rest; GIFs outside the trash are never touched.
    let (s, v) = h.call("POST", "/api/v1/gifs/trash/empty", None).await;
    assert_eq!((s, v["purged"].as_i64()), (StatusCode::OK, Some(1)));
    let (_, st) = h.call("GET", "/api/v1/stats", None).await;
    assert_eq!((st["gifs"].as_i64(), st["trash"].as_i64()), (Some(1), Some(0)));
    assert!(media_path(&h, &keep, "full.mp4").exists());
    // #secret only lived on a purged GIF, so the hashtag is gone too.
    let (_, t) = h.call("GET", "/api/v1/tags", None).await;
    let names: Vec<&str> =
        t["items"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["cat"]);
    // Nothing left: still a 200.
    let (s, v) = h.call("POST", "/api/v1/gifs/trash/empty", None).await;
    assert_eq!((s, v["purged"].as_i64()), (StatusCode::OK, Some(0)));
}

#[tokio::test]
async fn clearing_the_cache_removes_every_converted_file() {
    let h = Harness::new().await;
    let id = wide(&h, 1).await["id"].as_str().unwrap().to_string();
    let mut total = 0;
    for name in ["full.gif", "full.webm"] {
        let (s, _, body) = fetch(&h, &format!("/m/{id}/{name}")).await;
        assert_eq!(s, StatusCode::OK);
        total += body.len() as u64;
    }
    // A stray partial file stands in for a conversion in progress.
    std::fs::write(cache_file(&h, &id, "full.webp.part"), b"half").unwrap();

    // Admin only: a full-but-not-admin key is refused.
    let full = h.mint(unigif::auth::Scopes::preset("full").unwrap(), &[]).await;
    let resp = h.send(h.req("DELETE", "/api/v1/cache", Some(&full), None)).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(cache_file(&h, &id, "full.gif").exists());

    let (s, v) = h.call("DELETE", "/api/v1/cache", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["files"].as_u64(), v["bytes"].as_u64()), (Some(2), Some(total)));
    assert!(!cache_file(&h, &id, "full.gif").exists());
    assert!(!cache_file(&h, &id, "full.webm").exists());
    assert!(cache_file(&h, &id, "full.webp.part").exists(), "work in progress is left alone");
    let (_, st) = h.call("GET", "/api/v1/stats", None).await;
    assert_eq!((st["cache_files"].as_i64(), st["cache_bytes"].as_i64()), (Some(0), Some(0)));
    // Stored media is untouched, and the next request converts again.
    assert_eq!(fetch(&h, &format!("/m/{id}/full.mp4")).await.0, StatusCode::OK);
    assert_eq!(fetch(&h, &format!("/m/{id}/full.gif")).await.0, StatusCode::OK);
    assert!(cache_file(&h, &id, "full.gif").exists());
}

#[tokio::test]
async fn the_gif_object_reports_the_original_size() {
    let h = Harness::new().await;
    let bytes = gif(h.dir.path(), "o.gif", "96x64", 7);
    let len = bytes.len() as i64;
    let (s, v) = h.add("?wait=30", bytes).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(v["bytes_original"].as_i64(), Some(len));
    assert_eq!(v["original_kept"], false, "an opaque GIF is not kept by default");

    let path = h.dir.path().join("alpha.gif");
    transparent_gif_fixture(&path);
    let (_, v) = h.add("?wait=60", std::fs::read(&path).unwrap()).await;
    assert_eq!(v["original_kept"], true, "a transparent GIF is");
}

#[tokio::test]
async fn convert_now_warms_the_cache_without_waiting() {
    let h = Harness::new().await;
    let id = wide(&h, 1).await["id"].as_str().unwrap().to_string();
    let (s, v) = h
        .call(
            "POST",
            &format!("/api/v1/gifs/{id}/convert"),
            Some(serde_json::json!({"variants": ["gif", "webp"]})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["started"], serde_json::json!(["full.gif", "full.webp"]));
    // The conversions finish in the background.
    for _ in 0..300 {
        if cache_file(&h, &id, "full.gif").exists() && cache_file(&h, &id, "full.webp").exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(cache_file(&h, &id, "full.gif").exists());
    assert!(cache_file(&h, &id, "full.webp").exists());
    assert!(!cache_file(&h, &id, "full.webm").exists(), "only what was asked for");

    // `{}` means all four; bad names and unknown GIFs are refused.
    let (s, v) = h.call("POST", &format!("/api/v1/gifs/{id}/convert"), Some(serde_json::json!({}))).await;
    assert_eq!((s, v["started"].as_array().unwrap().len()), (StatusCode::ACCEPTED, 4));
    let (s, _) = h
        .call("POST", &format!("/api/v1/gifs/{id}/convert"), Some(serde_json::json!({"variants": ["mp4"]})))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = h.call("POST", "/api/v1/gifs/AAAAAAAAAAAA/convert", Some(serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // A key that cannot see the GIF cannot warm it either.
    let (_, _) =
        h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(serde_json::json!({"tags": ["secret"]}))).await;
    let limited = h.mint(unigif::auth::Scopes::preset("read_only").unwrap(), &["secret"]).await;
    let resp = h
        .send(h.req(
            "POST",
            &format!("/api/v1/gifs/{id}/convert"),
            Some(&limited),
            Some(serde_json::json!({})),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
