//! Hot → cold rotation cron. Periodically archives events older than
//! `HOT_RETENTION_HOURS` from the hot tier to the cold tier as hourly
//! NDJSON.gz objects, then deletes them from hot.
//!
//! Strategy (read-without-delete → write → delete-on-success):
//!   1. Query hot for events with `ts < cutoff` (paginated).
//!   2. Group by `(env, service, hour-bucket)` so each group maps cleanly
//!      to one S3 key (`{env}/{service}/{YYYY}/{MM}/{DD}/{HH}.ndjson.gz`).
//!   3. Write each group to cold via `ColdStore::write_batch`.
//!   4. If ALL groups wrote successfully, call `drain_older_than(cutoff)`
//!      to atomically delete from hot. The returned stream is discarded —
//!      we already have the events in our local buffer; we only need the
//!      DELETE side of drain.
//!   5. If any write failed, skip the delete. The events remain in hot
//!      and the next cycle retries.
//!
//! Race window: between step 1 and step 4, an emitter with backdated
//! timestamps (clock skew, replays) could write events with `ts < cutoff`
//! that get deleted in step 4 without being archived. Acceptable at V1
//! since hot retention is 48h — events with that-old timestamps are
//! near-impossible in practice.
//!
//! The task is best-effort: failures are logged at ERROR but don't crash
//! the service. Operators monitor via `ColdHealth.last_rotation` and
//! `ColdHealth.last_issue` (both surfaced in `/health` and the dashboard
//! footer).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use tokio::task::JoinHandle;

use crate::error::StorageError;
use crate::models::{LogEvent, QueryParams};
use crate::store::{ColdStore, HotStore};

/// Tunables. Defaults match production V1; override via env vars in
/// `Config::from_env`.
#[derive(Debug, Clone)]
pub struct RotationConfig {
    pub interval_secs: u64,
    pub hot_retention_hours: i64,
    pub batch_size: u32,
}

impl Default for RotationConfig {
    fn default() -> Self {
        Self {
            interval_secs: 3600,
            hot_retention_hours: 48,
            batch_size: 5000,
        }
    }
}

/// Stats from one rotation cycle. Logged at INFO on success.
#[derive(Debug, Default)]
pub struct CycleStats {
    pub archived: u64,
    pub groups: u64,
    pub failed_groups: u64,
}

/// Spawn the rotation task. Returns the JoinHandle so the caller can
/// abort on shutdown if needed; in practice we let it run for the
/// lifetime of the process.
///
/// First tick fires after `interval_secs / 2` to give ingest time to
/// settle after boot. After that, `interval_secs` between ticks.
pub fn spawn(
    hot: Arc<dyn HotStore>,
    cold: Arc<dyn ColdStore>,
    cfg: RotationConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let initial_delay = Duration::from_secs(cfg.interval_secs / 2);
        tracing::info!(
            interval_secs = cfg.interval_secs,
            hot_retention_hours = cfg.hot_retention_hours,
            initial_delay_secs = initial_delay.as_secs(),
            "rotation task spawned"
        );
        tokio::time::sleep(initial_delay).await;

        let mut interval = tokio::time::interval(Duration::from_secs(cfg.interval_secs));
        loop {
            interval.tick().await;
            match rotate_once(hot.as_ref(), cold.as_ref(), &cfg).await {
                Ok(stats) if stats.archived == 0 => {
                    tracing::debug!("rotation cycle: nothing to archive");
                }
                Ok(stats) => tracing::info!(
                    archived = stats.archived,
                    groups = stats.groups,
                    failed_groups = stats.failed_groups,
                    "rotation cycle complete"
                ),
                Err(e) => tracing::error!(error = %e, "rotation cycle failed"),
            }
        }
    })
}

