//! `unigif doctor`: verify the environment can actually run the pipeline.

use anyhow::Result;

use crate::config::Config;
use crate::media::ffmpeg::Ffmpeg;

pub struct Report {
    pub lines: Vec<(bool, String)>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.lines.iter().all(|(ok, _)| *ok)
    }
    fn push(&mut self, ok: bool, msg: impl Into<String>) {
        self.lines.push((ok, msg.into()));
    }
}

/// Encoders the pipeline needs, with what each is used for.
const REQUIRED_ENCODERS: [(&str, &str); 2] =
    [("libx264", "full.mp4 and preview.mp4"), ("libwebp", "poster.webp")];
/// Needed only for the on-demand variants; their absence is reported but not fatal.
const OPTIONAL_ENCODERS: [(&str, &str); 2] = [("gif", "on-demand GIF"), ("libvpx-vp9", "on-demand WebM")];

pub async fn run(cfg: &Config) -> Result<Report> {
    let mut r = Report { lines: vec![] };
    let ff = Ffmpeg::new(&cfg.media);

    match ff.version().await {
        Ok(v) => r.push(true, format!("ffmpeg: {v}")),
        Err(e) => {
            r.push(false, format!("ffmpeg not runnable ({}): {e}", cfg.media.ffmpeg));
            return Ok(r);
        }
    }
    match tokio::process::Command::new(ff.ffprobe_bin()).arg("-version").output().await {
        Ok(o) if o.status.success() => r.push(true, format!("ffprobe: {}", ff.ffprobe_bin())),
        _ => r.push(false, format!("ffprobe not runnable ({})", ff.ffprobe_bin())),
    }

    match ff.encoders().await {
        Ok(encs) => {
            for (name, why) in REQUIRED_ENCODERS {
                let has = encs.iter().any(|e| e == name);
                r.push(has, format!("encoder {name} ({why}){}", if has { "" } else { ": MISSING" }));
            }
            for (name, why) in OPTIONAL_ENCODERS {
                let has = encs.iter().any(|e| e == name);
                // Optional: always "ok", but say so when absent.
                r.push(
                    true,
                    format!(
                        "encoder {name} ({why}): {}",
                        if has { "present" } else { "missing: the matching on-demand format will fail" }
                    ),
                );
            }
        }
        Err(e) => r.push(false, format!("could not list encoders: {e}")),
    }

    for dir in [&cfg.data_dir, &cfg.cache_dir] {
        let probe = dir.join(".unigif-doctor");
        let ok = tokio::fs::create_dir_all(dir).await.is_ok()
            && tokio::fs::write(&probe, b"x").await.is_ok()
            && tokio::fs::remove_file(&probe).await.is_ok();
        r.push(ok, format!("{} is writable", dir.display()));
    }

    match crate::db::open(&cfg.db_path()).await {
        Ok(db) => {
            let res: Result<String, _> = sqlx::query_scalar("PRAGMA integrity_check").fetch_one(&db).await;
            match res {
                Ok(s) if s == "ok" => {
                    r.push(true, format!("database {} is healthy", cfg.db_path().display()))
                }
                Ok(s) => r.push(false, format!("database integrity_check: {s}")),
                Err(e) => r.push(false, format!("database integrity_check failed: {e}")),
            }
        }
        Err(e) => r.push(false, format!("database: {e:#}")),
    }

    if let Ok(db) = crate::db::open(&cfg.db_path()).await {
        match crate::db::users::count(&db).await {
            Ok(0) => r.push(
                true,
                "no user exists yet: run `unigif user add <name>` (or set auth.initialOwner) so you can log in",
            ),
            Ok(n) => r.push(true, format!("{n} user(s) can log in")),
            Err(e) => r.push(false, format!("reading users: {e}")),
        }
    }
    Ok(r)
}
