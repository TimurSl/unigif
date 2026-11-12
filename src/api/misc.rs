//! Stats, the event stream, and the served OpenAPI document.

use std::convert::Infallible;
use std::sync::OnceLock;

use axum::extract::State;
use axum::http::header;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use futures_util::{Stream, StreamExt};
use serde_json::{json, Value};
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;

use crate::app::App;
use crate::auth::{Auth, Scope};
use crate::db::gifs;
use crate::error::ApiError;

pub async fn stats(State(app): State<App>, Auth(p): Auth) -> Result<Json<Value>, ApiError> {
    p.require(Scope::Read)?;
    let vis = p.visibility();
    let t = gifs::totals(&app.db, &vis).await?;
    Ok(Json(json!({
        "gifs": t.gifs,
        "unsorted": t.unsorted,
        "tags": super::tags::count_visible(&app.db, &vis).await?,
        "trash": gifs::trash_count(&app.db, &vis).await?,
        "bytes_stored": t.bytes_stored,
        "bytes_original": t.bytes_original,
        "bytes_saved": (t.bytes_original - t.bytes_stored).max(0),
    })))
}

/// Server-sent events: `gif.added`, `gif.ready`, `gif.tagged`, `gif.deleted`,
/// each carrying `{"id": …}`. A subscriber that falls behind gets a `resync`
/// event and should reload. Events about GIFs the key cannot see are dropped.
pub async fn events(
    State(app): State<App>,
    Auth(p): Auth,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    p.require(Scope::Read)?;
    let vis = p.visibility();
    let db = app.db.clone();
    let stream = BroadcastStream::new(app.events.subscribe()).filter_map(move |msg| {
        let vis = vis.clone();
        let db = db.clone();
        async move {
            match msg {
                Ok(ev) => {
                    if vis.hides(&db, &ev.gif_id).await.unwrap_or(true) {
                        return None;
                    }
                    Some(Ok(SseEvent::default().event(ev.kind).data(json!({ "id": ev.gif_id }).to_string())))
                }
                Err(BroadcastStreamRecvError::Lagged(_)) => {
                    Some(Ok(SseEvent::default().event("resync").data("{}")))
                }
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

const SPEC_YAML: &str = include_str!("../../api/openapi.yaml");

/// The parsed OpenAPI document (the YAML in `api/openapi.yaml`, embedded).
pub fn spec() -> &'static Value {
    static SPEC: OnceLock<Value> = OnceLock::new();
    SPEC.get_or_init(|| serde_yaml::from_str(SPEC_YAML).expect("api/openapi.yaml is valid YAML"))
}

pub async fn openapi_json(State(app): State<App>) -> Json<Value> {
    let mut doc = spec().clone();
    if !app.cfg.base_url().is_empty() {
        doc["servers"] = json!([{ "url": app.cfg.base_url() }]);
    }
    Json(doc)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A plain server-rendered reference. No scripts, so the strict CSP holds.
pub async fn docs() -> Response {
    let doc = spec();
    let mut out = String::from(
        "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'>\
<title>Unigif API</title><style>\
body{background:#0a0a0a;color:#fafafa;font:14px/1.5 system-ui;margin:0 auto;max-width:860px;padding:24px 16px}\
h1{font-size:28px}h2{font-size:20px;margin-top:32px}a{color:#fafafa}\
.op{border:1px solid #262626;border-radius:10px;padding:12px 16px;margin:12px 0;background:#111}\
.m{font:600 12px ui-monospace,monospace;background:#fafafa;color:#000;border-radius:6px;padding:2px 8px;margin-right:8px}\
code,.p{font-family:ui-monospace,monospace}.mut{color:#a1a1a1}td,th{text-align:left;padding:2px 12px 2px 0;vertical-align:top}\
</style><body>",
    );
    out.push_str(&format!("<h1>{}</h1>", esc(doc["info"]["title"].as_str().unwrap_or("API"))));
    out.push_str(&format!(
        "<p class=mut>Version {}. Machine-readable: <a href=\"/api/v1/openapi.json\">openapi.json</a>.</p>",
        esc(doc["info"]["version"].as_str().unwrap_or(""))
    ));
    if let Some(d) = doc["info"]["description"].as_str() {
        out.push_str(&format!("<p style=\"white-space:pre-line\">{}</p>", esc(d)));
    }
    if let Some(paths) = doc["paths"].as_object() {
        for (path, item) in paths {
            for method in ["get", "post", "patch", "put", "delete"] {
                let Some(op) = item.get(method) else { continue };
                out.push_str(&format!(
                    "<div class=op><span class=m>{}</span><span class=p>{}</span>",
                    method.to_uppercase(),
                    esc(&format!("/api/v1{path}"))
                ));
                if let Some(s) = op["summary"].as_str() {
                    out.push_str(&format!("<div>{}</div>", esc(s)));
                }
                let scope = op["x-scope"].as_str().unwrap_or("none");
                out.push_str(&format!("<div class=mut>Needs scope: <code>{}</code></div>", esc(scope)));
                if let Some(d) = op["description"].as_str() {
                    out.push_str(&format!("<div class=mut style=\"white-space:pre-line\">{}</div>", esc(d)));
                }
                if let Some(params) = op["parameters"].as_array() {
                    out.push_str("<table>");
                    for prm in params {
                        out.push_str(&format!(
                            "<tr><td><code>{}</code></td><td class=mut>{}</td><td>{}</td></tr>",
                            esc(prm["name"].as_str().unwrap_or("")),
                            esc(prm["in"].as_str().unwrap_or("")),
                            esc(prm["description"].as_str().unwrap_or(""))
                        ));
                    }
                    out.push_str("</table>");
                }
                out.push_str("</div>");
            }
        }
    }
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], Html(out).0).into_response()
}
