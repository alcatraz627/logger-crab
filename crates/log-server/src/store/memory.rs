//! In-memory HotStore — for tests and `HOT_STORE=memory` dev harness.

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream;

use super::{EventStream, HotStore};
use crate::error::StorageError;
use crate::models::{HotHealth, IngestSummary, LogEvent, QueryPage, QueryParams};

pub struct MemoryHotStore {
    inner: Mutex<Vec<LogEvent>>,
}

impl MemoryHotStore {
    pub fn new() -> Self {
        Self { inner: Mutex::new(Vec::new()) }
    }
}

impl Default for MemoryHotStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HotStore for MemoryHotStore {
    async fn ingest(&self, events: &[LogEvent]) -> Result<IngestSummary, StorageError> {
        let mut guard = self.inner.lock().expect("memory store poisoned");
        guard.extend_from_slice(events);
        Ok(IngestSummary { accepted: events.len() as u32, rejected: 0, dropped: 0 })
    }

    async fn query(&self, params: &QueryParams) -> Result<QueryPage, StorageError> {
        let guard = self.inner.lock().expect("memory store poisoned");
        let limit = if params.limit == 0 { 100 } else { params.limit as usize };

        // Cursor pagination: skip events whose ts >= cursor (matches sqlite's
        // `WHERE ts < cursor` semantics in DESC order).
        let cursor_ts: Option<chrono::DateTime<chrono::Utc>> = params
            .cursor
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|d| d.with_timezone(&chrono::Utc))
            });

        let mut matched: Vec<LogEvent> = guard
            .iter()
            .filter(|e| match_event(e, params))
            .filter(|e| match cursor_ts {
                Some(c) => e.ts < c,
                None => true,
            })
            .cloned()
            .collect();

        matched.sort_by_key(|e| std::cmp::Reverse(e.ts));
        matched.truncate(limit);

        let next_cursor = if matched.len() == limit {
            matched.last().map(|e| e.ts.to_rfc3339())
        } else {
            None
        };

        Ok(QueryPage { events: matched, next_cursor })
    }

    async fn count(&self, params: &QueryParams) -> Result<u64, StorageError> {
        let guard = self.inner.lock().expect("memory store poisoned");
        let n = guard.iter().filter(|e| match_event(e, params)).count();
        Ok(n as u64)
    }

    async fn distinct_values(
        &self,
        field: &str,
        limit: u32,
    ) -> Result<Vec<String>, StorageError> {
        let guard = self.inner.lock().expect("memory store poisoned");
        let mut values: Vec<String> = match field {
            "service" => guard.iter().filter_map(|e| e.service.clone()).collect(),
            "env" => guard.iter().filter_map(|e| e.env.clone()).collect(),
            "event_prefix" => guard
                .iter()
                .map(|e| match e.event.split_once('.') {
                    Some((prefix, _)) => format!("{prefix}."),
                    None => e.event.clone(),
                })
                .collect(),
            _ => return Ok(Vec::new()),
        };
        values.retain(|v| !v.is_empty());
        values.sort();
        values.dedup();
        values.truncate(limit as usize);
        Ok(values)
    }

    async fn drain_older_than(&self, before: DateTime<Utc>) -> Result<EventStream, StorageError> {
        let mut guard = self.inner.lock().expect("memory store poisoned");
        let (drain, keep): (Vec<_>, Vec<_>) = guard.drain(..).partition(|e| e.ts < before);
        *guard = keep;
        Ok(Box::new(stream::iter(drain)))
    }

    async fn health(&self) -> Result<HotHealth, StorageError> {
        let guard = self.inner.lock().expect("memory store poisoned");
        let oldest_ts = guard.iter().map(|e| e.ts).min();
        Ok(HotHealth { ok: true, rows: guard.len() as u64, oldest_ts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use serde_json::json;

    fn ev(rid: &str, ts_offset_secs: i64, svc: Option<&str>, env: Option<&str>) -> LogEvent {
        LogEvent {
            request_id: rid.into(),
            event: "test".into(),
            severity_number: 9,
            severity_text: "info".into(),
            ts: Utc::now() - Duration::seconds(ts_offset_secs),
            message: None,
            service: svc.map(String::from),
            env: env.map(String::from),
            user_id: None,
            session_id: None,
            client_id: None,
            payload: json!({}),
        }
    }

    #[tokio::test]
    async fn cursor_pagination_walks_pages_in_order() {
        let store = MemoryHotStore::new();
        // 5 events, newest to oldest: e0 (now), e1 (-10s), e2 (-20s), e3 (-30s), e4 (-40s)
        let events: Vec<LogEvent> = (0..5).map(|i| ev(&format!("r{i}"), i * 10, None, None)).collect();
        store.ingest(&events).await.unwrap();

        // Page 1: page size 2, no cursor — returns r0, r1
        let p1 = store.query(&QueryParams { limit: 2, ..Default::default() }).await.unwrap();
        assert_eq!(p1.events.len(), 2);
        assert_eq!(p1.events[0].request_id, "r0");
        assert_eq!(p1.events[1].request_id, "r1");
        assert!(p1.next_cursor.is_some(), "full page should yield a cursor");

        // Page 2: use cursor from page 1 — returns r2, r3
        let p2 = store.query(&QueryParams {
            limit: 2,
            cursor: p1.next_cursor.clone(),
            ..Default::default()
        }).await.unwrap();
        assert_eq!(p2.events.len(), 2);
        assert_eq!(p2.events[0].request_id, "r2");
        assert_eq!(p2.events[1].request_id, "r3");
        assert!(p2.next_cursor.is_some(), "still full page");

        // Page 3: cursor from page 2 — returns r4 only (under-full)
        let p3 = store.query(&QueryParams {
            limit: 2,
            cursor: p2.next_cursor.clone(),
            ..Default::default()
        }).await.unwrap();
        assert_eq!(p3.events.len(), 1);
        assert_eq!(p3.events[0].request_id, "r4");
        assert!(p3.next_cursor.is_none(), "under-full page → no more cursor");
    }

    #[tokio::test]
    async fn cursor_pagination_respects_filters() {
        let store = MemoryHotStore::new();
        let events: Vec<LogEvent> = vec![
            ev("r0", 0, Some("api"), None),
            ev("r1", 10, Some("worker"), None),
            ev("r2", 20, Some("api"), None),
            ev("r3", 30, Some("worker"), None),
            ev("r4", 40, Some("api"), None),
        ];
        store.ingest(&events).await.unwrap();

        let p1 = store.query(&QueryParams {
            limit: 2,
            service: Some("api".into()),
            ..Default::default()
        }).await.unwrap();
        assert_eq!(p1.events.len(), 2);
        assert!(p1.events.iter().all(|e| e.service.as_deref() == Some("api")));

        let p2 = store.query(&QueryParams {
            limit: 2,
            service: Some("api".into()),
            cursor: p1.next_cursor.clone(),
            ..Default::default()
        }).await.unwrap();
        assert_eq!(p2.events.len(), 1, "only r4 left in api filter");
        assert_eq!(p2.events[0].request_id, "r4");
        assert!(p2.next_cursor.is_none());
    }

    #[tokio::test]
    async fn count_matches_filter() {
        let store = MemoryHotStore::new();
        let events = vec![
            ev("r0", 0, Some("api"), Some("prod")),
            ev("r1", 10, Some("api"), Some("dev")),
            ev("r2", 20, Some("worker"), Some("prod")),
            ev("r3", 30, Some("api"), Some("prod")),
        ];
        store.ingest(&events).await.unwrap();

        assert_eq!(store.count(&QueryParams::default()).await.unwrap(), 4);
        assert_eq!(
            store.count(&QueryParams { service: Some("api".into()), ..Default::default() }).await.unwrap(),
            3
        );
        assert_eq!(
            store.count(&QueryParams {
                service: Some("api".into()),
                env: Some("prod".into()),
                ..Default::default()
            }).await.unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn distinct_values_dedupes_and_sorts() {
        let store = MemoryHotStore::new();
        let events = vec![
            ev("r0", 0, Some("worker"), None),
            ev("r1", 10, Some("api"), None),
            ev("r2", 20, Some("worker"), None),
            ev("r3", 30, Some("api"), None),
            ev("r4", 40, Some("cron"), None),
        ];
        store.ingest(&events).await.unwrap();

        let services = store.distinct_values("service", 100).await.unwrap();
        assert_eq!(services, vec!["api", "cron", "worker"], "sorted + deduped");
    }

    #[tokio::test]
    async fn distinct_values_unknown_field_returns_empty() {
        let store = MemoryHotStore::new();
        let result = store.distinct_values("not_a_field", 100).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn drain_older_than_removes_only_old_events() {
        let store = MemoryHotStore::new();
        let now = Utc::now();
        let recent = ev("recent", -10, None, None);  // ts = now + 10s
        let old = ev("old", 86400, None, None);      // ts = now - 1d
        store.ingest(&[recent.clone(), old.clone()]).await.unwrap();

        let cutoff = now;
        let _drained = store.drain_older_than(cutoff).await.unwrap();
        // Only recent should remain.
        let h = store.health().await.unwrap();
        assert_eq!(h.rows, 1);
    }

    #[tokio::test]
    async fn count_with_no_filters_matches_total() {
        let store = MemoryHotStore::new();
        let events: Vec<LogEvent> = (0..7)
            .map(|i| ev(&format!("r{i}"), i * 10, Some("api"), Some("dev")))
            .collect();
        store.ingest(&events).await.unwrap();

        assert_eq!(store.count(&QueryParams::default()).await.unwrap(), 7);
    }

    #[tokio::test]
    async fn query_respects_cursor_with_filters() {
        let store = MemoryHotStore::new();
        let events: Vec<LogEvent> = vec![
            ev("e0", 0, Some("api"), None),
            ev("e1", 60, Some("worker"), None), // filtered out
            ev("e2", 120, Some("api"), None),
            ev("e3", 180, Some("api"), None),
        ];
        store.ingest(&events).await.unwrap();

        let p1 = store
            .query(&QueryParams {
                limit: 1,
                service: Some("api".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(p1.events.len(), 1);
        assert_eq!(p1.events[0].request_id, "e0");

        let p2 = store
            .query(&QueryParams {
                limit: 1,
                service: Some("api".into()),
                cursor: p1.next_cursor.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(p2.events.len(), 1);
        assert_eq!(p2.events[0].request_id, "e2"); // skips e1 (worker), goes to next api
    }
}

fn match_event(e: &LogEvent, p: &QueryParams) -> bool {
    if let Some(rid) = &p.request_id {
        if e.request_id != *rid {
            return false;
        }
    }
    if let Some(uid) = &p.user_id {
        if e.user_id.as_deref() != Some(uid.as_str()) {
            return false;
        }
    }
    if let Some(sid) = &p.session_id {
        if e.session_id.as_deref() != Some(sid.as_str()) {
            return false;
        }
    }
    if let Some(svc) = &p.service {
        if e.service.as_deref() != Some(svc.as_str()) {
            return false;
        }
    }
    if let Some(env) = &p.env {
        if e.env.as_deref() != Some(env.as_str()) {
            return false;
        }
    }
    if let Some(prefix) = &p.event_prefix {
        if !e.event.starts_with(prefix) {
            return false;
        }
    }
    if let Some(min) = p.min_severity {
        if e.severity_number < min {
            return false;
        }
    }
    if let Some(since) = p.since {
        if e.ts < since {
            return false;
        }
    }
    if let Some(until) = p.until {
        if e.ts > until {
            return false;
        }
    }
    if let Some(fts) = &p.fts {
        let needle = fts.to_ascii_lowercase();
        let haystack =
            format!("{} {}", e.message.as_deref().unwrap_or(""), e.payload).to_ascii_lowercase();
        if !haystack.contains(&needle) {
            return false;
        }
    }
    true
}
