//! `ffprobe` wrapper and the pre-decode limit checks.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::ffmpeg::Ffmpeg;
use super::SourceKind;
use crate::args;
use crate::config::MediaConfig;

#[derive(Debug, Clone)]
pub struct ProbeInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: String,
    /// Frame count if the container states it.
    pub frames: Option<u64>,
    pub duration_secs: f64,
    pub fps: f64,
    pub has_audio: bool,
}

impl ProbeInfo {
    /// Frame count from the container, else duration × fps.
    pub fn estimated_frames(&self) -> u64 {
        self.frames.unwrap_or_else(|| (self.duration_secs * self.fps).ceil() as u64)
    }

    pub fn duration_ms(&self) -> i64 {
        (self.duration_secs * 1000.0).round() as i64
    }
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Deserialize)]
struct RawStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    pix_fmt: Option<String>,
    nb_frames: Option<String>,
    duration: Option<String>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    disposition: Option<RawDisposition>,
}

#[derive(Deserialize)]
struct RawDisposition {
    attached_pic: Option<u8>,
}

#[derive(Deserialize)]
struct RawFormat {
    duration: Option<String>,
}

fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/')?;
    let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
    (d != 0.0 && n > 0.0).then(|| n / d)
}

pub fn parse(json: &[u8]) -> Result<ProbeInfo> {
    let raw: Raw = serde_json::from_slice(json).context("parsing ffprobe output")?;
    let has_audio = raw.streams.iter().any(|s| s.codec_type.as_deref() == Some("audio"));
    let v = raw
        .streams
        .iter()
        .find(|s| {
            s.codec_type.as_deref() == Some("video")
                && s.disposition.as_ref().and_then(|d| d.attached_pic) != Some(1)
        })
        .context("no video stream")?;
    let width = v.width.filter(|w| *w > 0).context("video stream has no width")?;
    let height = v.height.filter(|h| *h > 0).context("video stream has no height")?;
    let fps = v
        .avg_frame_rate
        .as_deref()
        .and_then(parse_rate)
        .or_else(|| v.r_frame_rate.as_deref().and_then(parse_rate))
        .unwrap_or(10.0);
    let dur = |s: &Option<String>| s.as_deref().and_then(|d| d.parse::<f64>().ok());
    let duration_secs =
        dur(&v.duration).or_else(|| raw.format.as_ref().and_then(|f| dur(&f.duration))).unwrap_or(0.0);
    Ok(ProbeInfo {
        codec: v.codec_name.clone().unwrap_or_default(),
        width,
        height,
        pix_fmt: v.pix_fmt.clone().unwrap_or_default(),
        frames: v.nb_frames.as_deref().and_then(|n| n.parse().ok()).filter(|n| *n > 0),
        duration_secs,
        fps,
        has_audio,
    })
}

pub async fn probe(ff: &Ffmpeg, path: &Path, demuxer: Option<&str>) -> Result<ProbeInfo> {
    let mut a = args!["-protocol_whitelist", "file"];
    if let Some(d) = demuxer {
        a.extend(args!["-f", d]);
    }
    a.extend(args!["-print_format", "json", "-show_format", "-show_streams", "-i", path]);
    let out = ff.run_probe(a, Duration::from_secs(20)).await?;
    parse(&out)
}

pub async fn probe_source(ff: &Ffmpeg, path: &Path, kind: SourceKind) -> Result<ProbeInfo> {
    probe(ff, path, kind.demuxer()).await
}

/// Decode the first frame. `ffprobe` only reads headers, so a file with a
/// plausible header and garbage inside passes it; this catches that cheaply.
pub async fn check_decodes(ff: &Ffmpeg, path: &Path, kind: SourceKind) -> Result<()> {
    let mut a = args!["-an"];
    a.extend(super::ffmpeg::input(path, kind.demuxer()));
    a.extend(args!["-frames:v", "1", "-f", "null", "-"]);
    ff.run(a, Duration::from_secs(20)).await.map(|_| ())
}

