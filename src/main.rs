use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use unigif::app::{App, AppState};
use unigif::auth::keys::{self, MintParams};
use unigif::auth::{accounts, Scopes};
use unigif::config::Config;
use unigif::db::{keys as key_db, now_ms, users};
use unigif::ingest::worker;
use unigif::{api, db, doctor, import, janitor, tags};

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
    /// Manage owner accounts.
    #[command(subcommand)]
    User(UserCommand),
    /// Manage API keys.
    #[command(subcommand)]
    Key(KeyCommand),
    /// Bulk-import a folder of GIFs and videos.
    Import {
        dir: PathBuf,
        /// Turn the folder names above each file into hashtags.
        #[arg(long)]
        tags_from_folders: bool,
    },
}

#[derive(Subcommand)]
enum UserCommand {
    /// Create an account. The password is prompted for, read from stdin when
    /// piped, or read from --password-file.
    Add {
        name: String,
        #[arg(long)]
        password_file: Option<PathBuf>,
    },
    /// Set a new password (and log the user out everywhere).
    Passwd {
        name: String,
        #[arg(long)]
        password_file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum KeyCommand {
    /// Mint a key and print it once on stdout.
    Create {
        #[arg(long)]
        name: String,
        /// Scopes, space or comma separated: read upload tag delete admin.
        #[arg(long, conflicts_with = "preset")]
        scopes: Option<String>,
        /// read_only, uploader, picker or full.
        #[arg(long)]
        preset: Option<String>,
        /// Hashtags whose GIFs the key can never see.
        #[arg(long, value_delimiter = ',')]
        deny_tags: Vec<String>,
        /// Requests per minute.
        #[arg(long)]
        rate_limit: Option<i64>,
        #[arg(long)]
        expires_days: Option<u32>,
    },
    /// List keys.
    List,
    /// Revoke a key by id.
    Revoke { id: String },
}

/// A password from a file, a prompt, or a pipe.
fn read_password(file: Option<&std::path::Path>, confirm: bool) -> Result<String> {
    use std::io::{IsTerminal, Read};
    if let Some(f) = file {
        let raw = std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
        return Ok(raw.lines().next().unwrap_or("").to_string());
    }
    if std::io::stdin().is_terminal() {
        let pw = rpassword::prompt_password("Password: ")?;
        if confirm && rpassword::prompt_password("Repeat password: ")? != pw {
            anyhow::bail!("the passwords do not match");
        }
        return Ok(pw);
    }
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    Ok(raw.lines().next().unwrap_or("").to_string())
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
        Command::User(cmd) => {
            let app = start(cfg, false).await?;
            match cmd {
                UserCommand::Add { name, password_file } => {
                    let pw = read_password(password_file.as_deref(), true)?;
                    accounts::create_user(&app.db, &name, &pw).await?;
                    println!("created user {name}");
                }
                UserCommand::Passwd { name, password_file } => {
                    let pw = read_password(password_file.as_deref(), true)?;
                    accounts::set_password(&app.db, &name, &pw).await?;
                    println!("password updated for {name}; all sessions were ended");
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Key(cmd) => {
            let app = start(cfg, false).await?;
            match cmd {
                KeyCommand::Create { name, scopes, preset, deny_tags, rate_limit, expires_days } => {
                    let scopes = match (scopes, preset) {
                        (Some(s), None) => Scopes::parse_list(&s).map_err(anyhow::Error::msg)?,
                        (None, Some(p)) => Scopes::preset(&p)
                            .context("preset must be read_only, uploader, picker or full")?,
                        _ => anyhow::bail!("give --scopes or --preset"),
                    };
                    let owner = users::first(&app.db)
                        .await?
                        .context("no user exists yet; run `unigif user add <name>` first")?;
                    let (row, token) = keys::mint(
                        &app.db,
                        MintParams {
                            name: keys::validate_name(&name).map_err(anyhow::Error::msg)?,
                            scopes,
                            deny_tags: tags::normalize_all(&deny_tags)?,
                            rate_limit: keys::validate_rate_limit(rate_limit).map_err(anyhow::Error::msg)?,
                            expires_at: expires_days.map(|d| now_ms() + d as i64 * 86_400_000),
                            created_by: owner.id,
                        },
                    )
                    .await?;
                    eprintln!("created key {} ({}) with scopes: {}", row.id, row.name, row.scopes);
                    eprintln!("This is the only time the key is shown:");
                    println!("{token}");
                }
                KeyCommand::List => {
                    for k in key_db::list(&app.db, false).await? {
                        println!("{}  {:<24} {}", k.id, k.name, k.scopes);
                    }
                }
                KeyCommand::Revoke { id } => {
                    if key_db::revoke(&app.db, &id, now_ms()).await? {
                        println!("revoked {id}");
                    } else {
                        anyhow::bail!("no live key with id {id}");
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
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
    accounts::ensure_initial_owner(&cfg, &pool).await?;
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
    janitor::spawn(&app);
    if users::count(&app.db).await? == 0 {
        tracing::warn!("no user exists yet: run `unigif user add <name>` (or set auth.initialOwner) to be able to log in");
    }

    let listener =
        tokio::net::TcpListener::bind(&listen).await.with_context(|| format!("binding {listen}"))?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, api::router(app).into_make_service_with_connect_info::<std::net::SocketAddr>())
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
