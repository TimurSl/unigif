//! Configuration: built-in defaults, then an optional TOML file, then
//! `UNIGIF_*` environment variables (nested keys separated by `__`,
//! e.g. `UNIGIF_UPLOAD__MAX_SIZE=20MiB`).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A byte count that deserialises from a number or a string such as `50MiB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSize(pub u64);

impl ByteSize {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let split = s.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(s.len());
        let (num, unit) = s.split_at(split);
        let num: f64 = num.parse().with_context(|| format!("invalid size {s:?}"))?;
        let mult: f64 = match unit.trim().to_ascii_lowercase().as_str() {
            "" | "b" => 1.0,
            "k" | "kb" => 1e3,
            "m" | "mb" => 1e6,
            "g" | "gb" => 1e9,
            "kib" => 1024.0,
            "mib" => 1024.0 * 1024.0,
            "gib" => 1024.0 * 1024.0 * 1024.0,
            other => bail!("unknown size unit {other:?} in {s:?}"),
        };
        Ok(Self((num * mult) as u64))
    }
}

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Num(u64),
            Str(String),
        }
        match Raw::deserialize(d)? {
            Raw::Num(n) => Ok(Self(n)),
            Raw::Str(s) => Self::parse(&s).map_err(serde::de::Error::custom),
        }
    }
}

