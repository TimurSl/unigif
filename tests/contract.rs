//! API contract tests: `api/openapi.yaml` is the source of truth, and the
//! server must agree with it, in both directions.
#![allow(clippy::field_reassign_with_default)]
mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use serde_json::{json, Value};

/// Validate `instance` against `#/components/schemas/{name}` of the spec.
fn validate(name: &str, instance: &Value) {
    let spec = unigif::api::spec();
    let root = json!({ "$ref": format!("#/components/schemas/{name}"), "components": spec["components"] });
    let validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .should_validate_formats(true)
        .build(&root)
        .unwrap_or_else(|e| panic!("schema {name} does not compile: {e}"));
    let errors: Vec<String> =
        validator.iter_errors(instance).map(|e| format!("{} at {}", e, e.instance_path)).collect();
    assert!(errors.is_empty(), "{name} does not match the spec:\n{}\nin {instance:#}", errors.join("\n"));
}

#[test]
fn spec_is_well_formed() {
    let spec = unigif::api::spec();
    assert!(spec["openapi"].as_str().unwrap().starts_with("3.1"));
    let schemas = spec["components"]["schemas"].as_object().unwrap();
    for name in schemas.keys() {
        // Compiling resolves every $ref the schema reaches.
        let root =
            json!({ "$ref": format!("#/components/schemas/{name}"), "components": spec["components"] });
        jsonschema::validator_for(&root).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    let scopes = ["read", "upload", "tag", "delete", "admin", "session", "any", "none"];
    for (path, item) in spec["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            if method == "parameters" {
                continue;
            }
            let at = format!("{} {path}", method.to_uppercase());
            assert!(op["summary"].is_string(), "{at}: summary");
            assert!(scopes.contains(&op["x-scope"].as_str().unwrap_or("?")), "{at}: x-scope");
            assert!(op["responses"].as_object().is_some_and(|r| !r.is_empty()), "{at}: responses");
        }
    }
}

/// Every documented operation exists, and answers with a documented status.
#[tokio::test]
async fn every_documented_operation_is_served() {
    let h = Harness::new().await;
    let spec = unigif::api::spec();
    let mut covered = 0;
    for (path, item) in spec["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            if method == "parameters" {
                continue;
            }
            let uri = format!("/api/v1{}", path.replace("{id}", "AAAAAAAAAAAA").replace("{name}", "nothere"));
            let body = (method != "get" && method != "delete").then(|| json!({}));
            let resp = h.send(h.req(&method.to_uppercase(), &uri, Some(&h.token), body)).await;
            let status = resp.status();
            assert_ne!(
                status,
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {path} is documented but not routed"
            );
            let is_stream = resp
                .headers()
                .get("content-type")
                .is_some_and(|c| c.to_str().unwrap_or("").starts_with("text/event-stream"));
            if is_stream {
                continue;
            }
            let (_, v) = json(resp).await;
            assert!(
                !(status == StatusCode::NOT_FOUND && v["detail"] == "no such endpoint"),
                "{method} {path} is documented but not routed"
            );
            let documented = op["responses"].as_object().unwrap();
            assert!(
                documented.contains_key(&status.as_u16().to_string()),
                "{method} {path} answered {status}, which the spec does not list ({:?}): {v}",
                documented.keys().collect::<Vec<_>>()
            );
            covered += 1;
        }
    }
    assert!(covered >= 25, "only {covered} operations exercised");
}

