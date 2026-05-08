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
use aws_sdk_s3::primitives::ByteStream;
use chrono::{DateTime, Duration, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::stream;
use tokio::sync::Mutex;

use super::{ColdStore, EventStream};
use crate::error::StorageError;
use crate::models::{ColdHealth, LogEvent, QueryParams};

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
            last_error: None,
            events_archived_total: 0,
            last_health_check: Some(Utc::now()),
        })
    }
}

// ─── S3ColdStore ──────────────────────────────────────────────────────────

#[derive(Default)]
struct S3State {
    last_rotation: Option<DateTime<Utc>>,
    last_error: Option<String>,
    events_archived_total: u64,
    last_health_check: Option<DateTime<Utc>>,
    last_health_ok: bool,
}

pub struct S3ColdStore {
    client: aws_sdk_s3::Client,
    bucket: String,
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
            state: Arc::new(Mutex::new(S3State::default())),
        };

        // Boot probe — best effort.
        match store.probe_bucket().await {
            Ok(()) => tracing::info!(bucket = %bucket, region = %region, "S3 cold store reachable"),
            Err(e) => tracing::error!(
                bucket = %bucket,
                region = %region,
                error = %e,
                "S3 cold store unreachable at boot — service will run but archives will fail until resolved"
            ),
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
                // Don't clear last_error — keep the most recent write error
                // visible until the next successful write replaces it.
                Ok(())
            }
            Err(e) => {
                let msg = format!("head_bucket failed: {}", display_sdk_err(&e));
                let mut s = self.state.lock().await;
                s.last_health_check = Some(now);
                s.last_health_ok = false;
                s.last_error = Some(msg.clone());
                Err(StorageError::Unavailable(msg))
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

        let mut s = self.state.lock().await;
        match put {
            Ok(_) => {
                s.last_rotation = Some(Utc::now());
                s.events_archived_total = s
                    .events_archived_total
                    .saturating_add(events.len() as u64);
                s.last_error = None;
                Ok(format!("s3://{}/{}", self.bucket, key))
            }
            Err(e) => {
                let msg = format!("put_object failed: {}", display_sdk_err(&e));
                s.last_error = Some(msg.clone());
                s.last_health_ok = false;
                Err(StorageError::Unavailable(msg))
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
            last_error: s.last_error.clone(),
            events_archived_total: s.events_archived_total,
            last_health_check: s.last_health_check,
        })
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

/// Format an aws-sdk error into a single line suitable for logs / health.
fn display_sdk_err<E: std::fmt::Display>(e: &E) -> String {
    let s = format!("{e}");
    // Collapse multi-line SDK errors to a single line for log readability.
    s.lines().collect::<Vec<_>>().join(" | ")
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
