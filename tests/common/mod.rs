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
use unigif::config::Config;
use unigif::ingest::worker;
use unigif::{api, db};

pub const TOKEN: &str = "test-token";

pub struct Harness {
    pub app: App,
    pub dir: tempfile::TempDir,
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
        cfg.auth.bootstrap_token = Some(TOKEN.into());
        cfg.media.preset = "ultrafast".into(); // keep the tests quick
        tweak(&mut cfg);
        for d in [cfg.media_dir(), cfg.tmp_dir(), cfg.work_dir(), cfg.cache_dir.clone()] {
            std::fs::create_dir_all(d).unwrap();
        }
        let pool = db::open(&cfg.db_path()).await.unwrap();
        let app = AppState::new(cfg, pool);
        worker::recover(&app).await.unwrap();
        worker::spawn(&app, 2);
        Self { app, dir }
    }

    pub async fn send(&self, req: Request<Body>) -> Response<Body> {
        api::router(self.app.clone()).oneshot(req).await.unwrap()
    }

    pub fn post(&self, uri: &str, content_type: &str, body: Vec<u8>) -> Request<Body> {
        Request::post(uri)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap()
    }

    pub fn get(&self, uri: &str) -> Request<Body> {
        Request::get(uri).header("authorization", format!("Bearer {TOKEN}")).body(Body::empty()).unwrap()
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
