#![allow(clippy::field_reassign_with_default)]
mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use serde_json::json;
use unigif::auth::{Scope, Scopes};

fn scopes(list: &[Scope]) -> Scopes {
    Scopes::from_slice(list)
}

async fn status(
    h: &Harness,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    json(h.send(h.req(method, uri, Some(token), body)).await).await
}

#[tokio::test]
async fn scopes_gate_every_operation() {
    let h = Harness::new().await;
    let g = h.add_gif(1, "cat").await;
    let id = g["id"].as_str().unwrap().to_string();
    let bytes = gif(h.dir.path(), "s.gif", "96x64", 9);

    let read = h.mint(scopes(&[Scope::Read]), &[]).await;
    let upload = h.mint(scopes(&[Scope::Upload]), &[]).await;
    let tag = h.mint(scopes(&[Scope::Tag]), &[]).await;
    let delete = h.mint(scopes(&[Scope::Delete]), &[]).await;

    // Read-only: can look, cannot touch.
    assert_eq!(status(&h, "GET", &format!("/api/v1/gifs/{id}"), &read, None).await.0, StatusCode::OK);
    assert_eq!(status(&h, "GET", "/api/v1/gifs", &read, None).await.0, StatusCode::OK);
    assert_eq!(status(&h, "GET", "/api/v1/tags", &read, None).await.0, StatusCode::OK);
    assert_eq!(status(&h, "GET", "/api/v1/stats", &read, None).await.0, StatusCode::OK);
    let resp = h.send(h.post("/api/v1/gifs", "image/gif", bytes.clone())).await; // full key works…
    assert_eq!(resp.status(), StatusCode::CREATED);
    let resp = h
        .send(
            Request::post("/api/v1/gifs")
                .header("authorization", format!("Bearer {read}"))
                .header("content-type", "image/gif")
                .body(Body::from(bytes.clone()))
                .unwrap(),
        )
        .await; // …read key does not
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let (_, v) = json(resp).await;
    assert_eq!(v["code"], "scope_required");
    assert!(v["detail"].as_str().unwrap().contains("upload"));
    for (m, uri, body) in [
        ("PATCH", format!("/api/v1/gifs/{id}"), Some(json!({"tags": ["x"]}))),
        ("DELETE", format!("/api/v1/gifs/{id}"), None),
        ("POST", format!("/api/v1/gifs/{id}/restore"), None),
        ("POST", "/api/v1/gifs/batch".into(), Some(json!({"ids": [id], "trash": true}))),
        ("PATCH", "/api/v1/tags/cat".into(), Some(json!({"name": "dog"}))),
        ("GET", "/api/v1/keys".into(), None),
        ("POST", "/api/v1/keys".into(), Some(json!({"name": "x", "preset": "full"}))),
        ("GET", "/api/v1/gifs?trash=true".into(), None),
    ] {
        let (s, v) = status(&h, m, &uri, &read, body).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("scope_required")), "{m} {uri}");
    }

    // Uploader: can add, cannot browse.
    let resp = h
        .send(
            Request::post("/api/v1/gifs?tags=new")
                .header("authorization", format!("Bearer {upload}"))
                .header("content-type", "image/gif")
                .body(Body::from(gif(h.dir.path(), "s2.gif", "96x64", 3)))
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    for uri in [
        format!("/api/v1/gifs/{id}"),
        "/api/v1/gifs".into(),
        "/api/v1/tags".into(),
        "/api/v1/stats".into(),
        "/api/v1/events".into(),
    ] {
        assert_eq!(status(&h, "GET", &uri, &upload, None).await.0, StatusCode::FORBIDDEN, "{uri}");
    }
    // Tag-only and delete-only keys cannot read or cross over.
    assert_eq!(
        status(&h, "PATCH", &format!("/api/v1/gifs/{id}"), &tag, Some(json!({"tags": ["y"]}))).await.0,
        StatusCode::OK
    );
    assert_eq!(
        status(&h, "DELETE", &format!("/api/v1/gifs/{id}"), &tag, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(status(&h, "GET", &format!("/api/v1/gifs/{id}"), &tag, None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        status(&h, "DELETE", &format!("/api/v1/gifs/{id}"), &delete, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        status(&h, "PATCH", &format!("/api/v1/gifs/{id}"), &delete, Some(json!({"tags": ["z"]}))).await.0,
        StatusCode::FORBIDDEN
    );

    // /me describes the key.
    let (s, me) = status(&h, "GET", "/api/v1/me", &read, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((me["kind"].as_str(), me["scopes"].clone()), (Some("key"), json!(["read"])));
}

#[tokio::test]
async fn revoked_expired_and_malformed_keys_are_refused() {
    let h = Harness::new().await;
    let (s, k) = h.call("POST", "/api/v1/keys", Some(json!({"name": "temp", "preset": "read_only"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{k}");
    let token = k["key"].as_str().unwrap().to_string();
    assert!(token.starts_with("ugf_"));
    assert_eq!(status(&h, "GET", "/api/v1/me", &token, None).await.0, StatusCode::OK);

    // Wrong secret, right id.
    let (id, _) = unigif::auth::keys::parse(&token).unwrap();
    let forged = format!("ugf_{id}_{}", "A".repeat(32));
    assert_eq!(status(&h, "GET", "/api/v1/me", &forged, None).await.0, StatusCode::UNAUTHORIZED);

    // Expiry is honoured on every request.
    sqlx::query("UPDATE api_keys SET expires_at = 1 WHERE id = ?").bind(id).execute(&h.app.db).await.unwrap();
    assert_eq!(status(&h, "GET", "/api/v1/me", &token, None).await.0, StatusCode::UNAUTHORIZED);
    sqlx::query("UPDATE api_keys SET expires_at = NULL WHERE id = ?")
        .bind(id)
        .execute(&h.app.db)
        .await
        .unwrap();
    assert_eq!(status(&h, "GET", "/api/v1/me", &token, None).await.0, StatusCode::OK);

    // Revocation too.
    assert_eq!(h.call("DELETE", &format!("/api/v1/keys/{id}"), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(status(&h, "GET", "/api/v1/me", &token, None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(h.call("DELETE", &format!("/api/v1/keys/{id}"), None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn keys_in_the_url_are_rejected() {
    let h = Harness::new().await;
    for q in [format!("api_key={}", h.token), format!("foo={}", h.token), format!("token={}", h.token)] {
        let resp = h.send(Request::get(format!("/api/v1/me?{q}")).body(Body::empty()).unwrap()).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{q}");
    }
    // Even with a perfectly good header alongside.
    let resp = h.send(h.req("GET", &format!("/api/v1/me?api_key={}", h.token), Some(&h.token), None)).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn key_management_lifecycle() {
    let h = Harness::new().await;
    let (s, k) = h
        .call(
            "POST",
            "/api/v1/keys",
            Some(json!({"name": "Alex (friend)", "scopes": ["read", "upload"], "deny_tags": ["#Private"], "rate_limit": 50, "expires_in_days": 30})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{k}");
    assert_eq!(k["scopes"], json!(["read", "upload"]));
    assert_eq!(k["deny_tags"], json!(["private"]));
    assert_eq!(k["rate_limit"], 50);
    assert!(k["expires_at"].is_string());
    let id = k["id"].as_str().unwrap();

    // Listed, but the secret is never shown again.
    let (_, list) = h.call("GET", "/api/v1/keys", None).await;
    let listed = list["items"].as_array().unwrap().iter().find(|x| x["id"] == id).unwrap();
    assert!(listed.get("key").is_none());
    assert!(!list.to_string().contains(k["key"].as_str().unwrap()));
    // Only the hash is stored.
    let row = unigif::db::keys::get(&h.app.db, id).await.unwrap().unwrap();
    assert_eq!(row.secret_hash.len(), 32);

    // Patch: change scopes, clear the rate limit and expiry.
    let (s, k2) = h
        .call(
            "PATCH",
            &format!("/api/v1/keys/{id}"),
            Some(json!({"preset": "picker", "rate_limit": null, "expires_at": null, "name": "Alex"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{k2}");
    assert_eq!(
        (k2["name"].as_str(), k2["rate_limit"].is_null(), k2["expires_at"].is_null()),
        (Some("Alex"), true, true)
    );
    assert_eq!(k2["deny_tags"], json!(["private"]), "untouched fields stay");

    // last_used_at is filled once the key is used.
    let token = k["key"].as_str().unwrap();
    assert_eq!(status(&h, "GET", "/api/v1/me", token, None).await.0, StatusCode::OK);
    let (_, list) = h.call("GET", "/api/v1/keys", None).await;
    let listed = list["items"].as_array().unwrap().iter().find(|x| x["id"] == id).unwrap();
    assert!(listed["last_used_at"].is_string());

    // Revoked keys drop out of the list unless asked for.
    h.call("DELETE", &format!("/api/v1/keys/{id}"), None).await;
    let (_, list) = h.call("GET", "/api/v1/keys", None).await;
    assert!(list["items"].as_array().unwrap().iter().all(|x| x["id"] != id));
    let (_, list) = h.call("GET", "/api/v1/keys?include_revoked=true", None).await;
    assert!(list["items"].as_array().unwrap().iter().any(|x| x["id"] == id && x["revoked_at"].is_string()));

    // Validation.
    for body in [
        json!({"name": "x"}),
        json!({"name": "x", "preset": "full", "scopes": ["read"]}),
        json!({"name": "x", "preset": "root"}),
        json!({"name": "x", "scopes": ["read", "wizard"]}),
        json!({"name": "", "preset": "full"}),
        json!({"name": "x", "preset": "full", "rate_limit": 0}),
        json!({"name": "x", "preset": "full", "expires_at": "yesterday"}),
        json!({"name": "x", "preset": "full", "expires_at": "2001-01-01T00:00:00Z"}),
        json!({"name": "x", "preset": "full", "deny_tags": ["no good"]}),
    ] {
        let (s, v) = h.call("POST", "/api/v1/keys", Some(body.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body}: {v}");
    }
}

#[tokio::test]
async fn hidden_hashtags_are_invisible_everywhere() {
    let h = Harness::new().await;
    let open = h.add_gif(1, "cat,funny").await;
    let secret = h.add_gif(2, "cat,private").await;
    let (open, secret) =
        (open["id"].as_str().unwrap().to_string(), secret["id"].as_str().unwrap().to_string());
    let friend = h.mint(scopes(&[Scope::Read, Scope::Upload, Scope::Tag, Scope::Delete]), &["private"]).await;

    // Lists and search.
    let (_, v) = status(&h, "GET", "/api/v1/gifs", &friend, None).await;
    assert_eq!(Harness::ids(&v), vec![open.clone()]);
    let (_, v) = status(&h, "GET", "/api/v1/gifs?q=cat", &friend, None).await;
    assert_eq!(Harness::ids(&v), vec![open.clone()]);
    let (_, v) = status(&h, "GET", "/api/v1/gifs?tag=private", &friend, None).await;
    assert!(Harness::ids(&v).is_empty());
    // By id, for every verb: the GIF simply does not exist for this key.
    for (m, uri, body) in [
        ("GET", format!("/api/v1/gifs/{secret}"), None),
        ("PATCH", format!("/api/v1/gifs/{secret}"), Some(json!({"tags": ["x"]}))),
        ("POST", format!("/api/v1/gifs/{secret}/tags"), Some(json!({"add": ["x"]}))),
        ("DELETE", format!("/api/v1/gifs/{secret}"), None),
        ("POST", format!("/api/v1/gifs/{secret}/use"), None),
    ] {
        assert_eq!(status(&h, m, &uri, &friend, body).await.0, StatusCode::NOT_FOUND, "{m} {uri}");
    }
    let (_, v) =
        status(&h, "POST", "/api/v1/gifs/batch", &friend, Some(json!({"ids": [open, secret], "add": ["z"]})))
            .await;
    assert_eq!((v["updated"].as_i64(), v["not_found"].clone()), (Some(1), json!([secret])));
    let (_, g) = h.call("GET", &format!("/api/v1/gifs/{secret}"), None).await;
    assert_eq!(g["tags"], json!(["cat", "private"]), "the hidden GIF was not touched");

    // Tag listings and counts only reflect what the key can see.
    let (_, t) = status(&h, "GET", "/api/v1/tags?sort=popular", &friend, None).await;
    let names: Vec<_> = t["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| (x["name"].as_str().unwrap().to_string(), x["gif_count"].as_i64().unwrap()))
        .collect();
    assert!(names.contains(&("cat".into(), 1)) && !names.iter().any(|(n, _)| n == "private"), "{names:?}");
    let (_, t) = status(&h, "GET", "/api/v1/tags?q=priv", &friend, None).await;
    assert!(t["items"].as_array().unwrap().is_empty());
    let (_, sg) = status(&h, "GET", "/api/v1/tags/suggest?selected=cat", &friend, None).await;
    assert!(!sg.to_string().contains("private"));
    let (_, st) = status(&h, "GET", "/api/v1/stats", &friend, None).await;
    // The friend sees one GIF carrying cat, funny and (from the batch above) z.
    assert_eq!((st["gifs"].as_i64(), st["tags"].as_i64()), (Some(1), Some(3)));
    let (_, st_owner) = h.call("GET", "/api/v1/stats", None).await;
    assert_eq!((st_owner["gifs"].as_i64(), st_owner["tags"].as_i64()), (Some(2), Some(4)));
    let (_, next) = status(&h, "GET", "/api/v1/gifs/unsorted/next", &friend, None).await;
    assert!(next["item"].is_null());

    // The key may not create the problem it is protected from, nor reshape the library.
    let (s, _) =
        status(&h, "PATCH", &format!("/api/v1/gifs/{open}"), &friend, Some(json!({"tags": ["private"]})))
            .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) =
        status(&h, "POST", &format!("/api/v1/gifs/{open}/tags"), &friend, Some(json!({"add": ["private"]})))
            .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = status(&h, "POST", "/api/v1/gifs?tags=private", &friend, None).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::BAD_REQUEST);
    for (m, uri, body) in [
        ("PATCH", "/api/v1/tags/cat", Some(json!({"name": "feline"}))),
        ("POST", "/api/v1/tags/cat/merge", Some(json!({"into": "funny"}))),
        ("DELETE", "/api/v1/tags/cat", None),
    ] {
        assert_eq!(status(&h, m, uri, &friend, body).await.0, StatusCode::FORBIDDEN, "{m} {uri}");
    }
}

#[tokio::test]
async fn hidden_hashtags_filter_the_event_stream_and_uploads() {
    let h = Harness::new().await;
    let secret = h.add_gif(2, "private").await;
    let secret = secret["id"].as_str().unwrap().to_string();
    let friend = h.mint(scopes(&[Scope::Read, Scope::Upload]), &["private"]).await;

    // Re-uploading the hidden bytes reveals neither its tags nor its media.
    let bytes = gif(h.dir.path(), "g2.gif", "96x64", 2);
    let resp = h
        .send(
            Request::post("/api/v1/gifs?tags=mine")
                .header("authorization", format!("Bearer {friend}"))
                .header("content-type", "image/gif")
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await;
    let (s, v) = json(resp).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["duplicate"], true);
    assert_eq!(v["tags"], json!(["mine"]));
    assert!(v["media"].is_null());

    // Uploader keys without `read` get the same treatment.
    let up = h.mint(scopes(&[Scope::Upload]), &[]).await;
    let bytes = gif(h.dir.path(), "g2.gif", "96x64", 2);
    let resp = h
        .send(
            Request::post("/api/v1/gifs")
                .header("authorization", format!("Bearer {up}"))
                .header("content-type", "image/gif")
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await;
    let (_, v) = json(resp).await;
    assert_eq!(v["duplicate"], true);
    assert_eq!(v["tags"], json!([]));

    // Events about the hidden GIF never reach the friend; others do.
    let resp = h.send(h.req("GET", "/api/v1/events", Some(&friend), None)).await;
    let mut body = resp.into_body();
    h.call("PATCH", &format!("/api/v1/gifs/{secret}"), Some(json!({"tags": ["private", "more"]}))).await; // hidden: dropped
    let other = h.add_gif(3, "open").await; // visible
    let mut text = String::new();
    use http_body_util::BodyExt;
    while !text.contains("gif.ready") {
        let f = tokio::time::timeout(std::time::Duration::from_secs(20), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Some(d) = f.data_ref() {
            text.push_str(&String::from_utf8_lossy(d));
        }
    }
    assert!(!text.contains(&secret), "{text}");
    assert!(text.contains(other["id"].as_str().unwrap()));
}

#[tokio::test]
async fn per_key_rate_limits_with_headers() {
    let h = Harness::with(|c| c.rate_limit.read_per_min = 5).await;
    let token = h.mint(scopes(&[Scope::Read]), &[]).await;
    let mut last_remaining = 5u32;
    for i in 0..5 {
        let resp = h.send(h.req("GET", "/api/v1/me", Some(&token), None)).await;
        assert_eq!(resp.status(), StatusCode::OK, "request {i}");
        assert_eq!(resp.headers()["ratelimit-limit"], "5");
        let rem: u32 = resp.headers()["ratelimit-remaining"].to_str().unwrap().parse().unwrap();
        assert!(rem < last_remaining);
        last_remaining = rem;
    }
    let resp = h.send(h.req("GET", "/api/v1/me", Some(&token), None)).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers()["retry-after"].to_str().unwrap().parse::<u64>().unwrap() >= 1);
    assert_eq!(resp.headers()["ratelimit-remaining"], "0");
    let (_, v) = json(resp).await;
    assert_eq!(v["code"], "rate_limited");
    // Other keys have their own buckets.
    assert_eq!(h.send(h.get("/api/v1/me")).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn upload_class_is_limited_separately_and_key_override_wins() {
    let h = Harness::with(|c| c.rate_limit.upload_per_min = 1).await;
    let up = h.mint(scopes(&[Scope::Upload, Scope::Read]), &[]).await;
    let post = |seed: u32| {
        let bytes = gif(h.dir.path(), &format!("r{seed}.gif"), "64x48", seed);
        Request::post("/api/v1/gifs?wait=0")
            .header("authorization", format!("Bearer {up}"))
            .header("content-type", "image/gif")
            .body(Body::from(bytes))
            .unwrap()
    };
    assert_eq!(h.send(post(1)).await.status(), StatusCode::CREATED);
    assert_eq!(h.send(post(2)).await.status(), StatusCode::TOO_MANY_REQUESTS);
    // Reads are a different bucket.
    assert_eq!(h.send(h.req("GET", "/api/v1/me", Some(&up), None)).await.status(), StatusCode::OK);

    // A per-key override replaces the default.
    let (_, k) = h
        .call("POST", "/api/v1/keys", Some(json!({"name": "slow", "preset": "read_only", "rate_limit": 2})))
        .await;
    let slow = k["key"].as_str().unwrap();
    let codes: Vec<_> = {
        let mut v = vec![];
        for _ in 0..3 {
            v.push(h.send(h.req("GET", "/api/v1/me", Some(slow), None)).await.status());
        }
        v
    };
    assert_eq!(codes, vec![StatusCode::OK, StatusCode::OK, StatusCode::TOO_MANY_REQUESTS]);
    let (_, me) = status(&h, "GET", "/api/v1/me", &h.token, None).await;
    assert_eq!(me["limits"]["read_per_min"], 120);
}

fn login_req(user: &str, pass: &str) -> Request<Body> {
    Request::post("/api/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(json!({"username": user, "password": pass}).to_string()))
        .unwrap()
}

#[tokio::test]
async fn login_sets_a_hardened_cookie_and_sessions_work() {
    let h = Harness::new().await;
    let resp = h.send(login_req("OWNER", PASSWORD)).await; // usernames are case-insensitive
    assert_eq!(resp.status(), StatusCode::OK);
    let set = resp.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(set.starts_with("unigif_session="));
    for flag in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=2592000"] {
        assert!(set.contains(flag), "{set}");
    }
    assert!(!set.contains("Secure"), "http dev setup must still work");
    let cookie = set.split(';').next().unwrap().to_string();
    // Only a hash of the cookie is stored.
    let value = cookie.split_once('=').unwrap().1;
    let stored: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT token_hash FROM sessions").fetch_all(&h.app.db).await.unwrap();
    assert_eq!(stored, vec![unigif::auth::hash_token(value).to_vec()]);

    let get = |uri: &str| Request::get(uri).header("cookie", &cookie).body(Body::empty()).unwrap();
    let (s, me) = json(h.send(get("/api/v1/me")).await).await;
    assert_eq!((s, me["kind"].as_str(), me["name"].as_str()), (StatusCode::OK, Some("user"), Some("owner")));
    assert_eq!(me["scopes"].as_array().unwrap().len(), 5, "the owner has every scope");
    assert_eq!(h.send(get("/api/v1/gifs")).await.status(), StatusCode::OK);
    assert_eq!(h.send(get("/api/v1/keys")).await.status(), StatusCode::OK);

    // A stale cookie is anonymous, not an error by itself.
    let stale =
        Request::get("/api/v1/me").header("cookie", "unigif_session=nope").body(Body::empty()).unwrap();
    assert_eq!(h.send(stale).await.status(), StatusCode::UNAUTHORIZED);
    let relogin = Request::post("/api/v1/auth/login")
        .header("cookie", "unigif_session=nope")
        .header("content-type", "application/json")
        .body(Body::from(json!({"username": "owner", "password": PASSWORD}).to_string()))
        .unwrap();
    assert_eq!(h.send(relogin).await.status(), StatusCode::OK);

    // Logout ends the session server-side and clears the cookie.
    let out = Request::post("/api/v1/auth/logout")
        .header("cookie", &cookie)
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();
    let resp = h.send(out).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert!(resp.headers()["set-cookie"].to_str().unwrap().contains("Max-Age=0"));
    assert_eq!(h.send(get("/api/v1/me")).await.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn secure_flag_follows_an_https_public_url() {
    let h = Harness::with(|c| c.public_url = "https://gif.example.com".into()).await;
    let resp = h.send(login_req("owner", PASSWORD)).await;
    assert!(resp.headers()["set-cookie"].to_str().unwrap().contains("; Secure"));
}

#[tokio::test]
async fn bad_logins_are_uniform_and_throttled() {
    let h = Harness::new().await;
    // Unknown user and wrong password are indistinguishable.
    let (s1, v1) = json(h.send(login_req("nobody", "whatever12")).await).await;
    let (s2, v2) = json(h.send(login_req("owner", "whatever12")).await).await;
    assert_eq!((s1, s2), (StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED));
    assert_eq!(v1, v2);
    assert_eq!(v1["code"], "invalid_credentials");
    // Malformed bodies are 400s.
    let bad = Request::post("/api/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(h.send(bad).await.status(), StatusCode::BAD_REQUEST);

    // Five failures in a minute lock the address out, even for the right password.
    // Two failures so far; the fifth one is still answered 401 but earns the lock-out.
    for _ in 0..3 {
        assert_eq!(h.send(login_req("owner", "nope nope")).await.status(), StatusCode::UNAUTHORIZED);
    }
    let resp = h.send(login_req("owner", PASSWORD)).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers()["retry-after"].to_str().unwrap().parse::<u64>().unwrap() > 0);
}

#[tokio::test]
async fn cookie_writes_need_json_or_the_header_and_the_right_origin() {
    let h = Harness::new().await;
    let cookie = h.login().await;
    let g = h.add_gif(1, "").await;
    let id = g["id"].as_str().unwrap();
    let bytes = gif(h.dir.path(), "c.gif", "96x64", 5);

    let post = |ct: Option<&str>, extra: Option<(&str, &str)>, body: Vec<u8>| {
        let mut b = Request::post("/api/v1/gifs").header("cookie", &cookie);
        if let Some(ct) = ct {
            b = b.header("content-type", ct);
        }
        if let Some((k, v)) = extra {
            b = b.header(k, v);
        }
        b.body(Body::from(body)).unwrap()
    };
    // A cross-site <form> can only send these simple content types, without the header.
    for ct in ["text/plain", "application/x-www-form-urlencoded", "multipart/form-data; boundary=x"] {
        let resp = h.send(post(Some(ct), None, bytes.clone())).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{ct}");
    }
    // The custom header opts a browser upload in.
    let resp = h.send(post(Some("image/gif"), Some(("x-unigif-request", "1")), bytes.clone())).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    // A foreign Origin is refused even then.
    let resp = h
        .send(post(
            Some("image/gif"),
            Some(("origin", "https://evil.example")),
            gif(h.dir.path(), "c2.gif", "96x64", 6),
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    // JSON writes with a same-origin Origin are fine.
    let patch = Request::patch(format!("/api/v1/gifs/{id}"))
        .header("cookie", &cookie)
        .header("content-type", "application/json")
        .header("host", "localhost:8787")
        .header("origin", "http://localhost:8787")
        .body(Body::from(json!({"tags": ["ok"]}).to_string()))
        .unwrap();
    assert_eq!(h.send(patch).await.status(), StatusCode::OK);
    let cross = Request::patch(format!("/api/v1/gifs/{id}"))
        .header("cookie", &cookie)
        .header("content-type", "application/json")
        .header("host", "localhost:8787")
        .header("origin", "http://evil.example")
        .body(Body::from(json!({"tags": ["pwned"]}).to_string()))
        .unwrap();
    assert_eq!(h.send(cross).await.status(), StatusCode::FORBIDDEN);
    // A cross-origin login attempt is refused as well.
    let login = Request::post("/api/v1/auth/login")
        .header("content-type", "application/json")
        .header("host", "localhost:8787")
        .header("origin", "http://evil.example")
        .body(Body::from(json!({"username": "owner", "password": PASSWORD}).to_string()))
        .unwrap();
    assert_eq!(h.send(login).await.status(), StatusCode::FORBIDDEN);
    // Bearer-key requests are not subject to the cookie rules.
    let resp = h.send(h.post("/api/v1/gifs", "text/plain", gif(h.dir.path(), "c3.gif", "96x64", 8))).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn session_management_and_password_change() {
    let h = Harness::new().await;
    let c1 = h.login().await;
    let c2 = h.login().await;
    let json_req = |method: &str, uri: &str, cookie: &str, body: Option<serde_json::Value>| {
        let b = Request::builder()
            .method(method)
            .uri(uri)
            .header("cookie", cookie)
            .header("content-type", "application/json");
        b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty)).unwrap()
    };

    let (_, list) = json(h.send(json_req("GET", "/api/v1/auth/sessions", &c1, None)).await).await;
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items.iter().filter(|s| s["current"] == true).count(), 1);
    let other = items.iter().find(|s| s["current"] == false).unwrap()["id"].as_str().unwrap().to_string();

    // Keys cannot manage sessions.
    let (s, _) = status(&h, "GET", "/api/v1/auth/sessions", &h.token, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Revoke the other session.
    let resp = h.send(json_req("DELETE", &format!("/api/v1/auth/sessions/{other}"), &c1, None)).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let resp = h.send(json_req("DELETE", &format!("/api/v1/auth/sessions/{other}"), &c1, None)).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let gone = h.send(json_req("GET", "/api/v1/me", &c2, None)).await.status();
    let still = h.send(json_req("GET", "/api/v1/me", &c1, None)).await.status();
    assert!(
        (gone, still) == (StatusCode::UNAUTHORIZED, StatusCode::OK)
            || (gone, still) == (StatusCode::OK, StatusCode::UNAUTHORIZED),
        "exactly the revoked one must be gone"
    );
    let (keep, drop_) =
        if still == StatusCode::OK { (c1.clone(), c2.clone()) } else { (c2.clone(), c1.clone()) };
    let _ = drop_;

    // Password change: wrong current password, weak new password, then success.
    let body = |cur: &str, new: &str| Some(json!({"current": cur, "new": new}));
    assert_eq!(
        h.send(json_req("POST", "/api/v1/auth/password", &keep, body("wrong password", "another long one")))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.send(json_req("POST", "/api/v1/auth/password", &keep, body(PASSWORD, "short"))).await.status(),
        StatusCode::BAD_REQUEST
    );
    let other_login = h.login().await; // a third session that the change must end
    assert_eq!(
        h.send(json_req("POST", "/api/v1/auth/password", &keep, body(PASSWORD, "a brand new password")))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.send(json_req("GET", "/api/v1/me", &keep, None)).await.status(),
        StatusCode::OK,
        "this session survives"
    );
    assert_eq!(
        h.send(json_req("GET", "/api/v1/me", &other_login, None)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(h.send(login_req("owner", PASSWORD)).await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(h.send(login_req("owner", "a brand new password")).await.status(), StatusCode::OK);

    // Preferences.
    let resp = h
        .send(json_req("PATCH", "/api/v1/me", &keep, Some(json!({"prefs": {"proceed_to_next": true}}))))
        .await;
    let (s, me) = json(resp).await;
    assert_eq!((s, me["prefs"]["proceed_to_next"].clone()), (StatusCode::OK, json!(true)));
    let (_, me) = json(h.send(json_req("GET", "/api/v1/me", &keep, None)).await).await;
    assert_eq!(me["prefs"]["proceed_to_next"], true);
}

#[tokio::test]
async fn cors_is_for_keys_never_for_cookies() {
    let h = Harness::new().await;
    let pre = Request::builder()
        .method("OPTIONS")
        .uri("/api/v1/gifs")
        .header("origin", "https://social.example")
        .header("access-control-request-method", "GET")
        .header("access-control-request-headers", "authorization")
        .body(Body::empty())
        .unwrap();
    let resp = h.send(pre).await;
    assert!(resp.status().is_success());
    assert_eq!(resp.headers()["access-control-allow-origin"], "*");
    assert!(resp.headers().get("access-control-allow-credentials").is_none());
    let allow_headers = resp.headers()["access-control-allow-headers"].to_str().unwrap().to_lowercase();
    assert!(allow_headers.contains("authorization") && allow_headers.contains("x-api-key"));

    // A configured allow-list is honoured.
    let h = Harness::with(|c| c.cors.origins = vec!["https://social.example".into()]).await;
    let ask = |origin: &str| {
        Request::builder()
            .method("OPTIONS")
            .uri("/api/v1/gifs")
            .header("origin", origin)
            .header("access-control-request-method", "GET")
            .body(Body::empty())
            .unwrap()
    };
    let ok = h.send(ask("https://social.example")).await;
    assert_eq!(ok.headers()["access-control-allow-origin"], "https://social.example");
    let no = h.send(ask("https://evil.example")).await;
    assert!(no.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn initial_owner_is_created_from_a_password_file_once() {
    let dir = tempfile::tempdir().unwrap();
    let pw = dir.path().join("pw");
    std::fs::write(&pw, "a long enough secret\n").unwrap();
    let mut cfg = unigif::config::Config::default();
    cfg.data_dir = dir.path().join("data");
    cfg.auth.initial_owner.username = Some("boss".into());
    cfg.auth.initial_owner.password_file = Some(pw.clone());
    let db = unigif::db::open(&cfg.db_path()).await.unwrap();
    unigif::auth::accounts::ensure_initial_owner(&cfg, &db).await.unwrap();
    let user = unigif::db::users::find_by_username(&db, "boss").await.unwrap().unwrap();
    assert!(unigif::auth::password::verify("a long enough secret", &user.password_hash));
    // Second start with a different file does nothing (the table is not empty).
    std::fs::write(&pw, "changed afterwards\n").unwrap();
    unigif::auth::accounts::ensure_initial_owner(&cfg, &db).await.unwrap();
    assert_eq!(unigif::db::users::count(&db).await.unwrap(), 1);
    let user = unigif::db::users::find_by_username(&db, "boss").await.unwrap().unwrap();
    assert!(unigif::auth::password::verify("a long enough secret", &user.password_hash));
}