impl Serialize for ByteSize {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeepOriginals {
    /// Keep the original only when it has transparency (default).
    Alpha,
    Never,
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Public base URL, used to build absolute media URLs. Empty = relative URLs.
    pub public_url: String,
    pub listen: String,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub upload: UploadConfig,
    pub media: MediaConfig,
    pub auth: AuthConfig,
    pub rate_limit: RateLimitConfig,
    pub cors: CorsConfig,
    pub fetch: FetchConfig,
    pub trash: TrashConfig,
    pub cache: CacheConfig,
    pub search: SearchConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CacheConfig {
    /// Upper bound for the on-demand conversion cache; the least recently
    /// used files are evicted beyond it. `0` disables caching beyond the
    /// file currently being served.
    pub max_size: ByteSize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UploadConfig {
    pub max_size: ByteSize,
    /// Default seconds a quick add waits for processing to finish.
    pub wait_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MediaConfig {
    pub keep_originals: KeepOriginals,
    /// Colour transparent pixels are composited onto, `RRGGBB`.
    pub matte: String,
    pub crf: u8,
    pub preset: String,
    pub long_edge: u32,
    pub max_fps: u32,
    /// On-demand `full.gif`: widest edge in pixels and frame-rate cap.
    pub gif_width: u32,
    pub gif_fps: u32,
    /// On-demand `preview.gif`.
    pub preview_gif_width: u32,
    pub preview_gif_fps: u32,
    /// On-demand `full.webp`: widest edge in pixels, quality 0-100.
    pub webp_width: u32,
    pub webp_quality: u8,
    /// On-demand `full.webm` (VP9 constant quality, lower is better).
    pub webm_crf: u8,
    pub preview_crf: u8,
    pub preview_height: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub max_duration_secs: u32,
    pub max_frames: u32,
    /// Parallel ffmpeg processes.
    pub concurrency: usize,
    /// Wall-clock limit for one on-demand ffmpeg run.
    pub timeout_secs: u64,
    /// Wall-clock limit for the ingest transcode of one GIF.
    pub transcode_timeout_secs: u64,
    pub ffmpeg: String,
    pub ffprobe: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AuthConfig {
    /// Creates the first user at start-up when the users table is empty.
    pub initial_owner: InitialOwner,
    /// Session lifetime; every use inside the window extends it.
    pub session_days: u32,
    /// Proxies whose `X-Forwarded-For` is believed (exact IP addresses).
    pub trusted_proxies: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InitialOwner {
    pub username: Option<String>,
    /// File holding the password (first line, trailing newline stripped).
    pub password_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RateLimitConfig {
    /// Requests per minute per key for everything except uploads.
    pub read_per_min: u32,
    /// Requests per minute per key for `POST /gifs`.
    pub upload_per_min: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CorsConfig {
    /// Origins allowed to call the API from a browser; `*` allows any.
    /// Cookies are never sent cross-origin whatever this says.
    pub origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FetchConfig {
    /// Allow quick add by URL to reach private, loopback and link-local
    /// addresses. Off in production; the test suite turns it on.
    pub allow_private: bool,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrashConfig {
    /// Days a trashed GIF is kept before its files are deleted.
    pub retention_days: u32,
}

/// Third-party GIF search. The API keys can also be entered in Settings (those win).
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchConfig {
    pub giphy_api_key: Option<String>,
    pub klipy_api_key: Option<String>,
    pub giphy_base_url: String,
    pub klipy_base_url: String,
    /// GIPHY content rating: `g`, `pg`, `pg-13` or `r`.
    pub rating: String,
    pub timeout_secs: u64,
}

// Hand-written so that a stray `{:?}` never prints an API key.
impl std::fmt::Debug for SearchConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchConfig")
            .field("giphy_api_key", &self.giphy_api_key.as_ref().map(|_| "<set>"))
            .field("klipy_api_key", &self.klipy_api_key.as_ref().map(|_| "<set>"))
            .field("giphy_base_url", &self.giphy_base_url)
            .field("klipy_base_url", &self.klipy_base_url)
            .field("rating", &self.rating)
            .finish()
    }
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            giphy_api_key: None,
            klipy_api_key: None,
            giphy_base_url: "https://api.giphy.com/v1".into(),
            klipy_base_url: "https://api.klipy.com/api/v1".into(),
            rating: "pg-13".into(),
            timeout_secs: 15,
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self { initial_owner: InitialOwner::default(), session_days: 30, trusted_proxies: vec![] }
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self { read_per_min: 120, upload_per_min: 30 }
    }
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self { origins: vec!["*".into()] }
    }
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self { allow_private: false, timeout_secs: 30 }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { max_size: ByteSize(2 * 1024 * 1024 * 1024) }
    }
}

impl Default for TrashConfig {
    fn default() -> Self {
        Self { retention_days: 30 }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            public_url: String::new(),
            listen: "127.0.0.1:8787".into(),
            data_dir: "./data".into(),
            cache_dir: "./data/cache".into(),
            upload: UploadConfig::default(),
            media: MediaConfig::default(),
            auth: AuthConfig::default(),
            rate_limit: RateLimitConfig::default(),
            cors: CorsConfig::default(),
            fetch: FetchConfig::default(),
            trash: TrashConfig::default(),
            cache: CacheConfig::default(),
            search: SearchConfig::default(),
        }
    }
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self { max_size: ByteSize(50 * 1024 * 1024), wait_secs: 10 }
    }
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            keep_originals: KeepOriginals::Alpha,
            matte: "000000".into(),
            crf: 23,
            preset: "slow".into(),
            long_edge: 720,
            max_fps: 30,
            gif_width: 480,
            gif_fps: 15,
            preview_gif_width: 220,
            preview_gif_fps: 10,
            webp_width: 480,
            webp_quality: 70,
            webm_crf: 33,
            preview_crf: 28,
            preview_height: 200,
            max_width: 4096,
            max_height: 4096,
            max_duration_secs: 60,
            max_frames: 3000,
            concurrency: 2,
            timeout_secs: 30,
            transcode_timeout_secs: 300,
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        }
    }
}

impl Config {
    pub fn load(file: Option<&Path>) -> Result<Self> {
        let mut fig = Figment::from(Serialized::defaults(Config::default()));
        if let Some(f) = file {
            fig = fig.merge(Toml::file_exact(f));
        }
        // UPLOAD__MAX_SIZE -> upload.maxSize, so env names mirror the TOML keys.
        // `lowercase(false)` must come after `map` (which resets it): figment
        // would otherwise lower-case the mapped key and destroy the camelCase.
        let env = Env::prefixed("UNIGIF_").map(|k| env_key_to_path(k.as_str()).into()).lowercase(false);
        fig = fig.merge(env);
        let cfg: Config = fig.extract().context("invalid configuration")?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        if self.media.matte.len() != 6 || !self.media.matte.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("media.matte must be a 6-digit hex colour like 000000");
        }
        if self.media.concurrency == 0 {
            bail!("media.concurrency must be at least 1");
        }
        if self.media.long_edge < 16 || self.media.preview_height < 16 {
            bail!("media.longEdge and media.previewHeight must be at least 16");
        }
        for (name, v) in [
            ("gifWidth", self.media.gif_width),
            ("gifFps", self.media.gif_fps),
            ("previewGifWidth", self.media.preview_gif_width),
            ("previewGifFps", self.media.preview_gif_fps),
            ("webpWidth", self.media.webp_width),
        ] {
            if v == 0 {
                bail!("media.{name} must be at least 1");
            }
        }
        if self.media.webp_quality > 100 || self.media.webm_crf > 63 {
            bail!("media.webpQuality is 0-100 and media.webmCrf is 0-63");
        }
        if self.auth.session_days == 0 {
            bail!("auth.sessionDays must be at least 1");
        }
        if self.rate_limit.read_per_min == 0 || self.rate_limit.upload_per_min == 0 {
            bail!("rateLimit values must be at least 1");
        }
        if !["g", "pg", "pg-13", "r"].contains(&self.search.rating.as_str()) {
            bail!("search.rating must be g, pg, pg-13 or r");
        }
        for p in &self.auth.trusted_proxies {
            if p.parse::<std::net::IpAddr>().is_err() {
                bail!("auth.trustedProxies entry {p:?} is not an IP address");
            }
        }
        Ok(())
    }