/// Reject files over the configured limits before anything decodes them.
/// The error string is safe to show to the uploader.
pub fn check_limits(info: &ProbeInfo, cfg: &MediaConfig) -> Result<(), String> {
    if info.width > cfg.max_width || info.height > cfg.max_height {
        return Err(format!(
            "dimensions {}x{} exceed the {}x{} limit",
            info.width, info.height, cfg.max_width, cfg.max_height
        ));
    }
    if info.duration_secs > cfg.max_duration_secs as f64 {
        return Err(format!(
            "duration {:.1}s exceeds the {}s limit",
            info.duration_secs, cfg.max_duration_secs
        ));
    }
    if info.estimated_frames() > cfg.max_frames as u64 {
        return Err(format!(
            "about {} frames exceed the {} frame limit",
            info.estimated_frames(),
            cfg.max_frames
        ));
    }
    Ok(())
}

/// Pixel formats that can carry transparency.
pub fn pix_fmt_has_alpha(fmt: &str) -> bool {
    matches!(
        fmt,
        "pal8" | "rgba" | "bgra" | "argb" | "abgr" | "ya8" | "ya16le" | "ya16be" | "ayuv" | "ayuv64le"
    ) || fmt.starts_with("yuva")
        || fmt.starts_with("gbrap")
        || fmt.starts_with("rgba")
        || fmt.starts_with("bgra")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIF_JSON: &str = r#"{"streams":[{"codec_type":"video","codec_name":"gif","width":480,"height":270,
      "pix_fmt":"bgra","avg_frame_rate":"25/2","r_frame_rate":"25/2","duration":"2.400000","nb_frames":"30"}],
      "format":{"duration":"2.400000"}}"#;

    #[test]
    fn parses_ffprobe_json() {
        let i = parse(GIF_JSON.as_bytes()).unwrap();
        assert_eq!((i.codec.as_str(), i.width, i.height), ("gif", 480, 270));
        assert_eq!(i.frames, Some(30));
        assert!((i.fps - 12.5).abs() < 1e-9);
        assert_eq!(i.duration_ms(), 2400);
        assert!(!i.has_audio);
    }

    #[test]
    fn missing_frame_count_is_estimated() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":10,"height":10,
          "avg_frame_rate":"10/1","nb_frames":"N/A"}],"format":{"duration":"3.0"}}"#;
        let i = parse(json.as_bytes()).unwrap();
        assert_eq!(i.frames, None);
        assert_eq!(i.estimated_frames(), 30);
    }

    #[test]
    fn audio_is_detected() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":10,"height":10,
          "avg_frame_rate":"10/1"},{"codec_type":"audio","codec_name":"aac"}],"format":{}}"#;
        assert!(parse(json.as_bytes()).unwrap().has_audio);
    }

    #[test]
    fn no_video_stream_is_an_error() {
        assert!(parse(br#"{"streams":[{"codec_type":"audio"}],"format":{}}"#).is_err());
        assert!(parse(b"{}").is_err());
    }

    #[test]
    fn limits_are_enforced() {
        let cfg = MediaConfig::default();
        let mut i = parse(GIF_JSON.as_bytes()).unwrap();
        assert!(check_limits(&i, &cfg).is_ok());
        i.width = 5000;
        assert!(check_limits(&i, &cfg).unwrap_err().contains("dimensions"));
        i.width = 480;
        i.duration_secs = 61.0;
        assert!(check_limits(&i, &cfg).unwrap_err().contains("duration"));
        i.duration_secs = 10.0;
        i.frames = Some(3001);
        assert!(check_limits(&i, &cfg).unwrap_err().contains("frame"));
    }

    #[test]
    fn alpha_formats() {
        assert!(pix_fmt_has_alpha("bgra"));
        assert!(pix_fmt_has_alpha("yuva420p"));
        assert!(!pix_fmt_has_alpha("yuv420p"));
        assert!(!pix_fmt_has_alpha("gray"));
    }
}
