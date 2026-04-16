use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{auth, AppState};
use crate::error::AppError;
use crate::models::LogEvent;

#[derive(Deserialize)]
pub struct IngestBody {
    #[serde(default)]
    pub resource: Option<Resource>,
    #[serde(default)]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub events: Vec<RawEvent>,
}

#[derive(Deserialize, Clone)]
pub struct Resource {
    pub service: Option<String>,
    pub env: Option<String>,
    #[serde(default)]
    pub deploy: Option<Value>,
    #[serde(default)]
    pub system: Option<Value>,
}

#[derive(Deserialize, Clone)]
pub struct Scope {
    pub name: Option<String>,
    pub version: Option<String>,
}

#[derive(Deserialize)]
pub struct RawEvent {
    pub request_id: Option<String>,
    pub event: Option<String>,
    pub severity_number: Option<u8>,
    pub severity_text: Option<String>,
    pub ts: Option<DateTime<Utc>>,
    pub message: Option<String>,
    pub service: Option<String>,
    pub env: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub client_id: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Serialize)]
pub struct IngestResponse {
    pub accepted: u32,
    pub rejected: Vec<RejectedEvent>,
}

#[derive(Serialize)]
pub struct RejectedEvent {
    pub index: usize,
    pub reason: String,
}

pub async fn post_ingest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<IngestBody>,
) -> Result<impl IntoResponse, AppError> {
    auth::require_bearer(&headers, state.ingest_token.as_deref().map(|s| s.as_str()))?;

    let resource =
        body.resource.unwrap_or(Resource { service: None, env: None, deploy: None, system: None });

    let mut events = Vec::with_capacity(body.events.len());
    let mut rejected = Vec::new();
    for (i, raw) in body.events.into_iter().enumerate() {
        match build_event(raw, &resource) {
            Ok(e) => events.push(e),
            Err(reason) => rejected.push(RejectedEvent { index: i, reason }),
        }
    }

    let summary = state.hot.ingest(&events).await?;
    tracing::info!(accepted = summary.accepted, rejected = rejected.len(), "ingest");

    let body = IngestResponse { accepted: summary.accepted, rejected };
    Ok((StatusCode::ACCEPTED, Json(body)))
}

fn build_event(raw: RawEvent, res: &Resource) -> Result<LogEvent, String> {
    let request_id = raw.request_id.ok_or("missing request_id")?;
    let event = raw.event.ok_or("missing event name")?;
    let ts = raw.ts.unwrap_or_else(Utc::now);
    let severity_number = raw.severity_number.unwrap_or(9);
    let severity_text = raw.severity_text.unwrap_or_else(|| severity_label(severity_number).into());

    Ok(LogEvent {
        request_id,
        event,
        severity_number,
        severity_text,
        ts,
        message: raw.message,
        service: raw.service.or_else(|| res.service.clone()),
        env: raw.env.or_else(|| res.env.clone()),
        user_id: raw.user_id,
        session_id: raw.session_id,
        client_id: raw.client_id,
        payload: raw.payload,
    })
}

fn severity_label(n: u8) -> &'static str {
    match n {
        1..=4 => "trace",
        5..=8 => "debug",
        9..=12 => "info",
        13..=16 => "warn",
        17..=20 => "error",
        21..=24 => "fatal",
        _ => "info",
    }
}