    pub fn media_dir(&self) -> PathBuf {
        self.data_dir.join("media")
    }
    pub fn tmp_dir(&self) -> PathBuf {
        self.data_dir.join("tmp")
    }
    /// Sources of GIFs that are queued or being transcoded. Unlike `tmp/`
    /// this survives a restart, so unfinished jobs can resume.
    pub fn work_dir(&self) -> PathBuf {
        self.data_dir.join("work")
    }
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("unigif.db")
    }
    /// Where on-demand conversions of one GIF live.
    pub fn cache_gif_dir(&self, id: &str) -> PathBuf {
        crate::media::gif_dir(&self.cache_dir, id)
    }
    /// Base for absolute URLs, without a trailing slash.
    pub fn base_url(&self) -> &str {
        self.public_url.trim_end_matches('/')
    }
}

/// `UPLOAD__MAX_SIZE` becomes `upload.maxSize`.
fn env_key_to_path(key: &str) -> String {
    key.to_ascii_lowercase()
        .split("__")
        .map(|seg| {
            let mut out = String::new();
            let mut upper = false;
            for c in seg.chars() {
                if c == '_' {
                    upper = true;
                } else if upper {
                    out.extend(c.to_uppercase());
                    upper = false;
                } else {
                    out.push(c);
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `Config::load` reads the process environment, and one test sets
    /// `UNIGIF_*` variables, so every test that loads must hold this lock.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn parses_sizes() {
        assert_eq!(ByteSize::parse("50MiB").unwrap().0, 50 * 1024 * 1024);
        assert_eq!(ByteSize::parse("2 GiB").unwrap().0, 2 * 1024 * 1024 * 1024);
        assert_eq!(ByteSize::parse("1500").unwrap().0, 1500);
        assert_eq!(ByteSize::parse("1.5kb").unwrap().0, 1500);
        assert!(ByteSize::parse("12 parsecs").is_err());
    }

    #[test]
    fn env_keys_map_to_camel_case_paths() {
        assert_eq!(env_key_to_path("upload__max_size"), "upload.maxSize");
        assert_eq!(env_key_to_path("public_url"), "publicUrl");
        assert_eq!(env_key_to_path("media__keep_originals"), "media.keepOriginals");
    }

    #[test]
    fn env_overrides_reach_nested_camel_case_fields() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("UNIGIF_DATA_DIR", "/srv/unigif");
        std::env::set_var("UNIGIF_UPLOAD__MAX_SIZE", "20MiB");
        std::env::set_var("UNIGIF_MEDIA__KEEP_ORIGINALS", "always");
        std::env::set_var("UNIGIF_AUTH__SESSION_DAYS", "7");
        let cfg = Config::load(None);
        for k in [
            "UNIGIF_DATA_DIR",
            "UNIGIF_UPLOAD__MAX_SIZE",
            "UNIGIF_MEDIA__KEEP_ORIGINALS",
            "UNIGIF_AUTH__SESSION_DAYS",
        ] {
            std::env::remove_var(k);
        }
        let cfg = cfg.unwrap();
        assert_eq!(cfg.data_dir, PathBuf::from("/srv/unigif"));
        assert_eq!(cfg.upload.max_size.0, 20 * 1024 * 1024);
        assert_eq!(cfg.media.keep_originals, KeepOriginals::Always);
        assert_eq!(cfg.auth.session_days, 7);
    }

    #[test]
    fn toml_file_uses_camel_case_keys() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("unigif-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("c.toml");
        std::fs::write(&f, "publicUrl = \"https://gif.example.com/\"\n[upload]\nmaxSize = \"5MiB\"\n[media]\nkeepOriginals = \"never\"\n").unwrap();
        let cfg = Config::load(Some(&f)).unwrap();
        assert_eq!(cfg.base_url(), "https://gif.example.com");
        assert_eq!(cfg.upload.max_size.0, 5 * 1024 * 1024);
        assert_eq!(cfg.media.keep_originals, KeepOriginals::Never);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn defaults_validate() {
        Config::default().validate().unwrap();
    }
}
