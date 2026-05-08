use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use chrono::Utc;
use log_server::config::Config;
use log_server::rotation::{self, RotationConfig};
use log_server::routes;
use log_server::routes::{mask_database_url, BootInfo};
use log_server::seed::seed_dummy_events;
use log_server::store::memory::MemoryHotStore;
use log_server::store::s3::{NoopColdStore, S3ColdStore};
use log_server::store::sqlite::SqliteHotStore;
use log_server::store::{ColdStore, HotStore};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Best-effort .env load. Picks up `.env` from the current working
    // directory when running locally via `cargo run`. In production
    // (Render, Docker), there's no .env file so this is a silent no-op
    // and env vars come from the platform runtime.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let cfg = Config::from_env().context("loading config")?;
    info!(?cfg, "starting logger-crab");

    // Surface non-fatal config warnings (deprecated vars, malformed token entries).
    for w in &cfg.warnings {
        warn!("{w}");
    }

    if cfg.ingest_tokens.is_empty() {
        warn!("no ingest tokens configured (INGEST_TOKEN_<NAME>=<tier>:<token>) — /ingest is unauthenticated (dev only)");
    } else {
        let full = cfg
            .ingest_tokens
            .iter()
            .filter(|r| matches!(r.tier, log_server::routes::auth::AuthRole::Full))
            .count();
        let public = cfg
            .ingest_tokens
            .iter()
            .filter(|r| matches!(r.tier, log_server::routes::auth::AuthRole::Public))
            .count();
        let consumers: Vec<&str> = cfg.ingest_tokens.iter().map(|r| r.name.as_str()).collect();
        info!(full, public, consumers = ?consumers, "ingest auth tokens configured");
    }
    if cfg.cors_origins.is_empty() {
        warn!("CORS_ORIGINS not set — allowing any origin (dev only; tighten for prod)");
    }
    if cfg.dashboard_token.is_none() {
        warn!("DASHBOARD_TOKEN not set — /logs is unauthenticated (dev only)");
    }

    let hot: Arc<dyn HotStore> = match cfg.hot_store.as_str() {
        "memory" => Arc::new(MemoryHotStore::new()),
        "sqlite" => Arc::new(
            SqliteHotStore::connect(&cfg.database_url).await.context("SqliteHotStore::connect")?,
        ),
        other => anyhow::bail!("unknown HOT_STORE: {other}"),
    };

    let cold: Arc<dyn ColdStore> = match cfg.cold_store.as_str() {
        "noop" => Arc::new(NoopColdStore),
        "s3" => {
            let bucket = cfg.s3_bucket.clone().ok_or_else(|| {
                anyhow::anyhow!("COLD_STORE=s3 requires S3_LOGS_BUCKET to be set")
            })?;
            Arc::new(S3ColdStore::connect(bucket, cfg.aws_region.clone()).await?)
        }
        other => anyhow::bail!("unknown COLD_STORE: {other}"),
    };

    if std::env::var("SEED_ON_BOOT").ok().as_deref() == Some("1") {
        match hot.health().await {
            Ok(h) if h.rows == 0 => {
                if let Err(e) = seed_dummy_events(&hot).await {
                    warn!(error = %e, "SEED_ON_BOOT=1 failed");
                }
            }
            Ok(h) => info!(rows = h.rows, "SEED_ON_BOOT=1 skipped — hot store already has rows"),
            Err(e) => warn!(error = %e, "SEED_ON_BOOT=1 health check failed"),
        }
    }

    let boot = Arc::new(BootInfo {
        started_at: Utc::now(),
        git_sha: env!("BUILD_GIT_SHA"),
        build_time_unix: env!("BUILD_TIME_UNIX").parse().unwrap_or(0),
        hot_store: cfg.hot_store.clone(),
        cold_store: cfg.cold_store.clone(),
        env_name: std::env::var("APP_ENV").unwrap_or_else(|_| "dev".into()),
        port: cfg.port,
        s3_bucket: cfg.s3_bucket.clone(),
        aws_region: cfg.aws_region.clone(),
        has_ingest_token: cfg.ingest_tokens.iter().any(|r| matches!(r.tier, log_server::routes::auth::AuthRole::Full)),
        has_ingest_token_public: cfg.ingest_tokens.iter().any(|r| matches!(r.tier, log_server::routes::auth::AuthRole::Public)),
        has_dashboard_token: cfg.dashboard_token.is_some(),
        database_url_masked: mask_database_url(&cfg.database_url),
    });

    // Spawn hot → cold rotation cron when cold is real and rotation isn't disabled.
    // No-op when COLD_STORE=noop (rotation would just delete events).
    if cfg.cold_store == "s3" && cfg.rotation_enabled {
        let rcfg = RotationConfig {
            interval_secs: cfg.rotation_interval_secs,
            hot_retention_hours: cfg.hot_retention_hours,
            batch_size: cfg.rotation_batch_size,
        };
        rotation::spawn(hot.clone(), cold.clone(), rcfg);
    } else {
        info!(
            cold_store = %cfg.cold_store,
            rotation_enabled = cfg.rotation_enabled,
            "rotation task NOT spawned"
        );
    }

    let app = routes::router(&cfg, hot, cold, boot);
    let addr: SocketAddr = ([0, 0, 0, 0], cfg.port).into();
    let listener = TcpListener::bind(addr).await.context("binding tcp listener")?;
    info!(%addr, "listening");
    axum::serve(listener, app).await.context("axum serve")?;
    Ok(())
}
