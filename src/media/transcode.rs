//! Source → stored variants: `full.mp4`, `preview.mp4`, `poster.webp`, plus
//! the ThumbHash and dHash placeholders.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};

use super::ffmpeg::{input, Ffmpeg};
use super::probe::{self, pix_fmt_has_alpha, ProbeInfo};
use super::{fit_height, fit_long_edge, SourceKind};
use crate::args;
use crate::config::MediaConfig;

/// Files and metadata produced for one GIF. All files live in `dir`.
#[derive(Debug)]
pub struct Encoded {
    pub dir: PathBuf,
    pub width: u32,
    pub height: u32,
    pub duration_ms: i64,
    pub has_alpha: bool,
    pub full: FileInfo,
    pub preview: FileInfo,
    pub poster: FileInfo,
    pub thumbhash: Vec<u8>,
    pub phash: i64,
    /// `true` when `full.mp4` is a lossless remux of the source.
    pub remuxed: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct FileInfo {
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
}

/// Can the source become `full.mp4` by copying packets only?
/// True for H.264 / yuv420p / silent / within the size limit, which is
/// every Telegram GIF.
pub fn can_remux(info: &ProbeInfo, cfg: &MediaConfig) -> bool {
    info.codec == "h264"
        && info.pix_fmt == "yuv420p"
        && !info.has_audio
        && info.width.max(info.height) <= cfg.long_edge
        && info.width % 2 == 0
        && info.height % 2 == 0
        && info.fps <= cfg.max_fps as f64 + 0.5
}

/// Does any pixel of the source actually use transparency?
/// A GIF decodes as `bgra` whether or not it has transparent pixels, so the
/// pixel format alone proves nothing; look at the alpha plane.
pub async fn detect_alpha(ff: &Ffmpeg, src: &Path, kind: SourceKind, info: &ProbeInfo) -> Result<bool> {
    if !pix_fmt_has_alpha(&info.pix_fmt) {
        return Ok(false);
    }
    let mut a = args!["-an"];
    a.extend(input(src, kind.demuxer()));
    a.extend(args![
        "-vf",
        "alphaextract,signalstats,metadata=mode=print:key=lavfi.signalstats.YMIN:file=-",
        "-f",
        "null",
        "-"
    ]);
    let out = ff.run(a, Duration::from_secs(60)).await?;
    let text = String::from_utf8_lossy(&out);
    let min = text
        .lines()
        .filter_map(|l| l.split_once("lavfi.signalstats.YMIN=")?.1.trim().parse::<f64>().ok())
        .fold(f64::INFINITY, f64::min);
    Ok(min.is_finite() && min < 255.0)
}

/// Run the whole pipeline for one source file, writing results into `out_dir`
/// (which must already exist and be empty).
pub async fn process(
    ff: &Ffmpeg,
    cfg: &MediaConfig,
    src: &Path,
    kind: SourceKind,
    out_dir: &Path,
) -> Result<Encoded> {
    let timeout = Duration::from_secs(cfg.transcode_timeout_secs);
    let info = probe::probe_source(ff, src, kind).await.context("probing source")?;
    let has_alpha = detect_alpha(ff, src, kind, &info).await.context("detecting transparency")?;

    let full_path = out_dir.join("full.mp4");
    let remuxed = !has_alpha && can_remux(&info, cfg);
    if remuxed {
        let mut a = args!["-an"];
        a.extend(input(src, kind.demuxer()));
        a.extend(args!["-c", "copy", "-movflags", "+faststart", "-f", "mp4", &full_path]);
        ff.run(a, timeout).await.context("remuxing")?;
    } else {
        encode_full(ff, cfg, src, kind, &info, has_alpha, &full_path, timeout).await.context("encoding")?;
    }

    let full_info = verify_full(ff, &full_path).await?;
    let (pw, ph) = fit_height(full_info.width, full_info.height, cfg.preview_height);

    let preview_path = out_dir.join("preview.mp4");
    let mut a = args!["-an"];
    a.extend(input(&full_path, None));
    a.extend(args![
        "-c:v",
        "libx264",
        "-preset",
        &cfg.preset,
        "-crf",
        cfg.preview_crf.to_string(),
        "-pix_fmt",
        "yuv420p",
        "-vf",
        format!("scale={pw}:{ph}:flags=lanczos"),
        "-fps_mode",
        "vfr",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
        &preview_path
    ]);
    ff.run(a, timeout).await.context("encoding preview")?;

    let poster_path = out_dir.join("poster.webp");
    make_poster(ff, &full_path, full_info.duration_secs, (pw, ph), &poster_path, timeout)
        .await
        .context("extracting poster")?;

    let (tw, th) = fit_long_edge(pw, ph, 100);
    let rgba = raw_frame(ff, &poster_path, tw, th, "rgba", timeout).await.context("thumbhash frame")?;
    let thumbhash = thumbhash::rgba_to_thumb_hash(tw as usize, th as usize, &rgba);
    let gray = raw_frame(ff, &poster_path, 9, 8, "gray", timeout).await.context("dhash frame")?;
    let phash = dhash(&gray).context("dhash")?;

    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).context("stat output");
    Ok(Encoded {
        dir: out_dir.to_path_buf(),
        width: full_info.width,
        height: full_info.height,
        duration_ms: full_info.duration_ms(),
        has_alpha,
        full: FileInfo { bytes: size(&full_path)?, width: full_info.width, height: full_info.height },
        preview: FileInfo { bytes: size(&preview_path)?, width: pw, height: ph },
        poster: FileInfo { bytes: size(&poster_path)?, width: pw, height: ph },
        thumbhash,
        phash,
        remuxed,
    })
}

