//! Shared application state.

use std::sync::Arc;

use tokio::sync::Notify;

use crate::config::Config;
use crate::db::Db;
use crate::media::ffmpeg::Ffmpeg;

pub struct AppState {
    pub cfg: Config,
    pub db: Db,
    pub ff: Ffmpeg,
    /// Wakes a worker when a job is queued.
    pub jobs: Notify,
}

pub type App = Arc<AppState>;

impl AppState {
    pub fn new(cfg: Config, db: Db) -> App {
        let ff = Ffmpeg::new(&cfg.media);
        Arc::new(Self { cfg, db, ff, jobs: Notify::new() })
    }
}
