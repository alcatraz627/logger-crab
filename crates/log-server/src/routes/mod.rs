use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use crate::config::Config;
use crate::store::{ColdStore, HotStore};

pub mod auth;
pub mod dashboard;
pub mod health;
pub mod ingest;
pub mod logs;

#[derive(Clone)]
pub struct AppState {
    pub hot: Arc<dyn HotStore>,
    pub cold: Arc<dyn ColdStore>,
    pub ingest_token: Option<Arc<String>>,
    pub dashboard_token: Option<Arc<String>>,
}

pub fn router(cfg: &Config, hot: Arc<dyn HotStore>, cold: Arc<dyn ColdStore>) -> Router {
    let state = AppState {
        hot,
        cold,
        ingest_token: cfg.ingest_token.as_ref().map(|s| Arc::new(s.clone())),
        dashboard_token: cfg.dashboard_token.as_ref().map(|s| Arc::new(s.clone())),
    };
    Router::new()
        .route("/", get(dashboard::get_dashboard))
        .route("/health", get(health::get_health))
        .route("/ingest", post(ingest::post_ingest))
        .route("/logs", get(logs::get_logs))
        .with_state(state)
}