#[tokio::test]
async fn unknown_endpoints_are_problem_json_404s() {
    let h = Harness::new().await;
    let resp = h.send(h.get("/api/v1/definitely/not/here")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(resp.headers()["content-type"], "application/problem+json");
    let (_, v) = json(resp).await;
    validate("Problem", &v);
}

/// Real responses validate against the documented schemas.
#[tokio::test]
async fn responses_match_their_schemas() {
    let h = Harness::new().await;

    // Gifs: created, duplicate, processing, by id, list.
    let a = h.add_gif(1, "cat,funny").await;
    validate("Gif", &a);
    let id = a["id"].as_str().unwrap();
    let (s, dup) = h.add("?tags=more", gif(h.dir.path(), "g1.gif", "96x64", 1)).await;
    assert_eq!((s, dup["duplicate"].clone()), (StatusCode::OK, json!(true)));
    validate("Gif", &dup);
    let (s, processing) = h.add("?wait=0", gif(h.dir.path(), "p.gif", "96x64", 4)).await;
    assert_eq!(s, StatusCode::CREATED);
    validate("Gif", &processing);
    wait_ready(&h, processing["id"].as_str().unwrap()).await;
    for uri in [
        "/api/v1/gifs".to_string(),
        "/api/v1/gifs?limit=1".to_string(),
        "/api/v1/gifs?q=cat&sort=used".to_string(),
        "/api/v1/gifs?trash=true".to_string(),
    ] {
        let (s, v) = h.call("GET", &uri, None).await;
        assert_eq!(s, StatusCode::OK, "{uri}");
        validate("GifPage", &v);
    }
    let (_, g) = h.call("GET", &format!("/api/v1/gifs/{id}"), None).await;
    validate("Gif", &g);
    let (_, g) = h.call("PATCH", &format!("/api/v1/gifs/{id}"), Some(json!({"description": "d"}))).await;
    validate("Gif", &g);
    let (_, g) = h.call("POST", &format!("/api/v1/gifs/{id}/tags"), Some(json!({"add": ["x"]}))).await;
    validate("Gif", &g);
    let (_, next) = h.call("GET", "/api/v1/gifs/unsorted/next", None).await;
    validate("UnsortedNext", &next);
    let (_, batch) =
        h.call("POST", "/api/v1/gifs/batch", Some(json!({"ids": [id, "AAAAAAAAAAAA"], "add": ["y"]}))).await;
    validate("BatchResult", &batch);
    h.call("DELETE", &format!("/api/v1/gifs/{id}"), None).await;
    let (_, restored) = h.call("POST", &format!("/api/v1/gifs/{id}/restore"), None).await;
    validate("Gif", &restored);
    let (s, emptied) = h.call("POST", "/api/v1/gifs/trash/empty", None).await;
    assert_eq!(s, StatusCode::OK);
    validate("TrashEmptied", &emptied);
    let (s, cleared) = h.call("DELETE", "/api/v1/cache", None).await;
    assert_eq!(s, StatusCode::OK);
    validate("CacheCleared", &cleared);

    // Tags.
    let (_, t) = h.call("GET", "/api/v1/tags?sort=recent", None).await;
    validate("TagPage", &t);
    let (_, t) = h.call("GET", "/api/v1/tags?limit=1", None).await;
    validate("TagPage", &t);
    assert!(t["next_cursor"].is_string());
    let (_, sg) = h.call("GET", "/api/v1/tags/suggest?selected=cat", None).await;
    validate("Suggestions", &sg);
    let (_, tag) = h.call("PATCH", "/api/v1/tags/x", Some(json!({"name": "xx"}))).await;
    validate("Tag", &tag);
    let (_, tag) = h.call("POST", "/api/v1/tags/xx/merge", Some(json!({"into": "y"}))).await;
    validate("Tag", &tag);

    // Keys, stats, me.
    let (s, k) = h.call("POST", "/api/v1/keys", Some(json!({"name": "k", "preset": "picker", "deny_tags": ["z"], "rate_limit": 10, "expires_in_days": 5}))).await;
    assert_eq!(s, StatusCode::CREATED);
    validate("KeyCreated", &k);
    let (_, kl) = h.call("GET", "/api/v1/keys", None).await;
    validate("KeyList", &kl);
    let (_, k2) = h
        .call(
            "PATCH",
            &format!("/api/v1/keys/{}", k["id"].as_str().unwrap()),
            Some(json!({"name": "renamed"})),
        )
        .await;
    validate("Key", &k2);
    let (_, st) = h.call("GET", "/api/v1/stats", None).await;
    validate("Stats", &st);
    let (_, me) = h.call("GET", "/api/v1/me", None).await;
    validate("Me", &me);
    let (_, me) = json(h.send(h.req("GET", "/api/v1/me", k["key"].as_str(), None)).await).await;
    validate("Me", &me);

    // Session endpoints.
    let cookie = h.login().await;
    let with_cookie = |m: &str, uri: &str, body: Option<Value>| {
        Request::builder()
            .method(m)
            .uri(uri)
            .header("cookie", &cookie)
            .header("content-type", "application/json")
            .body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty))
            .unwrap()
    };
    let (_, me) = json(h.send(with_cookie("GET", "/api/v1/me", None)).await).await;
    validate("Me", &me);
    let (_, sl) = json(h.send(with_cookie("GET", "/api/v1/auth/sessions", None)).await).await;
    validate("SessionList", &sl);
    let login = h
        .send(
            Request::post("/api/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(json!({"username": "owner", "password": PASSWORD}).to_string()))
                .unwrap(),
        )
        .await;
    validate("LoginResult", &json(login).await.1);

    // Errors, whichever way they arise.
    for (req, want) in [
        (h.req("GET", "/api/v1/gifs/AAAAAAAAAAAA", Some(&h.token), None), StatusCode::NOT_FOUND),
        (h.req("GET", "/api/v1/gifs", None, None), StatusCode::UNAUTHORIZED),
        (h.req("GET", "/api/v1/gifs?sort=x", Some(&h.token), None), StatusCode::BAD_REQUEST),
        (
            h.req("PATCH", "/api/v1/tags/nothere", Some(&h.token), Some(json!({"name": "q"}))),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let resp = h.send(req).await;
        assert_eq!(resp.status(), want);
        assert_eq!(resp.headers()["content-type"], "application/problem+json");
        validate("Problem", &json(resp).await.1);
    }
    let read = h.mint(unigif::auth::Scopes::from_slice(&[unigif::auth::Scope::Read]), &[]).await;
    let resp = h.send(h.req("DELETE", &format!("/api/v1/gifs/{id}"), Some(&read), None)).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    validate("Problem", &json(resp).await.1);
}

#[tokio::test]
async fn the_spec_and_a_reference_page_are_served() {
    let h = Harness::new().await;
    let resp = h.send(Request::get("/api/v1/openapi.json").body(Body::empty()).unwrap()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let (_, doc) = json(resp).await;
    assert_eq!(doc["info"]["title"], "Unigif API");
    assert!(doc["paths"]["/gifs"]["post"].is_object());

    let resp = h.send(Request::get("/api/docs").body(Body::empty()).unwrap()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers()["content-type"].to_str().unwrap().starts_with("text/html"));
    let page = String::from_utf8(bytes(resp).await).unwrap();
    assert!(
        page.contains("/api/v1/gifs/unsorted/next")
            && page.contains("Quick add")
            && !page.contains("<script")
    );

    // The configured public URL is advertised as the server.
    let h = Harness::with(|c| c.public_url = "https://gif.example.com/".into()).await;
    let (_, doc) =
        json(h.send(Request::get("/api/v1/openapi.json").body(Body::empty()).unwrap()).await).await;
    assert_eq!(doc["servers"][0]["url"], "https://gif.example.com");
}
