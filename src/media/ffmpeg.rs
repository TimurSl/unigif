//! Running ffmpeg and ffprobe as sandboxed subprocesses.
//!
//! Every invocation gets `-nostdin`, a `file`-only protocol whitelist, a
//! wall-clock timeout, and a slot from a global semaphore that caps how many
//! run in parallel.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::config::MediaConfig;

#[derive(Clone)]
pub struct Ffmpeg {
    inner: Arc<Inner>,
}

struct Inner {
    ffmpeg: String,
    ffprobe: String,
    sem: Semaphore,
}

/// Build a `Vec<OsString>` from mixed `&str` / `&Path` / `String` items.
#[macro_export]
macro_rules! args {
    ($($a:expr),* $(,)?) => {{
        let v: Vec<std::ffi::OsString> = vec![$(std::ffi::OsString::from($a)),*];
        v
    }};
}

impl Ffmpeg {
    pub fn new(cfg: &MediaConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                ffmpeg: cfg.ffmpeg.clone(),
                ffprobe: cfg.ffprobe.clone(),
                sem: Semaphore::new(cfg.concurrency),
            }),
        }
    }

    /// Run ffmpeg with `args`; returns stdout. Stderr is folded into the error.
    pub async fn run(&self, args: Vec<OsString>, timeout: Duration) -> Result<Vec<u8>> {
        let mut full = args!["-nostdin", "-hide_banner", "-v", "error", "-y"];
        full.extend(args);
        self.exec(&self.inner.ffmpeg, full, timeout).await
    }

    /// Run ffprobe with `args`; returns stdout.
    pub async fn run_probe(&self, args: Vec<OsString>, timeout: Duration) -> Result<Vec<u8>> {
        let mut full = args!["-v", "error"];
        full.extend(args);
        self.exec(&self.inner.ffprobe, full, timeout).await
    }

    async fn exec(&self, bin: &str, args: Vec<OsString>, timeout: Duration) -> Result<Vec<u8>> {
        let _permit = self.inner.sem.acquire().await.context("ffmpeg semaphore closed")?;
        let mut child = Command::new(bin)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {bin}"))?;
        let mut stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let mut stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;

        let work = async {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let (_, _, status) =
                tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err), child.wait());
            (out, err, status)
        };
        match tokio::time::timeout(timeout, work).await {
            Ok((out, err, status)) => {
                let status = status.with_context(|| format!("waiting for {bin}"))?;
                if !status.success() {
                    bail!("{bin} failed ({status}): {}", tail(&err));
                }
                Ok(out)
            }
            Err(_) => {
                // `kill_on_drop` reaps the process when `child` goes out of scope.
                bail!("{bin} timed out after {}s", timeout.as_secs());
            }
        }
    }

    /// First line of `ffmpeg -version`, for `doctor`.
    pub async fn version(&self) -> Result<String> {
        let out = self.exec(&self.inner.ffmpeg, args!["-version"], Duration::from_secs(10)).await?;
        Ok(String::from_utf8_lossy(&out).lines().next().unwrap_or("").to_string())
    }

    /// Names of the encoders this ffmpeg build offers.
    pub async fn encoders(&self) -> Result<Vec<String>> {
        let out = self
            .exec(&self.inner.ffmpeg, args!["-hide_banner", "-encoders"], Duration::from_secs(10))
            .await?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                let flags = it.next()?;
                let name = it.next()?;
                (flags.len() == 6 && flags.starts_with('V')).then(|| name.to_string())
            })
            .collect())
    }

    pub fn ffprobe_bin(&self) -> &str {
        &self.inner.ffprobe
    }
}

fn tail(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let s = s.trim();
    let start = s.len().saturating_sub(600);
    // stay on a char boundary
    let start = (start..s.len()).find(|i| s.is_char_boundary(*i)).unwrap_or(s.len());
    s[start..].to_string()
}

/// `-protocol_whitelist file [-f demuxer] -i path`
pub fn input(path: &Path, demuxer: Option<&str>) -> Vec<OsString> {
    let mut v = args!["-protocol_whitelist", "file"];
    if let Some(d) = demuxer {
        v.extend(args!["-f", d]);
    }
    v.extend(args!["-i", path]);
    v
}
