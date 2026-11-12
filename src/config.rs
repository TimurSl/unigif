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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AuthConfig {
    /// Interim static bearer token used until API keys land (milestone M2).
    /// Without it every `/api/v1` request is refused.
    pub bootstrap_token: Option<String>,
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
        std::env::set_var("UNIGIF_AUTH__BOOTSTRAP_TOKEN", "tok");
        let cfg = Config::load(None);
        for k in [
            "UNIGIF_DATA_DIR",
            "UNIGIF_UPLOAD__MAX_SIZE",
            "UNIGIF_MEDIA__KEEP_ORIGINALS",
            "UNIGIF_AUTH__BOOTSTRAP_TOKEN",
        ] {
            std::env::remove_var(k);
        }
        let cfg = cfg.unwrap();
        assert_eq!(cfg.data_dir, PathBuf::from("/srv/unigif"));
        assert_eq!(cfg.upload.max_size.0, 20 * 1024 * 1024);
        assert_eq!(cfg.media.keep_originals, KeepOriginals::Always);
        assert_eq!(cfg.auth.bootstrap_token.as_deref(), Some("tok"));
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
