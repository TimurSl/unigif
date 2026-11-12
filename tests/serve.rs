mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::*;

async fn ready_gif(h: &Harness) -> serde_json::Value {
    let src = gif(h.dir.path(), "a.gif", "320x240", 1);
    let (s, v) = h.add("?wait=30", src).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "ready");
    v
}

fn get_range(url: &str, range: &str) -> Request<Body> {
    Request::get(url).header(header::RANGE, range).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn media_is_public_immutable_and_has_an_etag() {
    let h = Harness::new().await;
    let v = ready_gif(&h).await;
    let url = v["media"]["mp4"]["url"].as_str().unwrap();

    // No Authorization header: media URLs are public by unguessable id.
    let resp = h.send(Request::get(url).body(Body::empty()).unwrap()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let hd = resp.headers().clone();
    assert_eq!(hd[header::CONTENT_TYPE], "video/mp4");
    assert_eq!(hd[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
    assert_eq!(hd[header::ACCEPT_RANGES], "bytes");
    assert_eq!(hd["x-content-type-options"], "nosniff");
    let etag = hd[header::ETAG].to_str().unwrap().to_string();
    let body = bytes(resp).await;
    assert_eq!(body.len() as i64, v["media"]["mp4"]["bytes"].as_i64().unwrap());
    assert_eq!(&body[4..8], b"ftyp");

    // Conditional request -> 304.
    let resp =
        h.send(Request::get(url).header(header::IF_NONE_MATCH, &etag).body(Body::empty()).unwrap()).await;
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn range_requests_work() {
    let h = Harness::new().await;
    let v = ready_gif(&h).await;
    let url = v["media"]["mp4"]["url"].as_str().unwrap();
    let all = bytes(h.send(Request::get(url).body(Body::empty()).unwrap()).await).await;
    let len = all.len();

    let resp = h.send(get_range(url, "bytes=0-99")).await;
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(resp.headers()[header::CONTENT_RANGE], format!("bytes 0-99/{len}"));
    assert_eq!(resp.headers()[header::CONTENT_LENGTH], "100");
    assert_eq!(bytes(resp).await, all[..100]);

    // iOS Safari opens with bytes=0-1, then asks for the tail.
    let resp = h.send(get_range(url, "bytes=0-1")).await;
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(bytes(resp).await, all[..2]);

    let resp = h.send(get_range(url, &format!("bytes={}-", len - 10))).await;
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(bytes(resp).await, all[len - 10..]);

    let resp = h.send(get_range(url, "bytes=-10")).await;
    assert_eq!(bytes(resp).await, all[len - 10..]);

    let resp = h.send(get_range(url, &format!("bytes={len}-"))).await;
    assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(resp.headers()[header::CONTENT_RANGE], format!("bytes */{len}"));

    // HEAD returns headers and no body.
    let resp = h.send(Request::head(url).body(Body::empty()).unwrap()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CONTENT_LENGTH], len.to_string());
    assert!(bytes(resp).await.is_empty());
}

#[tokio::test]
async fn only_known_variants_of_known_ids_are_served() {
    let h = Harness::new().await;
    let v = ready_gif(&h).await;
    let id = v["id"].as_str().unwrap();

    for bad in [
        format!("/m/{id}/nope.mp4"),
        format!("/m/{id}/original.gif"),
        format!("/m/{id}/..%2F..%2Funigif.db"),
        "/m/AAAAAAAAAAAA/full.mp4".to_string(),
        "/m/..%2F..%2Fetc/full.mp4".to_string(),
        format!("/m/{id}/full.png"),
    ] {
        let resp = h.send(Request::get(&bad).body(Body::empty()).unwrap()).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{bad}");
    }
    assert_eq!(
        h.send(Request::get(format!("/m/{id}/poster.webp")).body(Body::empty()).unwrap()).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn trashed_items_are_not_served() {
    let h = Harness::new().await;
    let v = ready_gif(&h).await;
    let id = v["id"].as_str().unwrap();
    sqlx::query("UPDATE gifs SET deleted_at = 1 WHERE id = ?").bind(id).execute(&h.app.db).await.unwrap();
    let resp = h.send(Request::get(format!("/m/{id}/full.mp4")).body(Body::empty()).unwrap()).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let resp = h.send(h.get(&format!("/api/v1/gifs/{id}"))).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_requires_a_key() {
    let h = Harness::new().await;
    let v = ready_gif(&h).await;
    let id = v["id"].as_str().unwrap();

    let no_auth = Request::get(format!("/api/v1/gifs/{id}")).body(Body::empty()).unwrap();
    assert_eq!(h.send(no_auth).await.status(), StatusCode::UNAUTHORIZED);

    for bad in ["Bearer nope", "Bearer ugf_abcdefgh_abcdefghijklmnopqrstuvwxyzABCDEF", "Basic abc"] {
        let wrong = Request::get(format!("/api/v1/gifs/{id}"))
            .header("authorization", bad)
            .body(Body::empty())
            .unwrap();
        assert_eq!(h.send(wrong).await.status(), StatusCode::UNAUTHORIZED, "{bad}");
    }

    let x_api_key =
        Request::get(format!("/api/v1/gifs/{id}")).header("x-api-key", &h.token).body(Body::empty()).unwrap();
    assert_eq!(h.send(x_api_key).await.status(), StatusCode::OK);

    let post_no_auth = Request::post("/api/v1/gifs").body(Body::from("x")).unwrap();
    assert_eq!(h.send(post_no_auth).await.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn errors_are_problem_json() {
    let h = Harness::new().await;
    let resp = h.send(h.get("/api/v1/gifs/AAAAAAAAAAAA")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "application/problem+json");
    let (_, v) = json(resp).await;
    assert_eq!(v["code"], "not_found");
    assert_eq!(v["status"], 404);
}

#[tokio::test]
async fn health_and_spa_fallback() {
    let h = Harness::new().await;
    for p in ["/healthz", "/readyz"] {
        assert_eq!(h.send(Request::get(p).body(Body::empty()).unwrap()).await.status(), StatusCode::OK);
    }
    let resp = h.send(Request::get("/unsorted").body(Body::empty()).unwrap()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"));
    assert!(resp.headers().contains_key(header::CONTENT_SECURITY_POLICY));
    let resp = h.send(Request::get("/missing.js").body(Body::empty()).unwrap()).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
