//! Wire format for events + the `*Health` shapes returned by `/health`.
//! `LogEvent` is the canonical record stored in the hot tier and archived
//! to the cold tier as NDJSON. Schema evolution rules: add fields freely,
//! never rename, never remove without a deprecation window.

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
    /// Most recent failure, classified into a structured report.
    /// Cleared on next successful write_batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_issue: Option<S3IssueReport>,
    /// Cumulative count of events successfully archived since process start.
    #[serde(default)]
    pub events_archived_total: u64,
    /// Timestamp of the most recent backend reachability check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_health_check: Option<DateTime<Utc>>,
}

/// Classified S3 failure with self-service remediation hint.
///
/// `kind` is a stable enum-string for filtering/alerting; `summary` is the
/// one-line human description for logs and the dashboard; `action` (when
/// present) tells the operator what to fix.
#[derive(Debug, Clone, Serialize)]
pub struct S3IssueReport {
    /// Stable identifier — one of the variants in [`S3IssueKind`]. Stringified
    /// so it serializes naturally to JSON without serde rename ceremony.
    pub kind: String,
    /// Single-line human description for logs / dashboard footer.
    pub summary: String,
    /// Operator-facing remediation hint. Absent for failure modes where the
    /// fix isn't a single config change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// HTTP status returned by S3 (when the failure was a service response).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// AWS error code — `NoSuchBucket`, `InvalidAccessKeyId`, etc. Often
    /// absent on HEAD requests where the response body is empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_code: Option<String>,
    /// AWS request ID (`x-amz-request-id`) — useful for AWS support tickets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_request_id: Option<String>,
}

/// Stable identifier for the kind of S3 failure observed. Stringified into
/// `S3IssueReport.kind` so the JSON shape stays plain; the enum is the
/// in-process source of truth and what `classify_*` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S3IssueKind {
    /// HTTP 301 — bucket is in a different region than configured.
    WrongRegion,
    /// HTTP 404 — bucket doesn't exist (or wrong account).
    BucketNotFound,
    /// HTTP 403 with InvalidAccessKeyId / SignatureDoesNotMatch — bad creds.
    AuthFailure,
    /// HTTP 403 with AccessDenied — IAM policy missing required action.
    AccessDenied,
    /// Network/DNS/connection failure — request never got a response.
    NetworkFailure,
    /// Request timed out before response.
    TimeoutError,
    /// Other service-level error (unusual status code or unrecognized AWS code).
    ServiceError,
    /// SDK-internal error (request construction, response parsing).
    SdkInternal,
}

impl S3IssueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WrongRegion => "WrongRegion",
            Self::BucketNotFound => "BucketNotFound",
            Self::AuthFailure => "AuthFailure",
            Self::AccessDenied => "AccessDenied",
            Self::NetworkFailure => "NetworkFailure",
            Self::TimeoutError => "TimeoutError",
            Self::ServiceError => "ServiceError",
            Self::SdkInternal => "SdkInternal",
        }
    }
}