/// One pass of the rotation algorithm. Public for testing.
pub async fn rotate_once(
    hot: &dyn HotStore,
    cold: &dyn ColdStore,
    cfg: &RotationConfig,
) -> Result<CycleStats, StorageError> {
    let cutoff = Utc::now() - chrono::Duration::hours(cfg.hot_retention_hours);

    // Step 1: read all events older than cutoff into memory.
    let to_archive = read_events_until(hot, cutoff, cfg.batch_size).await?;
    if to_archive.is_empty() {
        return Ok(CycleStats::default());
    }

    let total = to_archive.len() as u64;

    // Step 2: bucket by (env, service, hour).
    let groups = group_by_hour(to_archive);

    // Step 3: write each group; abort on first failure (next tick will retry).
    let mut archived = 0u64;
    let mut failed_groups = 0u64;
    for ((env, service, hour), events) in &groups {
        match cold.write_batch(env, service, *hour, events).await {
            Ok(key) => {
                tracing::debug!(
                    env = %env,
                    service = %service,
                    hour = %hour,
                    count = events.len(),
                    key = %key,
                    "rotation: archived group"
                );
                archived += events.len() as u64;
            }
            Err(e) => {
                tracing::error!(
                    env = %env,
                    service = %service,
                    hour = %hour,
                    count = events.len(),
                    error = %e,
                    "rotation: write_batch failed; skipping cycle delete"
                );
                failed_groups += 1;
            }
        }
    }

    // Step 4: only delete from hot if every group archived successfully.
    if failed_groups == 0 && archived == total {
        // drain_older_than atomically SELECT+DELETE; we discard the stream
        // since we already have the events in `to_archive`.
        let _stream = hot.drain_older_than(cutoff).await?;
        drop(_stream);
    } else {
        tracing::warn!(
            archived,
            total,
            failed_groups,
            "rotation: partial failure — leaving events in hot for next cycle"
        );
    }

    Ok(CycleStats {
        archived,
        groups: groups.len() as u64,
        failed_groups,
    })
}

async fn read_events_until(
    hot: &dyn HotStore,
    cutoff: DateTime<Utc>,
    batch_size: u32,
) -> Result<Vec<LogEvent>, StorageError> {
    let mut out: Vec<LogEvent> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = hot
            .query(&QueryParams {
                until: Some(cutoff),
                limit: batch_size,
                cursor: cursor.clone(),
                ..Default::default()
            })
            .await?;
        if page.events.is_empty() {
            break;
        }
        out.extend(page.events);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    Ok(out)
}

/// Groups events by `(env, service, hour-bucket)`. Events with missing
/// `env` or `service` get bucketed under "unknown" so they're still archived
/// rather than orphaned.
pub(crate) fn group_by_hour(
    events: Vec<LogEvent>,
) -> HashMap<(String, String, DateTime<Utc>), Vec<LogEvent>> {
    let mut groups: HashMap<(String, String, DateTime<Utc>), Vec<LogEvent>> = HashMap::new();
    for event in events {
        let env = event.env.clone().unwrap_or_else(|| "unknown".into());
        let service = event.service.clone().unwrap_or_else(|| "unknown".into());
        let hour = truncate_to_hour(event.ts);
        groups.entry((env, service, hour)).or_default().push(event);
    }
    groups
}

