//! Test harness: a real app (SQLite on disk, real ffmpeg) driven through the
//! real router with `tower::ServiceExt::oneshot`.
#![allow(dead_code, clippy::field_reassign_with_default)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use unigif::app::{App, AppState};
use unigif::auth::keys::{self, MintParams};
use unigif::auth::{accounts, Scopes};
use unigif::config::Config;
use unigif::ingest::worker;
use unigif::{api, db};

pub const PASSWORD: &str = "correct horse battery";

pub struct Harness {
    pub app: App,
    pub dir: tempfile::TempDir,
    /// A key with every scope.
    pub token: String,
    pub user_id: i64,
}

impl Harness {
    pub async fn new() -> Self {
        Self::with(|_| {}).await
    }

    pub async fn with(tweak: impl FnOnce(&mut Config)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.data_dir = dir.path().join("data");
        cfg.cache_dir = dir.path().join("cache");
        cfg.fetch.allow_private = true; // the URL tests talk to a local server
        cfg.media.preset = "ultrafast".into(); // keep the tests quick
        tweak(&mut cfg);
        for d in [cfg.media_dir(), cfg.tmp_dir(), cfg.work_dir(), cfg.cache_dir.clone()] {
            std::fs::create_dir_all(d).unwrap();
        }
        let pool = db::open(&cfg.db_path()).await.unwrap();
        let user_id = accounts::create_user(&pool, "owner", PASSWORD).await.unwrap();
        let app = AppState::new(cfg, pool);
        worker::recover(&app).await.unwrap();
        worker::spawn(&app, 2);
        let mut h = Self { app, dir, token: String::new(), user_id };
        h.token = h.mint(Scopes::all(), &[]).await;
        h
    }

    /// Mint a key and return its token.
    pub async fn mint(&self, scopes: Scopes, deny: &[&str]) -> String {
        keys::mint(
            &self.app.db,
            MintParams {
                name: "test".into(),
                scopes,
                deny_tags: deny.iter().map(|s| s.to_string()).collect(),
                rate_limit: None,
                expires_at: None,
                created_by: self.user_id,
            },
        )
        .await
        .unwrap()
        .1
    }

    /// Log in as the owner and return the `unigif_session=…` cookie pair.
    pub async fn login(&self) -> String {
        let resp = self
            .send(
                Request::post("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"username":"owner","password":"{PASSWORD}"}}"#)))
                    .unwrap(),
            )
            .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let set = resp.headers()["set-cookie"].to_str().unwrap().to_string();
        set.split(';').next().unwrap().to_string()
    }

    /// A JSON request with an optional bearer token.
    pub fn req(
        &self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Request<Body> {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        match body {
            Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
            None => b.body(Body::empty()).unwrap(),
        }
    }

    /// Send as the all-scopes key and return (status, JSON).
    pub async fn call(
        &self,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        json(self.send(self.req(method, uri, Some(&self.token), body)).await).await
    }

    pub async fn send(&self, req: Request<Body>) -> Response<Body> {
        api::router(self.app.clone()).oneshot(req).await.unwrap()
    }

    pub fn post(&self, uri: &str, content_type: &str, body: Vec<u8>) -> Request<Body> {
        Request::post(uri)
            .header("authorization", format!("Bearer {}", self.token))
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap()
    }

    pub fn get(&self, uri: &str) -> Request<Body> {
        Request::get(uri)
            .header("authorization", format!("Bearer {}", self.token))
            .body(Body::empty())
            .unwrap()
    }

    /// Quick-add raw bytes and return (status, JSON body).
    pub async fn add(&self, query: &str, bytes: Vec<u8>) -> (StatusCode, serde_json::Value) {
        let resp =
            self.send(self.post(&format!("/api/v1/gifs{query}"), "application/octet-stream", bytes)).await;
        json(resp).await
    }
}

pub async fn json(resp: Response<Body>) -> (StatusCode, serde_json::Value) {
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

pub async fn bytes(resp: Response<Body>) -> Vec<u8> {
    resp.into_body().collect().await.unwrap().to_bytes().to_vec()
}

/// Run ffmpeg to synthesise a fixture. Panics with ffmpeg's stderr on failure.
pub fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg").args(["-nostdin", "-v", "error", "-y"]).args(args).output().unwrap();
    assert!(out.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr));
}

/// A small animated GIF with a distinct look per `seed`.
pub fn gif(dir: &Path, name: &str, size: &str, seed: u32) -> Vec<u8> {
    let path = dir.join(name);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc2=size={size}:rate=10:duration=1.5,hue=h={}", seed * 40),
        path.to_str().unwrap(),
    ]);
    std::fs::read(path).unwrap()
}

/// A red square on a genuinely transparent background. Only a palette with a
/// reserved transparent entry yields a GIF that really has alpha.
pub fn transparent_gif_fixture(path: &std::path::Path) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=black@0:s=80x80:r=10:d=1,format=rgba",
        "-f",
        "lavfi",
        "-i",
        "color=c=red@1:s=40x40:r=10:d=1,format=rgba",
        "-filter_complex",
        "[0:v][1:v]overlay=20:20:format=auto,format=rgba,split[a][b];\
         [a]palettegen=reserve_transparent=1[p];[b][p]paletteuse",
        path.to_str().unwrap(),
    ]);
}

pub fn probe(path: &Path) -> serde_json::Value {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_streams", "-show_format"])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success(), "ffprobe failed on {path:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

pub fn video_stream(v: &serde_json::Value) -> &serde_json::Value {
    v["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap()
}

/// Poll until the GIF leaves `processing`.
pub async fn wait_ready(h: &Harness, id: &str) -> serde_json::Value {
    for _ in 0..300 {
        let (_, v) = json(h.send(h.get(&format!("/api/v1/gifs/{id}"))).await).await;
        if v["status"] != "processing" {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("gif {id} still processing after 30 s");
}

pub fn media_path(h: &Harness, id: &str, name: &str) -> PathBuf {
    unigif::media::gif_dir(&h.app.cfg.media_dir(), id).join(name)
}

impl Harness {
    /// Add a distinct tiny GIF (by `seed`) with hashtags and wait until it is ready.
    pub async fn add_gif(&self, seed: u32, tags: &str) -> serde_json::Value {
        let bytes = gif(self.dir.path(), &format!("g{seed}.gif"), "96x64", seed);
        let (s, v) = self.add(&format!("?wait=30&tags={tags}"), bytes).await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        assert_eq!(v["status"], "ready", "{v}");
        v
    }

    pub fn ids(v: &serde_json::Value) -> Vec<String> {
        v["items"].as_array().unwrap().iter().map(|g| g["id"].as_str().unwrap().to_string()).collect()
    }
}

/// Serve fixed files from a local port for the URL-fetch tests.
pub async fn serve_files(gif_bytes: Vec<u8>) -> String {
    use axum::response::Redirect;
    use axum::routing::get;
    let bytes = std::sync::Arc::new(gif_bytes);
    let app = axum::Router::new()
        .route(
            "/a.gif",
            get({
                let b = bytes.clone();
                move || async move { ([("content-type", "image/gif")], b.to_vec()) }
            }),
        )
        .route("/redir", get(|| async { Redirect::temporary("/a.gif") }))
        .route("/loop", get(|| async { Redirect::temporary("/loop") }))
        .route("/missing", get(|| async { (axum::http::StatusCode::NOT_FOUND, "nope") }))
        .route("/text", get(|| async { "this is not an animation" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}
