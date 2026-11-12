use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use unigif::app::{App, AppState};
use unigif::config::Config;
use unigif::ingest::worker;
use unigif::{api, db, doctor, import};

#[derive(Parser)]
#[command(name = "unigif", version, about = "Self-hosted, hashtag-first GIF library")]
struct Cli {
    /// Path to a TOML config file (environment variables override it).
    #[arg(long, short, global = true, env = "UNIGIF_CONFIG")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server.
    Serve,
    /// Check that ffmpeg, the data directories and the database are usable.
    Doctor,
    /// Bulk-import a folder of GIFs and videos.
    Import {
        dir: PathBuf,
        /// Turn the folder names above each file into hashtags.
        #[arg(long)]
        tags_from_folders: bool,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    match run(Cli::parse()).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    let cfg = Config::load(cli.config.as_deref())?;
    match cli.command {
        Command::Serve => serve(cfg).await.map(|_| ExitCode::SUCCESS),
        Command::Doctor => {
            let report = doctor::run(&cfg).await?;
            for (ok, line) in &report.lines {
                println!("{} {line}", if *ok { "✓" } else { "✗" });
            }
            Ok(if report.ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Command::Import { dir, tags_from_folders } => {
            // No crash recovery here: it wipes tmp/ and re-queues running jobs,
            // which would clobber a server running against the same data dir.
            let app = start(cfg, false).await?;
            let workers = worker::spawn(&app, app.cfg.media.concurrency);
            let summary = import::run(&app, &import::Options { dir, tags_from_folders }).await?;
            for w in workers {
                w.abort();
            }
            println!(
                "done: {} added, {} duplicates, {} rejected, {} failed to process",
                summary.added, summary.duplicates, summary.rejected, summary.failed
            );
            Ok(if summary.failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
    }
}

/// Create directories and open the database; with `recover`, also clean up
/// after a previous crash (only the long-running server should do that).
async fn start(cfg: Config, recover: bool) -> Result<App> {
    for dir in [cfg.media_dir(), cfg.tmp_dir(), cfg.work_dir(), cfg.cache_dir.clone()] {
        tokio::fs::create_dir_all(&dir).await.with_context(|| format!("creating {}", dir.display()))?;
    }
    let pool = db::open(&cfg.db_path()).await?;
    let app = AppState::new(cfg, pool);
    if recover {
        worker::recover(&app).await?;
    }
    Ok(app)
}

async fn serve(cfg: Config) -> Result<()> {
    let listen = cfg.listen.clone();
    let app = start(cfg, true).await?;
    worker::spawn(&app, app.cfg.media.concurrency);
    if app.cfg.auth.bootstrap_token.as_deref().unwrap_or("").is_empty() {
        tracing::warn!("auth.bootstrapToken is not set: every /api/v1 request will be refused");
    }

    let listener =
        tokio::net::TcpListener::bind(&listen).await.with_context(|| format!("binding {listen}"))?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, api::router(app))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutting down");
}
