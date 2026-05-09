//! Cold tier — NDJSON.gz on S3. See `docs/STORAGE.md` for the architecture.
//!
//! Two implementations:
//!   - `NoopColdStore`: drops all writes, used when COLD_STORE=noop. The hot
//!     tier (SQLite) becomes the only durable store. Useful for dev /
//!     low-stakes deploys.
//!   - `S3ColdStore`: writes hourly NDJSON.gz objects keyed by
//!     `{env}/{service}/{YYYY}/{MM}/{DD}/{HH}.ndjson.gz`. Used when
//!     COLD_STORE=s3 and `S3_LOGS_BUCKET` is set.
//!
//! Health policy: `S3ColdStore::connect()` does a boot-time `head_bucket` call
//! and logs the result, but **does NOT hard-fail** if S3 is unreachable — the
//! service still boots so the hot tier keeps working. `health()` reports the
//! true backend status (cached for ~30s to avoid hammering S3 on every probe).
//! Operators monitor /health and the dashboard footer to detect cold-tier outages.
//!
//! Read API (`read_range`) is V2 — currently returns an empty stream. Cold-tier
//! query is a separate, harder problem (LIST + GET + decode + filter); not
//! implemented in this revision.

use std::io::Write;
use std::sync::Arc;

use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_s3::primitives::ByteStream;
use chrono::{DateTime, Duration, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::stream;
use tokio::sync::Mutex;

use super::{ColdStore, EventStream};
use crate::error::StorageError;
use crate::models::{ColdHealth, LogEvent, QueryParams, S3IssueKind, S3IssueReport};

/// How long a successful health probe is trusted before re-checking S3.
/// Keeps `/health` cheap when polled frequently.
const HEALTH_CACHE_TTL_SECS: i64 = 30;

// ─── NoopColdStore ────────────────────────────────────────────────────────

pub struct NoopColdStore;

#[async_trait]
impl ColdStore for NoopColdStore {
    async fn write_batch(
        &self,
        _env: &str,
        _service: &str,
        _hour: DateTime<Utc>,
        events: &[LogEvent],
    ) -> Result<String, StorageError> {
        tracing::warn!(count = events.len(), "ColdStore=noop: dropping batch");
        Ok("noop://discarded".into())
    }

    async fn read_range(&self, _params: &QueryParams) -> Result<EventStream, StorageError> {
        Ok(Box::new(stream::empty()))
    }

    async fn health(&self) -> Result<ColdHealth, StorageError> {
        Ok(ColdHealth {
            ok: true,
            backend: "noop".into(),
            bucket: None,
            last_rotation: None,
            last_issue: None,
            events_archived_total: 0,
            last_health_check: Some(Utc::now()),
        })
    }
}

// ─── S3ColdStore ──────────────────────────────────────────────────────────

#[derive(Default)]
struct S3State {
    last_rotation: Option<DateTime<Utc>>,
    last_issue: Option<S3IssueReport>,
    events_archived_total: u64,
    last_health_check: Option<DateTime<Utc>>,
    last_health_ok: bool,
}

pub struct S3ColdStore {
    client: aws_sdk_s3::Client,
    bucket: String,
    region: String,
    state: Arc<Mutex<S3State>>,
}

impl S3ColdStore {
    /// Construct + boot-time reachability check.
    ///
    /// Failure of `head_bucket` does not error — we log loudly and proceed so
    /// the hot tier still serves. `health()` will keep reporting ok=false
    /// until the next successful probe.
    pub async fn connect(bucket: String, region: String) -> anyhow::Result<Self> {
        let region_provider = aws_config::Region::new(region.clone());
        let aws_cfg = aws_config::defaults(BehaviorVersion::latest())
            .region(region_provider)
            .load()
            .await;
        let client = aws_sdk_s3::Client::new(&aws_cfg);

        let store = Self {
            client,
            bucket: bucket.clone(),
            region: region.clone(),
            state: Arc::new(Mutex::new(S3State::default())),
        };

        // Boot probe — best effort. Logs the structured issue when it fails
        // so an operator scanning Render's log feed sees what to fix without
        // needing to hit /health or open the dashboard.
        match store.probe_bucket().await {
            Ok(()) => tracing::info!(
                bucket = %bucket,
                region = %region,
                "S3 cold store reachable"
            ),
            Err(_) => {
                let s = store.state.lock().await;
                if let Some(issue) = &s.last_issue {
                    tracing::error!(
                        bucket = %bucket,
                        configured_region = %region,
                        kind = %issue.kind,
                        status = ?issue.status,
                        aws_code = ?issue.aws_code,
                        aws_request_id = ?issue.aws_request_id,
                        action = ?issue.action,
                        "S3 cold store unreachable at boot: {}",
                        issue.summary
                    );
                }
            }
        }

        Ok(store)
    }

    /// Single-shot reachability check. Updates internal health state.
    async fn probe_bucket(&self) -> Result<(), StorageError> {
        let now = Utc::now();
        match self.client.head_bucket().bucket(&self.bucket).send().await {
            Ok(_) => {
                let mut s = self.state.lock().await;
                s.last_health_check = Some(now);
                s.last_health_ok = true;
                // Don't clear last_issue — keep the most recent write error
                // visible until the next successful write replaces it.
                Ok(())
            }
            Err(e) => {
                let issue = classify_sdk_err(&e, &self.region, &self.bucket);
                let summary = issue.summary.clone();
                let mut s = self.state.lock().await;
                s.last_health_check = Some(now);
                s.last_health_ok = false;
                s.last_issue = Some(issue);
                Err(StorageError::Unavailable(summary))
            }
        }
    }
}

#[async_trait]
impl ColdStore for S3ColdStore {
    async fn write_batch(
        &self,
        env: &str,
        service: &str,
        hour: DateTime<Utc>,
        events: &[LogEvent],
    ) -> Result<String, StorageError> {
        if events.is_empty() {
            return Ok(format!("s3://{}/<empty>", self.bucket));
        }

        let body = encode_ndjson_gz(events)
            .map_err(|e| StorageError::Other(anyhow::anyhow!("encode ndjson.gz: {e}")))?;
        let key = build_key(env, service, hour);

        let put = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(ByteStream::from(body))
            .content_type("application/x-ndjson")
            .content_encoding("gzip")
            .send()
            .await;

        match put {
            Ok(_) => {
                let mut s = self.state.lock().await;
                s.last_rotation = Some(Utc::now());
                s.events_archived_total = s
                    .events_archived_total
                    .saturating_add(events.len() as u64);
                s.last_issue = None;
                Ok(format!("s3://{}/{}", self.bucket, key))
            }
            Err(e) => {
                let issue = classify_sdk_err(&e, &self.region, &self.bucket);
                tracing::error!(
                    bucket = %self.bucket,
                    kind = %issue.kind,
                    status = ?issue.status,
                    aws_code = ?issue.aws_code,
                    "S3 put_object failed: {}",
                    issue.summary
                );
                let summary = issue.summary.clone();
                let mut s = self.state.lock().await;
                s.last_issue = Some(issue);
                s.last_health_ok = false;
                Err(StorageError::Unavailable(summary))
            }
        }
    }

    async fn read_range(&self, _params: &QueryParams) -> Result<EventStream, StorageError> {
        // Cold-tier query (LIST + GET + decode + filter) is a separate
        // workstream; for V1 the dashboard reads only from the hot tier.
        Ok(Box::new(stream::empty()))
    }

    async fn health(&self) -> Result<ColdHealth, StorageError> {
        // If we have a recent successful probe, return the cached state.
        // Otherwise re-probe (best effort — a probe failure still yields a
        // ColdHealth with ok=false rather than a hard error).
        let need_probe = {
            let s = self.state.lock().await;
            match s.last_health_check {
                Some(ts) if (Utc::now() - ts) < Duration::seconds(HEALTH_CACHE_TTL_SECS)
                    && s.last_health_ok =>
                {
                    false
                }
                _ => true,
            }
        };

        if need_probe {
            // Don't propagate probe error; encode it into the returned health.
            let _ = self.probe_bucket().await;
        }

        let s = self.state.lock().await;
        Ok(ColdHealth {
            ok: s.last_health_ok,
            backend: "s3".into(),
            bucket: Some(self.bucket.clone()),
            last_rotation: s.last_rotation,
            last_issue: s.last_issue.clone(),
            events_archived_total: s.events_archived_total,
            last_health_check: s.last_health_check,
        })
    }
}

// ─── Error classification ────────────────────────────────────────────────

/// Classify an `aws_sdk_s3::error::SdkError` into a structured `S3IssueReport`
/// that surfaces what's actually wrong + how to fix it.
///
/// Generic over the operation error type (`HeadBucketError`, `PutObjectError`,
/// `GetObjectError`, …) so the same logic applies to every S3 call. The
/// `R` parameter is the SDK's HTTP response type; we read status + headers
/// off it via the public `.raw()` accessor on `ServiceError`.
fn classify_sdk_err<E, R>(
    err: &SdkError<E, R>,
    configured_region: &str,
    bucket: &str,
) -> S3IssueReport
where
    E: std::fmt::Debug + std::fmt::Display + ProvideErrorMetadata,
    R: ResponseInfo,
{
    match err {
        SdkError::ServiceError(svc) => {
            let raw = svc.raw();
            let status = raw.status_u16();
            let aws_request_id = raw
                .get_header("x-amz-request-id")
                .map(|s| s.to_string());
            let region_header = raw
                .get_header("x-amz-bucket-region")
                .map(|s| s.to_string());

            let meta = ProvideErrorMetadata::meta(svc.err());
            let aws_code = meta.code().map(|s| s.to_string());
            let aws_msg = meta.message().unwrap_or("").to_string();

            // ── 301: bucket is in a different region ───────────────────────
            if status == 301 || matches!(region_header.as_deref(), Some(r) if r != configured_region) {
                if let Some(actual) = region_header {
                    return S3IssueReport {
                        kind: S3IssueKind::WrongRegion.as_str().into(),
                        summary: format!(
                            "Bucket '{bucket}' is in region '{actual}' but AWS_REGION is '{configured_region}'"
                        ),
                        action: Some(format!(
                            "Set AWS_REGION='{actual}' in your env config and restart"
                        )),
                        status: Some(status),
                        aws_code,
                        aws_request_id,
                    };
                }
            }

            // ── 403: auth or IAM ───────────────────────────────────────────
            if status == 403 {
                let kind = match aws_code.as_deref() {
                    Some("InvalidAccessKeyId") | Some("SignatureDoesNotMatch") => {
                        S3IssueKind::AuthFailure
                    }
                    _ => S3IssueKind::AccessDenied,
                };
                let action = match kind {
                    S3IssueKind::AuthFailure => Some(
                        "Verify AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY are correct and not deactivated".to_string(),
                    ),
                    _ => Some(format!(
                        "Verify the IAM user attached to AWS_ACCESS_KEY_ID has s3:ListBucket on arn:aws:s3:::{bucket} and s3:PutObject + s3:GetObject on arn:aws:s3:::{bucket}/*"
                    )),
                };
                return S3IssueReport {
                    kind: kind.as_str().into(),
                    summary: format!(
                        "S3 returned 403 Forbidden ({})",
                        aws_code.as_deref().unwrap_or(if aws_msg.is_empty() { "AccessDenied" } else { aws_msg.as_str() })
                    ),
                    action,
                    status: Some(status),
                    aws_code,
                    aws_request_id,
                };
            }

            // ── 404: bucket missing ────────────────────────────────────────
            if status == 404 {
                return S3IssueReport {
                    kind: S3IssueKind::BucketNotFound.as_str().into(),
                    summary: format!(
                        "Bucket '{bucket}' not found in region '{configured_region}'"
                    ),
                    action: Some(format!(
                        "Verify S3_LOGS_BUCKET name is correct, or create with: aws s3 mb s3://{bucket} --region {configured_region}"
                    )),
                    status: Some(status),
                    aws_code,
                    aws_request_id,
                };
            }

            // ── Other service-level errors ─────────────────────────────────
            S3IssueReport {
                kind: S3IssueKind::ServiceError.as_str().into(),
                summary: if !aws_msg.is_empty() {
                    format!("S3 returned {status} ({aws_msg})")
                } else if let Some(code) = &aws_code {
                    format!("S3 returned {status} ({code})")
                } else {
                    format!("S3 returned status {status}")
                },
                action: None,
                status: Some(status),
                aws_code,
                aws_request_id,
            }
        }
        SdkError::DispatchFailure(d) => S3IssueReport {
            kind: S3IssueKind::NetworkFailure.as_str().into(),
            summary: format!("Network/DNS failure dispatching to S3: {d:?}"),
            action: Some(
                "Check outbound network connectivity to *.amazonaws.com and DNS resolution".into(),
            ),
            status: None,
            aws_code: None,
            aws_request_id: None,
        },
        SdkError::TimeoutError(_) => S3IssueReport {
            kind: S3IssueKind::TimeoutError.as_str().into(),
            summary: "Request to S3 timed out before response".into(),
            action: Some("Check network latency or AWS service health".into()),
            status: None,
            aws_code: None,
            aws_request_id: None,
        },
        SdkError::ResponseError(_) | SdkError::ConstructionFailure(_) => S3IssueReport {
            kind: S3IssueKind::SdkInternal.as_str().into(),
            summary: format!("SDK internal error: {err}"),
            action: None,
            status: None,
            aws_code: None,
            aws_request_id: None,
        },
        _ => S3IssueReport {
            kind: S3IssueKind::ServiceError.as_str().into(),
            summary: format!("{err}"),
            action: None,
            status: None,
            aws_code: None,
            aws_request_id: None,
        },
    }
}

/// Tiny adapter over the SDK's HTTP response type so `classify_sdk_err`
/// stays generic without depending on aws-smithy-runtime-api directly.
/// Both `aws_smithy_runtime_api::http::Response` (the SDK's response type)
/// and `aws_smithy_runtime_api::client::orchestrator::HttpResponse` get a
/// matching impl below.
trait ResponseInfo {
    fn status_u16(&self) -> u16;
    fn get_header(&self, name: &str) -> Option<&str>;
}

impl ResponseInfo for aws_smithy_runtime_api::http::Response {
    fn status_u16(&self) -> u16 {
        self.status().as_u16()
    }
    fn get_header(&self, name: &str) -> Option<&str> {
        self.headers().get(name)
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────

/// `{env}/{service}/{YYYY}/{MM}/{DD}/{HH}.ndjson.gz`
///
/// Layout is load-bearing — S3 lifecycle policies (e.g. transition to
/// Glacier after N days) key off the `YYYY/MM/DD/HH` prefix. Don't change
/// without coordinating bucket-side rules.
fn build_key(env: &str, service: &str, hour: DateTime<Utc>) -> String {
    format!(
        "{env}/{service}/{}/{}.ndjson.gz",
        hour.format("%Y/%m/%d"),
        hour.format("%H")
    )
}

/// Encode a slice of events as NDJSON, then gzip. One newline per event;
/// no trailing newline. Returns the gzipped bytes ready for put_object.
fn encode_ndjson_gz(events: &[LogEvent]) -> std::io::Result<Vec<u8>> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    for (i, event) in events.iter().enumerate() {
        let json = serde_json::to_vec(event).map_err(std::io::Error::other)?;
        gz.write_all(&json)?;
        if i + 1 < events.len() {
            gz.write_all(b"\n")?;
        }
    }
    gz.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;
    use std::io::Read;

    fn ev(rid: &str, ts: DateTime<Utc>) -> LogEvent {
        LogEvent {
            request_id: rid.into(),
            event: "test.event".into(),
            severity_number: 9,
            severity_text: "info".into(),
            ts,
            message: None,
            service: Some("svc".into()),
            env: Some("dev".into()),
            user_id: None,
            session_id: None,
            client_id: None,
            payload: json!({"k": "v"}),
        }
    }

    #[test]
    fn key_layout_matches_spec() {
        let hour = Utc.with_ymd_and_hms(2026, 5, 8, 14, 0, 0).unwrap();
        assert_eq!(
            build_key("prod", "versable-app", hour),
            "prod/versable-app/2026/05/08/14.ndjson.gz"
        );
    }

    #[test]
    fn key_layout_pads_single_digit_components() {
        let hour = Utc.with_ymd_and_hms(2026, 1, 3, 7, 0, 0).unwrap();
        assert_eq!(
            build_key("dev", "svc", hour),
            "dev/svc/2026/01/03/07.ndjson.gz"
        );
    }

    #[test]
    fn ndjson_gz_round_trips() {
        let ts = Utc.with_ymd_and_hms(2026, 5, 8, 14, 0, 0).unwrap();
        let events = vec![ev("r1", ts), ev("r2", ts)];

        let gz = encode_ndjson_gz(&events).expect("encode");

        // Decompress + parse
        let mut decoder = flate2::read::GzDecoder::new(&gz[..]);
        let mut decoded = String::new();
        decoder.read_to_string(&mut decoded).expect("decompress");

        let lines: Vec<&str> = decoded.split('\n').collect();
        assert_eq!(lines.len(), 2, "expected 2 NDJSON lines, got: {decoded}");

        let e1: LogEvent = serde_json::from_str(lines[0]).expect("parse 1");
        let e2: LogEvent = serde_json::from_str(lines[1]).expect("parse 2");
        assert_eq!(e1.request_id, "r1");
        assert_eq!(e2.request_id, "r2");
    }

    #[test]
    fn ndjson_gz_empty_batch_ok() {
        let gz = encode_ndjson_gz(&[]).expect("encode empty");
        // Empty gzip stream is still a valid 20-ish-byte gzip wrapper.
        assert!(!gz.is_empty(), "even empty input produces gzip header bytes");
    }

    #[tokio::test]
    async fn noop_health_reports_noop_backend() {
        let h = NoopColdStore.health().await.expect("ok");
        assert_eq!(h.backend, "noop");
        assert!(h.ok);
        assert!(h.bucket.is_none());
        assert_eq!(h.events_archived_total, 0);
    }
}
