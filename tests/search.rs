//! Third-party search: KLIPY and GIPHY, against a local stand-in for both.
#![allow(clippy::field_reassign_with_default)]
mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use common::*;
use serde_json::{json, Value};

/// What the fake providers saw, so tests can check what the server sent.
#[derive(Default)]
struct Seen {
    giphy: Mutex<Vec<HashMap<String, String>>>,
    klipy: Mutex<Vec<(String, HashMap<String, String>)>>,
}

const GIPHY_KEY: &str = "giphy-secret-key";
const KLIPY_KEY: &str = "klipy-secret-key";

/// A fake of both APIs plus a file server; returns (base URL, what was seen).
async fn provider_stub(gif_bytes: Vec<u8>) -> (String, Arc<Seen>) {
    let seen = Arc::new(Seen::default());
    let gif = Arc::new(gif_bytes);
    let base = Arc::new(Mutex::new(String::new()));

    async fn giphy(
        State((seen, base)): State<(Arc<Seen>, Arc<Mutex<String>>)>,
        Query(q): Query<HashMap<String, String>>,
    ) -> (StatusCode, Json<Value>) {
        seen.giphy.lock().unwrap().push(q.clone());
        if q.get("api_key").map(String::as_str) != Some(GIPHY_KEY) {
            return (StatusCode::FORBIDDEN, Json(json!({"message": "invalid key"})));
        }
        let b = base.lock().unwrap().clone();
        let offset: u64 = q.get("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
        (
            StatusCode::OK,
            Json(json!({
                "data": [{
                    "id": "g1", "title": "Cat", "url": "https://giphy.com/gifs/g1",
                    "images": {
                        "original": {"url": format!("{b}/files/a.gif"), "width": "96", "height": "64", "size": "1000"},
                        "fixed_width": {"url": format!("{b}/files/a.gif"), "mp4": format!("{b}/files/a.mp4")}
                    }
                }],
                "pagination": {"total_count": 2, "count": 1, "offset": offset}
            })),
        )
    }

    async fn klipy(
        State((seen, base)): State<(Arc<Seen>, Arc<Mutex<String>>)>,
        Path((key, kind)): Path<(String, String)>,
        Query(q): Query<HashMap<String, String>>,
    ) -> (StatusCode, Json<Value>) {
        seen.klipy.lock().unwrap().push((format!("{key}/{kind}"), q));
        if key != KLIPY_KEY {
            return (StatusCode::UNAUTHORIZED, Json(json!({"result": false})));
        }
        let b = base.lock().unwrap().clone();
        let f = |u: &str| json!({"url": format!("{b}/files/{u}"), "width": 96, "height": 64, "size": 1000});
        (
            StatusCode::OK,
            Json(json!({"result": true, "data": {
                "data": [{"id": 5, "title": "Dog", "file": {"hd": {"gif": f("a.gif"), "mp4": f("a.mp4")}, "sm": {"mp4": f("a.mp4")}}}],
                "current_page": 1, "per_page": 24, "has_next": false
            }})),
        )
    }

    let app = Router::new()
        .route("/giphy/gifs/search", get(giphy))
        .route("/giphy/gifs/trending", get(giphy))
        .route("/klipy/{key}/gifs/{kind}", get(klipy))
        .route(
            "/files/a.gif",
            get({
                let g = gif.clone();
                move || async move { ([("content-type", "image/gif")], g.to_vec()) }
            }),
        )
        .with_state((seen.clone(), base.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    *base.lock().unwrap() = url.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, seen)
}

async fn harness() -> (Harness, Arc<Seen>, String) {
    let tmp = tempfile::tempdir().unwrap();
    let (stub, seen) = provider_stub(gif(tmp.path(), "stub.gif", "96x64", 3)).await;
    let (g, k) = (format!("{stub}/giphy"), format!("{stub}/klipy"));
    let h = Harness::with(|c| {
        c.search.giphy_base_url = g;
        c.search.klipy_base_url = k;
    })
    .await;
    (h, seen, stub)
}

/// A request carrying the owner's session cookie.
async fn as_owner(h: &Harness, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let cookie = h.login().await;
    let mut b = axum::http::Request::builder().method(method).uri(uri).header("cookie", cookie);
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            axum::body::Body::from(v.to_string())
        }
        None => axum::body::Body::empty(),
    };
    json(h.send(b.body(body).unwrap()).await).await
}

