use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{auth, AppState};
use crate::error::AppError;
use crate::models::{QueryPage, QueryParams};
use crate::store::{ColdStore, HotStore};

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
    pub job_id: Option<String>,
    pub task_id: Option<String>,
    pub team_id: Option<String>,
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
    let (page, _) = query_both_tiers(&state, &params).await?;
    Ok(Json(page))
}

/// Answers a query from whichever tier holds the requested time range.
///
/// Recent events live in the hot store; anything rotated out (older than the
/// hot retention window) lives in the cold S3 archive. A `since` older than the
/// hot store's oldest event reads cold, or both tiers when the range straddles
/// the boundary. Returns the page and whether cold was read.
pub(crate) async fn query_tiered(
    hot: &dyn HotStore,
    cold: &dyn ColdStore,
    params: &QueryParams,
    hot_oldest: Option<DateTime<Utc>>,
    cold_ok: bool,
) -> Result<(QueryPage, bool), AppError> {
    let reads_cold = match (params.since, hot_oldest, cold_ok) {
        (Some(since), Some(oldest), true) => since < oldest,
        (Some(_), None, true) => true, // hot is empty, so everything asked for is cold
        _ => false,
    };
    if !reads_cold {
        return Ok((hot.query(params).await?, false));
    }

    let straddles = match (params.until, hot_oldest) {
        (until, Some(oldest)) => until.map(|u| u >= oldest).unwrap_or(true),
        _ => false,
    };
    if !straddles {
        return Ok((cold.read_range(params).await?, true));
    }

    // Rotation writes to cold then deletes from hot, so the boundary has no overlap.
    let oldest = hot_oldest.expect("straddles implies hot_oldest");
    let mut cold_params = params.clone();
    cold_params.until = Some(oldest);
    cold_params.cursor = None;
    let cold_page = cold.read_range(&cold_params).await?;

    let mut hot_params = params.clone();
    hot_params.since = Some(oldest);
    let hot_page = hot.query(&hot_params).await?;

    let mut merged = hot_page.events;
    merged.extend(cold_page.events);
    merged.sort_by_key(|e| std::cmp::Reverse(e.ts));
    merged.truncate(params.limit as usize);
    // Straddle pagination follows the hot (newer) half's cursor.
    Ok((QueryPage { events: merged, next_cursor: hot_page.next_cursor }, true))
}

async fn query_both_tiers(
    state: &AppState,
    params: &QueryParams,
) -> Result<(QueryPage, bool), AppError> {
    let hot_oldest = state.hot.health().await.ok().and_then(|h| h.oldest_ts);
    let cold_ok = state
        .cold
        .health()
        .await
        .ok()
        .map(|c| c.backend == "s3" && c.ok)
        .unwrap_or(false);
    query_tiered(state.hot.as_ref(), state.cold.as_ref(), params, hot_oldest, cold_ok).await
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

    let (page, _) = query_both_tiers(&state, &params).await?;

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
        job_id: q.job_id,
        task_id: q.task_id,
        team_id: q.team_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::StorageError;
    use crate::models::{ColdHealth, LogEvent};
    use crate::store::memory::MemoryHotStore;
    use async_trait::async_trait;
    use chrono::Duration;
    use serde_json::json;

    fn ev(rid: &str, ts: DateTime<Utc>) -> LogEvent {
        LogEvent {
            request_id: rid.into(),
            event: "test".into(),
            severity_number: 9,
            severity_text: "info".into(),
            ts,
            message: None,
            service: None,
            env: None,
            user_id: None,
            session_id: None,
            client_id: None,
            payload: json!({}),
        }
    }

    struct ArchivedCold(Vec<LogEvent>);

    #[async_trait]
    impl ColdStore for ArchivedCold {
        async fn write_batch(
            &self,
            _: &str,
            _: &str,
            _: DateTime<Utc>,
            _: &[LogEvent],
        ) -> Result<String, StorageError> {
            unimplemented!()
        }
        async fn read_range(&self, p: &QueryParams) -> Result<QueryPage, StorageError> {
            let events = self
                .0
                .iter()
                .filter(|e| p.until.map(|u| e.ts < u).unwrap_or(true))
                .cloned()
                .collect();
            Ok(QueryPage { events, next_cursor: None })
        }
        async fn health(&self) -> Result<ColdHealth, StorageError> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn reads_archived_events_older_than_the_hot_store() {
        let now = Utc::now();
        let hot = MemoryHotStore::new();
        hot.ingest(&[ev("recent", now - Duration::hours(1))]).await.unwrap();
        let cold = ArchivedCold(vec![ev("archived", now - Duration::days(5))]);
        let hot_oldest = Some(now - Duration::hours(1));

        let week = QueryParams { since: Some(now - Duration::days(7)), limit: 50, ..Default::default() };
        let (page, read_cold) = query_tiered(&hot, &cold, &week, hot_oldest, true).await.unwrap();
        let ids: Vec<_> = page.events.iter().map(|e| e.request_id.as_str()).collect();
        assert!(read_cold);
        assert_eq!(ids, ["recent", "archived"]);

        let latest = QueryParams { limit: 50, ..Default::default() };
        let (page, read_cold) = query_tiered(&hot, &cold, &latest, hot_oldest, true).await.unwrap();
        assert!(!read_cold);
        assert_eq!(page.events.len(), 1);
    }
}
