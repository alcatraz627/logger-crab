//! Dummy-tagged seed data for first-look demos. Every event carries
//! `payload.dummy = true` so prod filters can exclude it cleanly.

use std::sync::Arc;

use chrono::{Duration, Utc};
use serde_json::json;

use crate::models::LogEvent;
use crate::store::HotStore;

pub async fn seed_dummy_events(hot: &Arc<dyn HotStore>) -> anyhow::Result<()> {
    let now = Utc::now();
    let base = now - Duration::minutes(30);

    let mut events: Vec<LogEvent> = Vec::new();

    events.push(make(
        base,
        0,
        "req_alice_01",
        "versable-app",
        "prod",
        "ui.page.view",
        9,
        "User viewed /enhance/123",
        json!({"dummy": true, "path": "/enhance/123", "user_id": "u_alice"}),
    ));
    events.push(make(
        base,
        150,
        "req_alice_01",
        "versable-api",
        "prod",
        "http.request",
        9,
        "POST /api/jobs 201",
        json!({"dummy": true, "method": "POST", "status": 201, "latency_ms": 128}),
    ));
    events.push(make(
        base,
        400,
        "req_alice_01",
        "versable-api",
        "prod",
        "redis.enqueue",
        5,
        "enqueued job_id=j_9001",
        json!({"dummy": true, "queue": "credits", "job_id": "j_9001"}),
    ));
    events.push(make(
        base,
        820,
        "req_alice_01",
        "credit-worker",
        "prod",
        "pipeline.start",
        9,
        "picked up job_id=j_9001",
        json!({"dummy": true, "job_id": "j_9001", "attempt": 1}),
    ));
    events.push(make(
        base,
        3400,
        "req_alice_01",
        "credit-worker",
        "prod",
        "pipeline.done",
        9,
        "completed job_id=j_9001 in 3.4s",
        json!({"dummy": true, "job_id": "j_9001", "duration_ms": 3400, "rows": 42}),
    ));

    events.push(make(
        base,
        5000,
        "req_bob_02",
        "versable-app",
        "prod",
        "ui.upload.start",
        9,
        "Bob started upload 12MB CSV",
        json!({"dummy": true, "size_bytes": 12_582_912, "user_id": "u_bob"}),
    ));
    events.push(make(
        base,
        6200,
        "req_bob_02",
        "versable-api",
        "prod",
        "http.request",
        13,
        "POST /api/upload 413 payload too large",
        json!({"dummy": true, "status": 413, "reason": "limit_exceeded"}),
    ));
    events.push(make(
        base,
        6210,
        "req_bob_02",
        "versable-app",
        "prod",
        "ui.upload.error",
        13,
        "Upload failed: file too large",
        json!({"dummy": true, "shown_to_user": true}),
    ));

    events.push(make(
        base,
        8000,
        "req_crn_03",
        "cron-daily",
        "prod",
        "cron.rollup.start",
        9,
        "Starting nightly rollup",
        json!({"dummy": true, "job": "rollup.daily"}),
    ));
    events.push(make(
        base,
        9500,
        "req_crn_03",
        "cron-daily",
        "prod",
        "cron.rollup.warn",
        13,
        "Skipping 3 stale rows",
        json!({"dummy": true, "skipped": 3, "reason": "stale"}),
    ));
    events.push(make(
        base,
        11000,
        "req_crn_03",
        "cron-daily",
        "prod",
        "cron.rollup.done",
        9,
        "Rollup OK (1847 rows, 2.9s)",
        json!({"dummy": true, "rows": 1847, "duration_ms": 2912}),
    ));

    events.push(make(
        base,
        14000,
        "req_err_04",
        "credit-worker",
        "prod",
        "openai.call.error",
        17,
        "OpenAI API 429 rate limit",
        json!({
            "dummy": true, "provider": "openai", "status": 429,
            "retry_after_s": 30, "model": "gpt-5"
        }),
    ));
    events.push(make(
        base,
        14010,
        "req_err_04",
        "credit-worker",
        "prod",
        "pipeline.retry",
        13,
        "Retrying after 30s",
        json!({"dummy": true, "attempt": 2, "backoff_ms": 30000}),
    ));

    events.push(make(
        base,
        16000,
        "req_dev_05",
        "versable-api",
        "dev",
        "db.query.slow",
        13,
        "Slow query: 2.1s",
        json!({
            "dummy": true, "duration_ms": 2108,
            "sql_fingerprint": "select_jobs_by_user"
        }),
    ));
    events.push(make(
        base,
        16500,
        "req_dev_06",
        "versable-api",
        "dev",
        "auth.login.ok",
        9,
        "login successful",
        json!({"dummy": true, "user_id": "u_dev", "method": "oauth"}),
    ));

    events.push(make(
        base,
        18000,
        "req_fatal_07",
        "credit-worker",
        "prod",
        "worker.panic",
        21,
        "Worker panicked, exited",
        json!({
            "dummy": true,
            "panic_msg": "index out of bounds: len=0 idx=3",
            "will_restart": true
        }),
    ));

    hot.ingest(&events).await?;
    tracing::info!(count = events.len(), "seeded dummy events");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn make(
    base: chrono::DateTime<Utc>,
    offset_ms: i64,
    request_id: &str,
    service: &str,
    env: &str,
    event: &str,
    severity_number: u8,
    message: &str,
    payload: serde_json::Value,
) -> LogEvent {
    LogEvent {
        request_id: request_id.into(),
        event: event.into(),
        severity_number,
        severity_text: severity_label(severity_number).into(),
        ts: base + Duration::milliseconds(offset_ms),
        message: Some(message.into()),
        service: Some(service.into()),
        env: Some(env.into()),
        user_id: None,
        session_id: None,
        client_id: None,
        payload,
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
