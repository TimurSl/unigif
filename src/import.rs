//! `unigif import <dir>`: bulk-add an existing folder of GIFs and videos.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio_util::io::ReaderStream;

use crate::app::App;
use crate::db::gifs;
use crate::ingest::{self, worker, AddRequest};
use crate::tags;

const EXTENSIONS: [&str; 8] = ["gif", "webp", "png", "mp4", "m4v", "mov", "webm", "mkv"];

#[derive(Debug, Default)]
pub struct Summary {
    pub added: usize,
    pub duplicates: usize,
    pub rejected: usize,
    pub failed: usize,
}

pub struct Options {
    pub dir: PathBuf,
    pub tags_from_folders: bool,
}

pub async fn run(app: &App, opts: &Options) -> Result<Summary> {
    let mut files = Vec::new();
    collect(&opts.dir, &mut files).with_context(|| format!("reading {}", opts.dir.display()))?;
    files.sort();
    println!("found {} candidate files", files.len());

    let mut summary = Summary::default();
    let mut ids = Vec::new();
    for (i, path) in files.iter().enumerate() {
        let rel = path.strip_prefix(&opts.dir).unwrap_or(path);
        let mut req = AddRequest::new("import");
        if opts.tags_from_folders {
            req.tags = folder_tags(rel);
        }
        let result = async {
            let file = tokio::fs::File::open(path).await.map_err(anyhow::Error::from)?;
            let upload = ingest::receive(app, ReaderStream::new(file)).await?;
            ingest::add(app, upload, req).await
        }
        .await;
        match result {
            Ok(added) if added.duplicate => {
                summary.duplicates += 1;
                println!("[{}/{}] duplicate  {}", i + 1, files.len(), rel.display());
            }
            Ok(added) => {
                summary.added += 1;
                ids.push(added.gif.id);
                println!("[{}/{}] queued     {}", i + 1, files.len(), rel.display());
            }
            Err(e) => {
                summary.rejected += 1;
                println!("[{}/{}] rejected   {}: {e}", i + 1, files.len(), rel.display());
            }
        }
    }

    println!("waiting for {} transcodes…", ids.len());
    while !worker::idle(app).await? {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    for id in &ids {
        if gifs::get(&app.db, id).await?.is_some_and(|g| g.status == "failed") {
            summary.failed += 1;
        }
    }
    Ok(summary)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let path = entry.path();
        // Symlinks are not followed: an import must stay inside the folder given.
        if ty.is_symlink() || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if ty.is_dir() {
            collect(&path, out)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Folder names above a file become hashtags: `cats/funny/a.gif` → `cats`, `funny`.
pub fn folder_tags(rel: &Path) -> Vec<String> {
    let Some(parent) = rel.parent() else { return vec![] };
    let mut out: Vec<String> = Vec::new();
    for comp in parent.components() {
        let name = comp.as_os_str().to_string_lossy();
        let cleaned: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect::<String>()
            .trim_matches('_')
            .to_string();
        if let Ok(tag) = tags::normalize(&cleaned) {
            if !out.contains(&tag) {
                out.push(tag);
            }
        }
    }
    out.truncate(tags::MAX_TAGS_PER_GIF);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_become_tags() {
        assert_eq!(folder_tags(Path::new("a.gif")), Vec::<String>::new());
        assert_eq!(folder_tags(Path::new("Cats/funny cats/a.gif")), vec!["cats", "funny_cats"]);
        assert_eq!(folder_tags(Path::new("cats/cats/a.gif")), vec!["cats"]);
        assert_eq!(folder_tags(Path::new("--/a.gif")), Vec::<String>::new());
    }
}
