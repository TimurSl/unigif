//! Everything that touches media bytes: sniffing, probing, transcoding,
//! variant naming and the on-disk layout.

use std::path::{Path, PathBuf};

pub mod ffmpeg;
pub mod probe;
pub mod transcode;

/// What kind of file the uploaded bytes are, decided from content (never from
/// the file name or the client's `Content-Type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Gif,
    Webp,
    Png,
    Mp4,
    Matroska,
}

impl SourceKind {
    /// Sniff the first bytes of a file. `None` means "not an accepted type".
    pub fn sniff(head: &[u8]) -> Option<Self> {
        if let Some(t) = infer::get(head) {
            match t.mime_type() {
                "image/gif" => return Some(Self::Gif),
                "image/webp" => return Some(Self::Webp),
                "image/png" => return Some(Self::Png),
                "video/mp4" | "video/quicktime" => return Some(Self::Mp4),
                "video/webm" | "video/x-matroska" => return Some(Self::Matroska),
                _ => {}
            }
        }
        // Some valid MP4 brands are unknown to `infer`.
        if head.len() >= 12 && &head[4..8] == b"ftyp" {
            return Some(Self::Mp4);
        }
        None
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Png => "image/png",
            Self::Mp4 => "video/mp4",
            Self::Matroska => "video/x-matroska",
        }
    }

    /// File extension used when the original is kept.
    pub fn ext(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Png => "png",
            Self::Mp4 => "mp4",
            Self::Matroska => "mkv",
        }
    }

    pub fn from_mime(m: &str) -> Option<Self> {
        [Self::Gif, Self::Webp, Self::Png, Self::Mp4, Self::Matroska].into_iter().find(|k| k.mime() == m)
    }

    /// Force ffmpeg's demuxer where we can, so a crafted file cannot make it
    /// pick an exotic demuxer (playlists, concat scripts, …).
    pub fn demuxer(self) -> Option<&'static str> {
        match self {
            Self::Gif => Some("gif"),
            Self::Mp4 => Some("mov"),
            Self::Matroska => Some("matroska"),
            Self::Webp | Self::Png => None,
        }
    }
}

/// The fixed list of variant names that exist. There is no free-form size
/// parameter, so nobody can fill the disk by requesting thousands of sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    FullMp4,
    PreviewMp4,
    PosterWebp,
}

impl Variant {
    pub const STORED: [Variant; 3] = [Self::FullMp4, Self::PreviewMp4, Self::PosterWebp];

    pub fn parse(name: &str) -> Option<Self> {
        Self::STORED.into_iter().find(|v| v.file_name() == name)
    }

    pub fn file_name(self) -> &'static str {
        match self {
            Self::FullMp4 => "full.mp4",
            Self::PreviewMp4 => "preview.mp4",
            Self::PosterWebp => "poster.webp",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::FullMp4 | Self::PreviewMp4 => "video/mp4",
            Self::PosterWebp => "image/webp",
        }
    }
}

/// `media/7K/7Kq2mXw9Fb3T/`
pub fn gif_dir(media_dir: &Path, id: &str) -> PathBuf {
    let shard = id.get(..2).unwrap_or(id);
    media_dir.join(shard).join(id)
}

/// A GIF id is exactly 12 base58 characters. Checked before an id is ever
/// used to build a path.
pub fn valid_id(id: &str) -> bool {
    id.len() == ID_LEN && id.bytes().all(|b| ID_ALPHABET.contains(&b))
}

pub const ID_LEN: usize = 12;
const ID_ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

pub fn new_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..ID_LEN).map(|_| ID_ALPHABET[rng.random_range(0..ID_ALPHABET.len())] as char).collect()
}

/// Largest size that fits `long_edge` on its longest side, never upscaled,
/// with even dimensions (required by yuv420p).
pub fn fit_long_edge(w: u32, h: u32, long_edge: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= long_edge {
        return (even(w), even(h));
    }
    let f = long_edge as f64 / long as f64;
    (even((w as f64 * f).round() as u32), even((h as f64 * f).round() as u32))
}

/// Largest size no taller than `max_h`, never upscaled, even dimensions.
pub fn fit_height(w: u32, h: u32, max_h: u32) -> (u32, u32) {
    if h <= max_h {
        return (even(w), even(h));
    }
    let f = max_h as f64 / h as f64;
    (even((w as f64 * f).round() as u32), even(max_h))
}

fn even(v: u32) -> u32 {
    let v = v.max(2);
    v - (v % 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_even_and_never_upscaled() {
        assert_eq!(fit_long_edge(480, 270, 720), (480, 270));
        assert_eq!(fit_long_edge(1920, 1080, 720), (720, 404));
        assert_eq!(fit_long_edge(1081, 1921, 720), (404, 720));
        assert_eq!(fit_long_edge(1, 1, 720), (2, 2));
        assert_eq!(fit_height(1280, 720, 200), (356, 200));
        assert_eq!(fit_height(100, 50, 200), (100, 50));
        assert_eq!(fit_height(333, 333, 200), (200, 200));
    }

    #[test]
    fn ids_are_valid_base58() {
        for _ in 0..100 {
            let id = new_id();
            assert!(valid_id(&id), "{id}");
        }
        assert!(!valid_id("../etc/passwd"));
        assert!(!valid_id("0OIl00000000")); // excluded characters
        assert!(!valid_id("short"));
    }

    #[test]
    fn variant_names_are_fixed() {
        assert_eq!(Variant::parse("full.mp4"), Some(Variant::FullMp4));
        assert_eq!(Variant::parse("../full.mp4"), None);
        assert_eq!(Variant::parse("original.gif"), None);
    }

    #[test]
    fn sniffs_content_not_names() {
        assert_eq!(SourceKind::sniff(b"GIF89a\x01\x00\x01\x00\x00\x00\x00"), Some(SourceKind::Gif));
        assert_eq!(SourceKind::sniff(b"<html>not media</html>"), None);
        let mut mp4 = vec![0, 0, 0, 0x18];
        mp4.extend_from_slice(b"ftypisom\0\0\0\0isom");
        assert_eq!(SourceKind::sniff(&mp4), Some(SourceKind::Mp4));
    }
}
