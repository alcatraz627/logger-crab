//! Wire format for events. Kept minimal at Phase 0 — typed envelope
//! subgroups (actor/object/state/system/deploy/source/trace) fill in at
//! Phase 2 before the ingest path lands. See PLAN.md §4.1.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    pub request_id: String,
    pub event: String,
    pub severity_number: u8,
    pub severity_text: String,
    pub ts: DateTime<Utc>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,

    // V1.5 identity additions — see identity-hierarchy.md
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,

    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Default)]
pub struct QueryParams {
    pub request_id: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub service: Option<String>,
    pub env: Option<String>,
    pub event_prefix: Option<String>,
    pub min_severity: Option<u8>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub fts: Option<String>,
    pub limit: u32,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct IngestSummary {
    pub accepted: u32,
    pub rejected: u32,
    pub dropped: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct QueryPage {
    pub events: Vec<LogEvent>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HotHealth {
    pub ok: bool,
    pub rows: u64,
    pub oldest_ts: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ColdHealth {
    pub ok: bool,
    /// Backend in use: "noop" or "s3". Surfaces in /health and dashboard.
    pub backend: String,
    /// Bucket name (S3 only). None for noop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    /// Most recent successful write to cold tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_rotation: Option<DateTime<Utc>>,
    /// Last error from a write or health probe — cleared on next success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Cumulative count of events successfully archived since process start.
    #[serde(default)]
    pub events_archived_total: u64,
    /// Timestamp of the most recent backend reachability check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_health_check: Option<DateTime<Utc>>,
}
