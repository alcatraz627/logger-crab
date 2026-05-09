use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{auth, AppState};
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
    // /logs API: dashboard cookie OR Bearer (curl-friendly).
    auth::require_dashboard_auth(&headers, state.dashboard_token.as_deref().map(|s| s.as_str()))?;

    let params = build_query_params(q);
    let page = state.hot.query(&params).await?;
    Ok(Json(page))
}

/// `GET /logs/download.ndjson?<filter params>` — streams the matching events
/// as NDJSON (one JSON object per line) with a download Content-Disposition
/// so the browser saves a file. Auth identical to /logs (cookie or Bearer).
///
/// Hard-capped at the underlying HotStore's max query limit (2000 today) so
/// the download is bounded. To export larger sets, paginate by `request_id`
/// or `since`/`until` ranges.
pub async fn get_logs_download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LogsQuery>,
) -> Result<Response, AppError> {
    auth::require_dashboard_auth(&headers, state.dashboard_token.as_deref().map(|s| s.as_str()))?;

    // Build filename hint from the most-specific filter present, falling
    // back to a timestamp. Stays human-readable when shared/saved locally.
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let filename = if let Some(rid) = &q.request_id {
        format!("logger-crab-{}-{stamp}.ndjson", rid_filename_safe(rid))
    } else if let Some(svc) = &q.service {
        format!("logger-crab-{svc}-{stamp}.ndjson")
    } else {
        format!("logger-crab-{stamp}.ndjson")
    };

    let mut params = build_query_params(q);
    // Cap export at 2000 (the underlying store's max). The dashboard's
    // 50-100-row default doesn't apply here — caller wants the filtered set.
    if params.limit < 2000 {
        params.limit = 2000;
    }

    let page = state.hot.query(&params).await?;

    let mut body = String::with_capacity(page.events.len() * 256);
    for event in &page.events {
        if let Ok(json) = serde_json::to_string(event) {
            body.push_str(&json);
            body.push('\n');
        }
    }

    let mut response = body.into_response();
    let headers_mut = response.headers_mut();
    headers_mut.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    if let Ok(disposition) =
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
    {
        headers_mut.insert(header::CONTENT_DISPOSITION, disposition);
    }
    headers_mut.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

fn build_query_params(q: LogsQuery) -> QueryParams {
    QueryParams {
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
    }
}

/// Make a request_id safe for use as a filename component — keep
/// alphanumerics + dash/underscore, collapse anything else to underscore,
/// truncate to a sensible length.
fn rid_filename_safe(rid: &str) -> String {
    let cleaned: String = rid
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    cleaned.chars().take(40).collect()
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
