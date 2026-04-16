use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{AppState, auth};
use crate::error::AppError;
use crate::models::{QueryPage, QueryParams};

#[derive(Deserialize, Default)]
pub struct LogsQuery {
    pub request_id: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub service: Option<String>,
    pub env: Option<String>,
    pub event_prefix: Option<String>,
    pub level: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub q: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

pub async fn get_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LogsQuery>,
) -> Result<Json<QueryPage>, AppError> {
    auth::require_bearer(&headers, state.dashboard_token.as_deref().map(|s| s.as_str()))?;

    let params = QueryParams {
        request_id: q.request_id,
        user_id: q.user_id,
        session_id: q.session_id,
        service: q.service,
        env: q.env,
        event_prefix: q.event_prefix,
        min_severity: q.level.as_deref().map(level_to_min_severity),
        since: q.since,
        until: q.until,
        fts: q.q,
        limit: q.limit.unwrap_or(200),
        cursor: q.cursor,
    };

    let page = state.hot.query(&params).await?;
    Ok(Json(page))
}

fn level_to_min_severity(s: &str) -> u8 {
    match s.to_ascii_lowercase().as_str() {
        "trace" => 1,
        "debug" => 5,
        "info" => 9,
        "warn" | "warning" => 13,
        "error" => 17,
        "fatal" => 21,
        _ => 1,
    }
}