#[tokio::test]
async fn providers_start_unconfigured_and_keys_are_set_by_the_owner_only() {
    let (h, _, _) = harness().await;
    let (s, v) = h.call("GET", "/api/v1/search/providers", None).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = v["items"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["klipy", "giphy"]);
    assert!(v["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["configured"] == false && p["key_source"].is_null()));

    // A key may not change settings, even with every scope.
    let (s, _) = h.call("PUT", "/api/v1/search/providers/giphy/key", Some(json!({"key": GIPHY_KEY}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (s, v) = as_owner(
        &h,
        "PUT",
        "/api/v1/search/providers/giphy/key",
        Some(json!({"key": format!("  {GIPHY_KEY} ")})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["configured"].clone(), v["key_source"].clone()), (json!(true), json!("settings")));
    assert!(!v.to_string().contains(GIPHY_KEY), "the key must never be sent back");
    let (_, v) = h.call("GET", "/api/v1/search/providers", None).await;
    assert!(!v.to_string().contains(GIPHY_KEY));

    // Rejects a key that would break the Klipy URL, an unknown provider, and clears on null.
    let (s, _) =
        as_owner(&h, "PUT", "/api/v1/search/providers/klipy/key", Some(json!({"key": "a/b?c"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = as_owner(&h, "PUT", "/api/v1/search/providers/tenor/key", Some(json!({"key": "x"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, v) = as_owner(&h, "PUT", "/api/v1/search/providers/giphy/key", Some(json!({"key": null}))).await;
    assert_eq!(v["configured"], false);
}

#[tokio::test]
async fn searching_needs_a_provider_a_key_and_the_upload_scope() {
    let (h, _, _) = harness().await;
    let (s, v) = h.call("GET", "/api/v1/search", None).await;
    assert_eq!((s, v["code"].clone()), (StatusCode::BAD_REQUEST, json!("bad_request")));
    let (s, _) = h.call("GET", "/api/v1/search?provider=tenor&q=cat", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = h.call("GET", "/api/v1/search?provider=giphy&q=cat", None).await;
    assert_eq!((s, v["code"].clone()), (StatusCode::CONFLICT, json!("not_configured")));

    let read = h.mint(unigif::auth::Scopes::from_slice(&[unigif::auth::Scope::Read]), &[]).await;
    let resp = h.send(h.req("GET", "/api/v1/search?provider=giphy&q=cat", Some(&read), None)).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn giphy_search_paginates_and_never_leaks_the_key() {
    let (h, seen, _) = harness().await;
    as_owner(&h, "PUT", "/api/v1/search/providers/giphy/key", Some(json!({"key": GIPHY_KEY}))).await;

    let (s, v) = h.call("GET", "/api/v1/search?provider=giphy&q=funny%20cat&limit=1", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["next_cursor"], "1");
    let item = &v["items"][0];
    assert_eq!(
        (item["id"].clone(), item["title"].clone(), item["width"].clone()),
        (json!("g1"), json!("Cat"), json!(96))
    );
    assert_eq!(
        (item["download"]["format"].clone(), item["preview"]["kind"].clone()),
        (json!("gif"), json!("video"))
    );
    let sent = seen.giphy.lock().unwrap()[0].clone();
    assert_eq!(
        (sent["q"].as_str(), sent["limit"].as_str(), sent["offset"].as_str()),
        ("funny cat", "1", "0")
    );
    assert_eq!(sent["rating"], "pg-13");

    // The cursor is the next offset; an empty query asks for what is trending.
    h.call("GET", "/api/v1/search?provider=giphy&q=funny%20cat&cursor=1", None).await;
    assert_eq!(seen.giphy.lock().unwrap()[1]["offset"], "1");
    let (s, _) = h.call("GET", "/api/v1/search?provider=giphy", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!seen.giphy.lock().unwrap()[2].contains_key("q"));

    // A wrong key is a provider error that does not echo the key.
    as_owner(&h, "PUT", "/api/v1/search/providers/giphy/key", Some(json!({"key": "wrong-key"}))).await;
    let (s, v) = h.call("GET", "/api/v1/search?provider=giphy&q=cat", None).await;
    assert_eq!((s, v["code"].clone()), (StatusCode::BAD_GATEWAY, json!("provider_error")));
    assert!(v["detail"].as_str().unwrap().contains("rejected the API key"));
    assert!(!v.to_string().contains("wrong-key"));
}

#[tokio::test]
async fn klipy_search_puts_the_key_in_the_path_and_the_config_key_is_a_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let (stub, seen) = provider_stub(gif(tmp.path(), "s.gif", "96x64", 1)).await;
    let h = Harness::with(|c| {
        c.search.klipy_base_url = format!("{stub}/klipy");
        c.search.klipy_api_key = Some(KLIPY_KEY.into());
    })
    .await;

    let (_, v) = h.call("GET", "/api/v1/search/providers", None).await;
    assert_eq!(
        (v["items"][0]["configured"].clone(), v["items"][0]["key_source"].clone()),
        (json!(true), json!("config"))
    );

    let (s, v) = h.call("GET", "/api/v1/search?provider=klipy&q=dog", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["next_cursor"], Value::Null);
    let item = &v["items"][0];
    assert_eq!((item["id"].clone(), item["preview"]["kind"].clone()), (json!("5"), json!("video")));
    let (path, q) = seen.klipy.lock().unwrap()[0].clone();
    assert_eq!(path, format!("{KLIPY_KEY}/search"));
    assert_eq!((q["q"].as_str(), q["page"].as_str(), q["per_page"].as_str()), ("dog", "1", "24"));

    h.call("GET", "/api/v1/search?provider=klipy", None).await;
    assert_eq!(seen.klipy.lock().unwrap()[1].0, format!("{KLIPY_KEY}/trending"));

    // A key saved in Settings wins over the configuration file.
    as_owner(&h, "PUT", "/api/v1/search/providers/klipy/key", Some(json!({"key": "wrong"}))).await;
    let (s, _) = h.call("GET", "/api/v1/search?provider=klipy&q=dog", None).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn a_result_is_added_through_quick_add_and_deduplicated() {
    let (h, _, _) = harness().await;
    as_owner(&h, "PUT", "/api/v1/search/providers/giphy/key", Some(json!({"key": GIPHY_KEY}))).await;
    let (_, v) = h.call("GET", "/api/v1/search?provider=giphy&q=cat", None).await;
    let item = &v["items"][0];

    let body = json!({
        "url": item["download"]["url"], "source": "giphy", "source_id": item["id"],
        "description": item["title"], "wait": 30,
    });
    let (s, g) = h.call("POST", "/api/v1/gifs", Some(body.clone())).await;
    assert_eq!(s, StatusCode::CREATED, "{g}");
    assert_eq!(
        (g["source"]["type"].clone(), g["source"]["id"].clone(), g["description"].clone()),
        (json!("giphy"), json!("g1"), json!("Cat"))
    );
    assert_eq!(g["tags"], json!([]), "it lands in Unsorted");

    let (s, again) = h.call("POST", "/api/v1/gifs", Some(body)).await;
    assert_eq!(
        (s, again["duplicate"].clone(), again["id"].clone()),
        (StatusCode::OK, json!(true), g["id"].clone())
    );
}

/// Responses validate against the documented schemas.
#[tokio::test]
async fn responses_match_the_spec() {
    fn validate(name: &str, instance: &Value) {
        let spec = unigif::api::spec();
        let root =
            json!({ "$ref": format!("#/components/schemas/{name}"), "components": spec["components"] });
        let v = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .should_validate_formats(true)
            .build(&root)
            .unwrap();
        let errors: Vec<String> =
            v.iter_errors(instance).map(|e| format!("{e} at {}", e.instance_path)).collect();
        assert!(errors.is_empty(), "{name}: {}\nin {instance:#}", errors.join("\n"));
    }
    let (h, _, _) = harness().await;
    let (_, v) = h.call("GET", "/api/v1/search/providers", None).await;
    validate("SearchProviderList", &v);
    let (_, v) =
        as_owner(&h, "PUT", "/api/v1/search/providers/giphy/key", Some(json!({"key": GIPHY_KEY}))).await;
    validate("SearchProvider", &v);
    let (_, v) = h.call("GET", "/api/v1/search?provider=giphy&q=cat", None).await;
    validate("SearchPage", &v);
    as_owner(&h, "PUT", "/api/v1/search/providers/klipy/key", Some(json!({"key": KLIPY_KEY}))).await;
    let (_, v) = h.call("GET", "/api/v1/search?provider=klipy&q=dog", None).await;
    validate("SearchPage", &v);
    let (_, v) = h.call("GET", "/api/v1/search?provider=tenor", None).await;
    validate("Problem", &v);
}
