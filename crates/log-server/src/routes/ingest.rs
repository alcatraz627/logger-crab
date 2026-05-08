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
    let outcome = auth::require_bearer_record(&headers, &state.ingest_tokens)?;
    let (consumer_name, role) = match outcome {
        auth::AuthOutcome::Authenticated(rec) => (rec.name.as_str(), Some(rec.tier)),
        auth::AuthOutcome::Unauthenticated => ("_unauth", None),
    };

    let resource =
        body.resource.unwrap_or(Resource { service: None, env: None, deploy: None, system: None });

    let mut events = Vec::with_capacity(body.events.len());
    let mut rejected = Vec::new();
    for (i, raw) in body.events.into_iter().enumerate() {
        match build_event(raw, &resource, consumer_name) {
            Ok(e) => events.push(e),
            Err(reason) => rejected.push(RejectedEvent { index: i, reason }),
        }
    }

    let summary = state.hot.ingest(&events).await?;
    tracing::info!(
        accepted = summary.accepted,
        rejected = rejected.len(),
        consumer = consumer_name,
        role = ?role,
        "ingest"
    );

    let body = IngestResponse { accepted: summary.accepted, rejected };
    Ok((StatusCode::ACCEPTED, Json(body)))
}

fn build_event(
    raw: RawEvent,
    res: &Resource,
    consumer_name: &str,
) -> Result<LogEvent, String> {
    // request_id is optional — events without a rid are accepted (system
    // events, cron, ad-hoc emissions). Stored as empty string. The dashboard
    // renders "—" for empty rids and the request-trail filter naturally
    // skips them.
    let request_id = raw.request_id.unwrap_or_default();
    let event = raw.event.ok_or("missing event name")?;
    let ts = raw.ts.unwrap_or_else(Utc::now);
    let severity_number = raw.severity_number.unwrap_or(9);
    let severity_text = raw.severity_text.unwrap_or_else(|| severity_label(severity_number).into());

    // Server-stamp the auth_consumer into payload. Emitter-supplied values are
    // overwritten — this is the trustworthy attribution field that survives
    // any service/env spoofing in the rest of the event.
    let payload = stamp_auth_consumer(raw.payload, consumer_name);

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
        payload,
    })
}

fn stamp_auth_consumer(payload: Value, consumer_name: &str) -> Value {
    match payload {
        Value::Object(mut map) => {
            map.insert("_auth_consumer".to_string(), Value::String(consumer_name.to_string()));
            Value::Object(map)
        }
        Value::Null => {
            let mut map = serde_json::Map::new();
            map.insert("_auth_consumer".to_string(), Value::String(consumer_name.to_string()));
            Value::Object(map)
        }
        // Non-object payload (array, scalar) — wrap it under `_value` and stamp alongside.
        // Keeps the stamp queryable without losing the original.
        other => {
            let mut map = serde_json::Map::new();
            map.insert("_auth_consumer".to_string(), Value::String(consumer_name.to_string()));
            map.insert("_value".to_string(), other);
            Value::Object(map)
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stamps_into_object_payload() {
        let stamped = stamp_auth_consumer(json!({"foo": "bar", "n": 42}), "prod-app-server");
        assert_eq!(stamped["foo"], "bar");
        assert_eq!(stamped["n"], 42);
        assert_eq!(stamped["_auth_consumer"], "prod-app-server");
    }

    #[test]
    fn overwrites_emitter_set_value() {
        // Critical security property: emitter cannot fake _auth_consumer.
        let stamped = stamp_auth_consumer(
            json!({"_auth_consumer": "fake-name", "x": 1}),
            "real-name",
        );
        assert_eq!(stamped["_auth_consumer"], "real-name");
        assert_eq!(stamped["x"], 1);
    }

    #[test]
    fn handles_null_payload() {
        let stamped = stamp_auth_consumer(Value::Null, "consumer");
        assert!(stamped.is_object());
        assert_eq!(stamped["_auth_consumer"], "consumer");
    }

    #[test]
    fn wraps_array_payload_under_value_key() {
        let stamped = stamp_auth_consumer(json!([1, 2, 3]), "consumer");
        assert_eq!(stamped["_auth_consumer"], "consumer");
        assert_eq!(stamped["_value"], json!([1, 2, 3]));
    }

    #[test]
    fn wraps_scalar_payload_under_value_key() {
        let stamped = stamp_auth_consumer(json!("hello"), "consumer");
        assert_eq!(stamped["_auth_consumer"], "consumer");
        assert_eq!(stamped["_value"], "hello");
    }

    #[test]
    fn build_event_stamps_consumer_into_payload() {
        let raw = RawEvent {
            request_id: Some("req-1".into()),
            event: Some("test.event".into()),
            severity_number: Some(9),
            severity_text: Some("info".into()),
            ts: None,
            message: None,
            service: None,
            env: None,
            user_id: None,
            session_id: None,
            client_id: None,
            payload: json!({"k": "v"}),
        };
        let res = Resource { service: None, env: None, deploy: None, system: None };
        let event = build_event(raw, &res, "prod-app-server").expect("build_event ok");
        assert_eq!(event.payload["k"], "v");
        assert_eq!(event.payload["_auth_consumer"], "prod-app-server");
    }

    #[test]
    fn build_event_accepts_missing_request_id() {
        let raw = RawEvent {
            request_id: None,
            event: Some("system.boot".into()),
            severity_number: None,
            severity_text: None,
            ts: None,
            message: None,
            service: None,
            env: None,
            user_id: None,
            session_id: None,
            client_id: None,
            payload: Value::Null,
        };
        let res = Resource { service: None, env: None, deploy: None, system: None };
        let event = build_event(raw, &res, "anyone").expect("should accept missing rid");
        assert_eq!(event.request_id, "", "missing rid stored as empty string");
    }

    #[test]
    fn build_event_rejects_missing_event_name() {
        let raw = RawEvent {
            request_id: Some("r".into()),
            event: None,
            severity_number: None,
            severity_text: None,
            ts: None,
            message: None,
            service: None,
            env: None,
            user_id: None,
            session_id: None,
            client_id: None,
            payload: Value::Null,
        };
        let res = Resource { service: None, env: None, deploy: None, system: None };
        let result = build_event(raw, &res, "anyone");
        assert!(result.is_err(), "event name is still required");
    }

    #[test]
    fn severity_label_buckets() {
        assert_eq!(severity_label(1), "trace");
        assert_eq!(severity_label(9), "info");
        assert_eq!(severity_label(13), "warn");
        assert_eq!(severity_label(17), "error");
        assert_eq!(severity_label(21), "fatal");
        assert_eq!(severity_label(99), "info"); // out-of-range falls back
    }
}
