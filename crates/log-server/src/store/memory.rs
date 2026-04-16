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
        let events: Vec<LogEvent> = guard.iter().rev().take(limit).cloned().collect();
        Ok(QueryPage { events, next_cursor: None })
    }

    async fn drain_older_than(
        &self,
        before: DateTime<Utc>,
    ) -> Result<EventStream, StorageError> {
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