/// Round a timestamp down to the start of its hour (UTC).
pub(crate) fn truncate_to_hour(ts: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(ts.year(), ts.month(), ts.day(), ts.hour(), 0, 0)
        .single()
        .unwrap_or(ts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ColdHealth, HotHealth, IngestSummary, QueryPage};
    use crate::store::EventStream;
    use async_trait::async_trait;
    use futures::stream;
    use serde_json::json;
    use std::sync::Mutex;

    fn ev(rid: &str, ts: DateTime<Utc>, env: Option<&str>, service: Option<&str>) -> LogEvent {
        LogEvent {
            request_id: rid.into(),
            event: "x".into(),
            severity_number: 9,
            severity_text: "info".into(),
            ts,
            message: None,
            service: service.map(String::from),
            env: env.map(String::from),
            user_id: None,
            session_id: None,
            client_id: None,
            payload: json!({}),
        }
    }

    #[test]
    fn truncate_rounds_down_to_hour() {
        let ts = Utc.with_ymd_and_hms(2026, 5, 9, 14, 37, 42).unwrap();
        let truncated = truncate_to_hour(ts);
        assert_eq!(truncated, Utc.with_ymd_and_hms(2026, 5, 9, 14, 0, 0).unwrap());
    }

    #[test]
    fn truncate_preserves_already_aligned_hour() {
        let ts = Utc.with_ymd_and_hms(2026, 5, 9, 14, 0, 0).unwrap();
        assert_eq!(truncate_to_hour(ts), ts);
    }

    #[test]
    fn group_by_hour_buckets_correctly() {
        let h14 = Utc.with_ymd_and_hms(2026, 5, 9, 14, 30, 0).unwrap();
        let h14b = Utc.with_ymd_and_hms(2026, 5, 9, 14, 59, 59).unwrap();
        let h15 = Utc.with_ymd_and_hms(2026, 5, 9, 15, 0, 0).unwrap();
        let events = vec![
            ev("a", h14, Some("prod"), Some("api")),
            ev("b", h14b, Some("prod"), Some("api")),  // same bucket as `a`
            ev("c", h15, Some("prod"), Some("api")),   // next-hour bucket
            ev("d", h14, Some("prod"), Some("worker")), // different service
            ev("e", h14, Some("staging"), Some("api")), // different env
        ];
        let groups = group_by_hour(events);
        assert_eq!(groups.len(), 4, "expected 4 distinct buckets, got {}", groups.len());

        let key = ("prod".into(), "api".into(), Utc.with_ymd_and_hms(2026, 5, 9, 14, 0, 0).unwrap());
        assert_eq!(groups.get(&key).expect("prod/api/14:00").len(), 2);
    }

    #[test]
    fn group_by_hour_uses_unknown_for_missing_fields() {
        let ts = Utc.with_ymd_and_hms(2026, 5, 9, 14, 0, 0).unwrap();
        let events = vec![ev("a", ts, None, None)];
        let groups = group_by_hour(events);
        let key = ("unknown".into(), "unknown".into(), ts);
        assert!(groups.contains_key(&key));
    }

    // ─── Integration: rotate_once with stub stores ────────────────────

    struct StubHot {
        events: Mutex<Vec<LogEvent>>,
        deleted_calls: Mutex<u32>,
    }
    impl StubHot {
        fn new(events: Vec<LogEvent>) -> Self {
            Self { events: Mutex::new(events), deleted_calls: Mutex::new(0) }
        }
    }
    #[async_trait]
    impl HotStore for StubHot {
        async fn ingest(&self, _: &[LogEvent]) -> Result<IngestSummary, StorageError> {
            unimplemented!()
        }
        async fn count(&self, _: &QueryParams) -> Result<u64, StorageError> { unimplemented!() }
        async fn distinct_values(&self, _: &str, _: u32) -> Result<Vec<String>, StorageError> {
            Ok(Vec::new())
        }
        async fn query(&self, params: &QueryParams) -> Result<QueryPage, StorageError> {
            let g = self.events.lock().unwrap();
            let cutoff = params.until.unwrap_or(Utc::now());
            let matched: Vec<LogEvent> =
                g.iter().filter(|e| e.ts < cutoff).cloned().collect();
            Ok(QueryPage { events: matched, next_cursor: None })
        }
        async fn drain_older_than(&self, before: DateTime<Utc>) -> Result<EventStream, StorageError> {
            let mut g = self.events.lock().unwrap();
            let (drain, keep): (Vec<_>, Vec<_>) = g.drain(..).partition(|e| e.ts < before);
            *g = keep;
            *self.deleted_calls.lock().unwrap() += 1;
            Ok(Box::new(stream::iter(drain)))
        }
        async fn health(&self) -> Result<HotHealth, StorageError> {
            unimplemented!()
        }
    }

    struct StubCold {
        writes: Mutex<Vec<(String, String, DateTime<Utc>, usize)>>,
        fail_after: Option<usize>,
    }
    impl StubCold {
        fn ok() -> Self { Self { writes: Mutex::new(Vec::new()), fail_after: None } }
        fn fail_after(n: usize) -> Self {
            Self { writes: Mutex::new(Vec::new()), fail_after: Some(n) }
        }
    }
    #[async_trait]
    impl ColdStore for StubCold {
        // Track whether read_range was ever called (used in stub implementations
        // for any future test asserting cold queries didn't fire during rotation).
        async fn write_batch(
            &self,
            env: &str,
            service: &str,
            hour: DateTime<Utc>,
            events: &[LogEvent],
        ) -> Result<String, StorageError> {
            let mut w = self.writes.lock().unwrap();
            if let Some(n) = self.fail_after {
                if w.len() >= n {
                    return Err(StorageError::Unavailable("simulated failure".into()));
                }
            }
            w.push((env.into(), service.into(), hour, events.len()));
            Ok(format!("stub://{env}/{service}"))
        }
        async fn read_range(&self, _: &QueryParams) -> Result<QueryPage, StorageError> {
            Ok(QueryPage::default())
        }
        async fn health(&self) -> Result<ColdHealth, StorageError> {
            Ok(ColdHealth {
                ok: true,
                backend: "stub".into(),
                bucket: None,
                last_rotation: None,
                last_issue: None,
                events_archived_total: 0,
                last_health_check: None,
            })
        }
    }

    #[tokio::test]
    async fn rotate_once_archives_old_events_and_deletes() {
        let now = Utc::now();
        let old = now - chrono::Duration::hours(72);
        let young = now - chrono::Duration::hours(1);
        let hot = StubHot::new(vec![
            ev("old1", old, Some("prod"), Some("api")),
            ev("old2", old, Some("prod"), Some("api")),
            ev("young", young, Some("prod"), Some("api")),
        ]);
        let cold = StubCold::ok();
        let cfg = RotationConfig::default();

        let stats = rotate_once(&hot, &cold, &cfg).await.expect("rotate ok");
        assert_eq!(stats.archived, 2);
        assert_eq!(stats.groups, 1);
        assert_eq!(stats.failed_groups, 0);

        // Hot should have only the young event left.
        assert_eq!(hot.events.lock().unwrap().len(), 1);
        // drain_older_than was called exactly once (the post-success delete).
        assert_eq!(*hot.deleted_calls.lock().unwrap(), 1);
        // Cold got one write of 2 events.
        let writes = cold.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].3, 2);
    }

    #[tokio::test]
    async fn rotate_once_with_no_old_events_is_noop() {
        let young = Utc::now() - chrono::Duration::hours(1);
        let hot = StubHot::new(vec![ev("y1", young, Some("p"), Some("s"))]);
        let cold = StubCold::ok();
        let cfg = RotationConfig::default();

        let stats = rotate_once(&hot, &cold, &cfg).await.expect("rotate ok");
        assert_eq!(stats.archived, 0);
        assert_eq!(stats.groups, 0);
        assert_eq!(*hot.deleted_calls.lock().unwrap(), 0, "drain not called when nothing to archive");
        assert!(cold.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn rotate_once_skips_delete_on_cold_failure() {
        let old = Utc::now() - chrono::Duration::hours(72);
        let hot = StubHot::new(vec![
            ev("a", old, Some("prod"), Some("api")),
            ev("b", old, Some("prod"), Some("worker")), // different group
        ]);
        // Fail on the second write_batch call.
        let cold = StubCold::fail_after(1);
        let cfg = RotationConfig::default();

        let stats = rotate_once(&hot, &cold, &cfg).await.expect("rotate ok");
        assert_eq!(stats.archived, 1);
        assert_eq!(stats.failed_groups, 1);
        // Both events remain in hot — no delete on partial failure.
        assert_eq!(hot.events.lock().unwrap().len(), 2);
        assert_eq!(*hot.deleted_calls.lock().unwrap(), 0);
    }
}
