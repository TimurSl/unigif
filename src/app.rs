//! Shared application state.

use std::sync::Arc;

use tokio::sync::{broadcast, Notify};

use crate::auth::ratelimit::{LoginThrottle, RateLimiter};
use crate::config::Config;
use crate::db::Db;
use crate::media::ffmpeg::Ffmpeg;

/// A change worth telling `GET /events` subscribers about. Carries only the
/// id: subscribers re-fetch, and the server re-checks visibility per key.
#[derive(Debug, Clone)]
pub struct Event {
    pub kind: &'static str,
    pub gif_id: String,
}

pub struct AppState {
    pub cfg: Config,
    pub db: Db,
    pub ff: Ffmpeg,
    /// Wakes a worker when a job is queued.
    pub jobs: Notify,
    pub events: broadcast::Sender<Event>,
    pub limiter: RateLimiter,
    pub login_throttle: LoginThrottle,
}

pub type App = Arc<AppState>;

impl AppState {
    pub fn new(cfg: Config, db: Db) -> App {
        let ff = Ffmpeg::new(&cfg.media);
        let (events, _) = broadcast::channel(256);
        Arc::new(Self {
            cfg,
            db,
            ff,
            jobs: Notify::new(),
            events,
            limiter: RateLimiter::default(),
            login_throttle: LoginThrottle::default(),
        })
    }

    /// Announce a change. Nobody listening is fine.
    pub fn emit(&self, kind: &'static str, gif_id: &str) {
        let _ = self.events.send(Event { kind, gif_id: gif_id.to_string() });
    }
}
