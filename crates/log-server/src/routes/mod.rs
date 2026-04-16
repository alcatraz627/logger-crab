use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::store::{ColdStore, HotStore};

pub mod auth;
pub mod dashboard;
pub mod docs;
pub mod health;
pub mod ingest;
pub mod logs;
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
    pub ingest_token: Option<Arc<String>>,
    pub dashboard_token: Option<Arc<String>>,
    pub boot: Arc<BootInfo>,
}

pub fn router(
    cfg: &Config,
    hot: Arc<dyn HotStore>,
    cold: Arc<dyn ColdStore>,
    boot: Arc<BootInfo>,
) -> Router {
    let state = AppState {
        hot,
        cold,
        ingest_token: cfg.ingest_token.as_ref().map(|s| Arc::new(s.clone())),
        dashboard_token: cfg.dashboard_token.as_ref().map(|s| Arc::new(s.clone())),
        boot,
    };
    Router::new()
        .route("/", get(dashboard::get_dashboard))
        .route("/health", get(health::get_health))
        .route("/ingest", post(ingest::post_ingest))
        .route("/logs", get(logs::get_logs))
        .route("/api", get(openapi::get_swagger_ui))
        .route("/openapi.yaml", get(openapi::get_openapi_yaml))
        .route("/docs", get(docs::get_docs))
        .with_state(state)
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
