//! On-demand variants: `full.gif`, `preview.gif`, `full.webm`, `full.webp`.
//!
//! Pure ffmpeg work. Deciding which source to use, sharing concurrent
//! requests and the cache itself live in `crate::cache`.

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use super::ffmpeg::{input, Ffmpeg};
use super::probe::{self, ProbeInfo};
use super::Variant;
use crate::args;
use crate::config::MediaConfig;

/// What a conversion reads from.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    pub path: &'a Path,
    /// Forced ffmpeg demuxer (see `SourceKind::demuxer`).
    pub demuxer: Option<&'static str>,
    /// The kept original rather than `full.mp4`. Only then can transparency
    /// survive, so only then is the palette told to reserve it.
    pub is_original: bool,
}

/// Width to scale to: the configured edge, never upscaled.
pub fn target_width(src_w: u32, cap: u32) -> u32 {
    src_w.min(cap).max(1)
}

/// `fps=…,` when the source runs faster than `cap`; otherwise the source
/// timing is left alone (an unconditional `fps` filter would duplicate
/// frames of a slow GIF and make the file bigger for nothing).
pub fn fps_cap_filter(src_fps: f64, cap: u32) -> String {
    if src_fps > cap as f64 + 0.5 {
        format!("fps={cap},")
    } else {
        String::new()
    }
}

/// The filtergraph for a GIF; `alpha` reserves a transparent palette entry.
pub fn gif_filter(fps_part: &str, width: u32, alpha: bool) -> String {
    let scale = format!("{fps_part}scale={width}:-1:flags=lanczos");
    if alpha {
        format!(
            "{scale},format=rgba,split[a][b];\
             [a]palettegen=stats_mode=diff:reserve_transparent=1[p];\
             [b][p]paletteuse=dither=bayer:bayer_scale=5:alpha_threshold=128"
        )
    } else {
        format!(
            "{scale},split[a][b];[a]palettegen=stats_mode=diff[p];\
             [b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle"
        )
    }
}

/// Convert `src` into `variant`, writing `out`. `has_alpha` says whether the
/// GIF's original actually uses transparency.
pub async fn convert(
    ff: &Ffmpeg,
    cfg: &MediaConfig,
    variant: Variant,
    src: Source<'_>,
    has_alpha: bool,
    out: &Path,
) -> Result<()> {
    if variant.is_stored() {
        bail!("{} is not an on-demand variant", variant.file_name());
    }
    let timeout = Duration::from_secs(cfg.timeout_secs);
    let info: ProbeInfo =
        probe::probe(ff, src.path, src.demuxer).await.context("probing conversion source")?;
    let alpha = has_alpha && src.is_original;

    let mut a = args!["-an"];
    a.extend(input(src.path, src.demuxer));
    match variant {
        Variant::FullGif | Variant::PreviewGif => {
            let (cap_w, cap_fps) = if variant == Variant::FullGif {
                (cfg.gif_width, cfg.gif_fps)
            } else {
                (cfg.preview_gif_width, cfg.preview_gif_fps)
            };
            let vf = gif_filter(&fps_cap_filter(info.fps, cap_fps), target_width(info.width, cap_w), alpha);
            a.extend(args!["-filter_complex", vf, "-loop", "0", "-f", "gif", out]);
        }
        Variant::FullWebm => {
            a.extend(args![
                "-c:v",
                "libvpx-vp9",
                "-crf",
                cfg.webm_crf.to_string(),
                "-b:v",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-row-mt",
                "1",
                "-deadline",
                "good",
                "-cpu-used",
                "4",
                "-fps_mode",
                "vfr",
                "-f",
                "webm",
                out
            ]);
        }
        Variant::FullWebp => {
            let vf = format!(
                "{}scale={}:-1:flags=lanczos",
                fps_cap_filter(info.fps, cfg.max_fps),
                target_width(info.width, cfg.webp_width)
            );
            a.extend(args![
                "-vf",
                vf,
                "-c:v",
                "libwebp",
                "-lossless",
                "0",
                "-quality",
                cfg.webp_quality.to_string(),
                "-compression_level",
                "4",
                "-loop",
                "0",
                "-fps_mode",
                "vfr",
                "-f",
                "webp",
                out
            ]);
        }
        _ => unreachable!("stored variants were rejected above"),
    }
    ff.run(a, timeout).await.with_context(|| format!("converting to {}", variant.file_name()))?;

    let len = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    if len == 0 {
        bail!("{} came out empty", variant.file_name());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_is_capped_never_upscaled() {
        assert_eq!(target_width(1280, 480), 480);
        assert_eq!(target_width(320, 480), 320);
        assert_eq!(target_width(0, 480), 1);
    }

    #[test]
    fn fps_is_only_capped_downward() {
        assert_eq!(fps_cap_filter(60.0, 15), "fps=15,");
        assert_eq!(fps_cap_filter(15.0, 15), "");
        assert_eq!(fps_cap_filter(10.0, 15), "");
    }

    #[test]
    fn gif_filters_differ_for_alpha() {
        let plain = gif_filter("", 480, false);
        assert!(plain.contains("diff_mode=rectangle") && !plain.contains("reserve_transparent"));
        let alpha = gif_filter("fps=15,", 480, true);
        assert!(alpha.starts_with("fps=15,scale=480:-1") && alpha.contains("reserve_transparent=1"));
    }
}