#[allow(clippy::too_many_arguments)]
async fn encode_full(
    ff: &Ffmpeg,
    cfg: &MediaConfig,
    src: &Path,
    kind: SourceKind,
    info: &ProbeInfo,
    has_alpha: bool,
    out: &Path,
    timeout: Duration,
) -> Result<()> {
    let (w, h) = fit_long_edge(info.width, info.height, cfg.long_edge);
    let mut a = args!["-an"];
    a.extend(input(src, kind.demuxer()));
    let cap_fps = info.fps > cfg.max_fps as f64;
    if has_alpha {
        // H.264 has no alpha: composite onto the matte colour. The matte
        // source is constant-rate, so the output is too.
        let fps = info.fps.clamp(1.0, cfg.max_fps as f64);
        a.extend(args![
            "-f",
            "lavfi",
            "-i",
            format!("color=c=0x{}:s={w}x{h}:r={fps:.3}", cfg.matte),
            "-filter_complex",
            format!(
                "[0:v]scale={w}:{h}:flags=lanczos,format=rgba[fg];\
                 [1:v][fg]overlay=shortest=1:format=auto,format=yuv420p[v]"
            ),
            "-map",
            "[v]",
            "-fps_mode",
            "cfr"
        ]);
    } else {
        let mut vf = format!("scale={w}:{h}:flags=lanczos");
        if cap_fps {
            vf.push_str(&format!(",fps={}", cfg.max_fps));
        }
        a.extend(args!["-vf", vf, "-fps_mode", "vfr"]);
    }
    a.extend(args![
        "-c:v",
        "libx264",
        "-preset",
        &cfg.preset,
        "-crf",
        cfg.crf.to_string(),
        "-profile:v",
        "high",
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
        out
    ]);
    ff.run(a, timeout).await?;
    Ok(())
}

/// Check the encoded `full.mp4` is what we promise to serve.
async fn verify_full(ff: &Ffmpeg, path: &Path) -> Result<ProbeInfo> {
    let info = probe::probe(ff, path, Some("mov")).await.context("probing output")?;
    if info.codec != "h264" || info.pix_fmt != "yuv420p" {
        bail!("output is {}/{}, expected h264/yuv420p", info.codec, info.pix_fmt);
    }
    if info.has_audio {
        bail!("output unexpectedly has an audio stream");
    }
    if info.width % 2 != 0 || info.height % 2 != 0 {
        bail!("output has odd dimensions {}x{}", info.width, info.height);
    }
    Ok(info)
}

