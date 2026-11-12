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

/// An 8 x 8 lossless animated WebP with two different frames (red, then red
/// with a blue corner), made once with `ffmpeg -c:v libwebp -lossless 1 -loop 0`.
/// Embedded so the check works without the encoder.
const TWO_FRAME_WEBP: [u8; 144] = [
    0x52, 0x49, 0x46, 0x46, 0x88, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x58, 0x0a,
    0x00, 0x00, 0x00, 0x12, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x07, 0x00, 0x00, 0x41, 0x4e, 0x49, 0x4d,
    0x06, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x41, 0x4e, 0x4d, 0x46, 0x28, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x07, 0x00, 0x00, 0xf4, 0x01, 0x00, 0x00,
    0x56, 0x50, 0x38, 0x4c, 0x0f, 0x00, 0x00, 0x00, 0x2f, 0x07, 0xc0, 0x01, 0x00, 0x07, 0x10, 0xfd, 0x8f,
    0xfe, 0x07, 0x22, 0xa2, 0xff, 0x01, 0x00, 0x41, 0x4e, 0x4d, 0x46, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x07, 0x00, 0x00, 0xf4, 0x01, 0x00, 0x00, 0x56, 0x50, 0x38,
    0x4c, 0x14, 0x00, 0x00, 0x00, 0x2f, 0x07, 0xc0, 0x01, 0x00, 0x0f, 0x10, 0xf3, 0xbf, 0xff, 0xf3, 0x1f,
    0x0e, 0x0e, 0xff, 0x1f, 0xd1, 0xff, 0x80, 0x07,
];

/// How many frames this ffmpeg decodes from an animated WebP. ffmpeg 6.x
/// cannot decode animated WebP at all (it errors or yields at most one frame);
/// 7.1 and later decode the animation.
async fn webp_frames(ff: &Ffmpeg, scratch: &std::path::Path) -> Result<u64> {
    let path = scratch.join(".unigif-doctor.webp");
    tokio::fs::write(&path, TWO_FRAME_WEBP).await?;
    let out = ff
        .run_probe(
            crate::args![
                "-count_frames",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=nb_read_frames",
                "-of",
                "csv=p=0",
                &path
            ],
            std::time::Duration::from_secs(10),
        )
        .await;
    let _ = tokio::fs::remove_file(&path).await;
    let text = String::from_utf8_lossy(&out?).trim().to_string();
    Ok(text.lines().next().and_then(|l| l.trim().parse().ok()).unwrap_or(0))
}

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

    // Not fatal: only animated WebP uploads are affected.
    let frames = webp_frames(&ff, &cfg.data_dir).await.unwrap_or(0);
    r.push(
        true,
        if frames >= 2 {
            "animated WebP decoding: ok".to_string()
        } else {
            format!(
                "animated WebP decoding: warning: this ffmpeg decodes {frames} of 2 frames, so animated WebP \
                 uploads will fail or keep one frame (ffmpeg 7.1 or later decodes them)"
            )
        },
    );

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
