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

        let mut matched: Vec<LogEvent> =
            guard.iter().filter(|e| match_event(e, params)).cloned().collect();

        matched.sort_by_key(|e| std::cmp::Reverse(e.ts));
        matched.truncate(limit);

        Ok(QueryPage { events: matched, next_cursor: None })
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