async fn make_poster(
    ff: &Ffmpeg,
    full: &Path,
    duration_secs: f64,
    (w, h): (u32, u32),
    out: &Path,
    timeout: Duration,
) -> Result<()> {
    // One frame from about 10 % in; fall back to the first frame when the
    // seek lands past the last one.
    let at = if duration_secs >= 1.0 { duration_secs * 0.1 } else { 0.0 };
    for ss in [at, 0.0] {
        let mut a = args!["-ss", format!("{ss:.3}")];
        a.extend(input(full, None));
        a.extend(args![
            "-frames:v",
            "1",
            "-vf",
            format!("scale={w}:{h}:flags=lanczos"),
            "-c:v",
            "libwebp",
            "-quality",
            "80",
            "-f",
            "webp",
            out
        ]);
        let _ = std::fs::remove_file(out);
        // A seek past the last frame can exit 0 and leave an empty image, so
        // only a poster that decodes counts.
        if ff.run(a, timeout).await.is_ok() && raw_frame(ff, out, 2, 2, "gray", timeout).await.is_ok() {
            return Ok(());
        }
        if ss == 0.0 {
            break;
        }
    }
    bail!("could not extract a poster frame")
}

/// Decode `img` scaled to `w`×`h` into raw pixels of the given format.
async fn raw_frame(
    ff: &Ffmpeg,
    img: &Path,
    w: u32,
    h: u32,
    pix_fmt: &str,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let mut a = args!["-an"];
    a.extend(input(img, None));
    a.extend(args![
        "-frames:v",
        "1",
        "-vf",
        format!("scale={w}:{h}:flags=area"),
        "-f",
        "rawvideo",
        "-pix_fmt",
        pix_fmt,
        "-"
    ]);
    let out = ff.run(a, timeout).await?;
    let channels = if pix_fmt == "rgba" { 4 } else { 1 };
    let expected = (w * h) as usize * channels;
    if out.len() != expected {
        bail!("expected {expected} raw bytes, got {}", out.len());
    }
    Ok(out)
}

/// 64-bit difference hash of a 9×8 grayscale image: one bit per horizontally
/// adjacent pair, set when the left pixel is darker than the right.
pub fn dhash(gray_9x8: &[u8]) -> Result<i64> {
    if gray_9x8.len() != 72 {
        bail!("dhash needs 72 bytes, got {}", gray_9x8.len());
    }
    let mut bits: u64 = 0;
    for y in 0..8 {
        for x in 0..8 {
            let l = gray_9x8[y * 9 + x];
            let r = gray_9x8[y * 9 + x + 1];
            bits = (bits << 1) | u64::from(l < r);
        }
    }
    Ok(bits as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h264(w: u32, h: u32) -> ProbeInfo {
        ProbeInfo {
            codec: "h264".into(),
            width: w,
            height: h,
            pix_fmt: "yuv420p".into(),
            frames: Some(30),
            duration_secs: 2.0,
            fps: 15.0,
            has_audio: false,
        }
    }

    #[test]
    fn remux_rules() {
        let cfg = MediaConfig::default();
        assert!(can_remux(&h264(480, 270), &cfg));
        assert!(!can_remux(&h264(1280, 720), &cfg), "too large");
        assert!(!can_remux(&h264(481, 270), &cfg), "odd width");
        let mut audio = h264(480, 270);
        audio.has_audio = true;
        assert!(!can_remux(&audio, &cfg));
        let mut hevc = h264(480, 270);
        hevc.codec = "hevc".into();
        assert!(!can_remux(&hevc, &cfg));
        let mut fast = h264(480, 270);
        fast.fps = 60.0;
        assert!(!can_remux(&fast, &cfg));
        let mut p10 = h264(480, 270);
        p10.pix_fmt = "yuv420p10le".into();
        assert!(!can_remux(&p10, &cfg));
    }

    #[test]
    fn dhash_bits() {
        // Strictly increasing rows: every left pixel is darker -> all ones.
        let inc: Vec<u8> = (0..8).flat_map(|_| 0..9u8).collect();
        assert_eq!(dhash(&inc).unwrap(), -1); // 0xFFFF_FFFF_FFFF_FFFF
                                              // Strictly decreasing rows -> all zeros.
        let dec: Vec<u8> = (0..8).flat_map(|_| (0..9u8).rev()).collect();
        assert_eq!(dhash(&dec).unwrap(), 0);
        assert!(dhash(&[0; 10]).is_err());
    }
}
