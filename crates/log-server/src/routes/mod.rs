use std::sync::Arc;

use axum::http::{header, HeaderName, HeaderValue, Method};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use chrono::{DateTime, Utc};
use tower_http::cors::{Any, CorsLayer};

use crate::config::Config;
use crate::store::{ColdStore, HotStore};

pub mod auth;
pub mod dashboard;
pub mod dashboard_login;
pub mod dashboard_modal;
pub mod dashboard_url;
pub mod docs;
pub mod health;
pub mod ingest;
pub mod logs;
pub mod nav;
pub mod openapi;

#[derive(Debug, Clone)]
pub struct BootInfo {
    pub started_at: DateTime<Utc>,
    pub git_sha: &'static str,
    pub build_time_unix: u64,
    pub hot_store: String,
    pub cold_store: String,
    pub env_name: String,
    pub port: u16,
    pub s3_bucket: Option<String>,
    pub aws_region: String,
    pub has_ingest_token: bool,
    pub has_ingest_token_public: bool,
    pub has_dashboard_token: bool,
    pub database_url_masked: String,
}

impl BootInfo {
    pub fn uptime_seconds(&self) -> i64 {
        (Utc::now() - self.started_at).num_seconds().max(0)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub hot: Arc<dyn HotStore>,
    pub cold: Arc<dyn ColdStore>,
    /// Named ingest tokens. Source of truth for /ingest auth + attribution.
    pub ingest_tokens: Arc<Vec<auth::TokenRecord>>,
    pub dashboard_token: Option<Arc<String>>,
    pub boot: Arc<BootInfo>,
    /// Non-fatal config warnings collected at boot. Surfaced in the
    /// dashboard settings modal so misconfigured env vars are visible.
    pub config_warnings: Arc<Vec<String>>,
    /// 60-second TTL cache of distinct service/env/event-prefix values.
    /// Eliminates per-render store queries on the dashboard.
    pub distinct_cache: Arc<tokio::sync::Mutex<DistinctCache>>,
}

/// Cached distinct values for the filter datalists. Refreshed on demand
/// when the per-field `last_refresh` is older than 60 seconds.
#[derive(Default, Clone)]
pub struct DistinctCache {
    pub services: Vec<String>,
    pub envs: Vec<String>,
    pub event_prefixes: Vec<String>,
    pub last_refresh: Option<DateTime<Utc>>,
}

/// Cache TTL — 60s is plenty for filter autocomplete; new services/envs
/// don't appear in autocomplete for ≤60s after first event, acceptable.
pub const DISTINCT_CACHE_TTL_SECS: i64 = 60;

pub fn router(
    cfg: &Config,
    hot: Arc<dyn HotStore>,
    cold: Arc<dyn ColdStore>,
    boot: Arc<BootInfo>,
) -> Router {
    let state = AppState {
        hot,
        cold,
        ingest_tokens: Arc::new(cfg.ingest_tokens.clone()),
        dashboard_token: cfg.dashboard_token.as_ref().map(|s| Arc::new(s.clone())),
        boot,
        config_warnings: Arc::new(cfg.warnings.clone()),
        distinct_cache: Arc::new(tokio::sync::Mutex::new(DistinctCache::default())),
    };
    Router::new()
        .route("/", get(dashboard::get_dashboard))
        .route("/health", get(health::get_health))
        .route("/health/full", get(health::get_health_full))
        .route("/ingest", post(ingest::post_ingest))
        .route("/logs", get(logs::get_logs))
        .route("/logs/download.ndjson", get(logs::get_logs_download))
        .route("/api", get(openapi::get_swagger_ui))
        .route("/openapi.yaml", get(openapi::get_openapi_yaml))
        .route("/docs", get(docs::get_docs))
        .route("/favicon.svg", get(get_favicon))
        .route("/favicon.ico", get(get_favicon))
        .route("/assets/crab-logo.svg", get(get_favicon))
        .route("/assets/versable-logo.svg", get(get_versable_logo))
        .route("/assets/versable-wordmark.svg", get(get_wordmark))
        .with_state(state)
        .layer(build_cors(&cfg.cors_origins))
}

fn build_cors(origins: &[String]) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            HeaderName::from_static("authorization"),
            HeaderName::from_static("content-type"),
            HeaderName::from_static("x-request-id"),
        ]);
    if origins.is_empty() {
        // Dev / no allowlist configured — preserve historical permissive behavior.
        layer.allow_origin(Any)
    } else {
        let parsed: Vec<HeaderValue> =
            origins.iter().filter_map(|o| HeaderValue::from_str(o).ok()).collect();
        layer.allow_origin(parsed)
    }
}

const FAVICON_SVG: &str = include_str!("../assets/crab-logo.svg");
const VERSABLE_LOGO_SVG: &str = include_str!("../assets/versable-logo.svg");
const WORDMARK_SVG: &str = include_str!("../assets/versable-wordmark.svg");

async fn get_favicon() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        FAVICON_SVG,
    )
}

async fn get_versable_logo() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        VERSABLE_LOGO_SVG,
    )
}

async fn get_wordmark() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        WORDMARK_SVG,
    )
}

/// Mask a DATABASE_URL for display: strips userinfo in `scheme://user:pass@host`,
/// leaves sqlite file paths as-is.
pub fn mask_database_url(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let (scheme, rest) = url.split_at(scheme_end + 3);
        if let Some(at) = rest.find('@') {
            return format!("{scheme}***:***@{}", &rest[at + 1..]);
        }
    }
    url.to_string()
}
