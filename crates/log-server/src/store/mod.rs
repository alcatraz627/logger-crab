//! Pluggable storage traits. `HotStore` holds the last ~24–48h for fast
//! query. `ColdStore` holds everything else as hourly NDJSON gzip on S3.
//! Selection is env-driven (`HOT_STORE`, `COLD_STORE`). See PLAN.md §4.5.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;

use crate::error::StorageError;
use crate::models::{
    ColdHealth, HotHealth, IngestSummary, LogEvent, QueryPage, QueryParams,
};

pub mod memory;
pub mod s3;
pub mod sqlite;

pub type EventStream = Box<dyn Stream<Item = LogEvent> + Send + Unpin>;

#[async_trait]
pub trait HotStore: Send + Sync {
    async fn ingest(&self, events: &[LogEvent]) -> Result<IngestSummary, StorageError>;
    async fn query(&self, params: &QueryParams) -> Result<QueryPage, StorageError>;
    /// Count of events matching `params` (no limit). Powers the dashboard's
    /// "X matching · Y in store" counter so a filter shows the real total.
    async fn count(&self, params: &QueryParams) -> Result<u64, StorageError>;
    /// Distinct values for a given indexed field across the whole hot store.
    /// Powers the filter datalists so autocomplete suggests every known
    /// service/env, not just those on the current page. `field` accepts
    /// `"service"`, `"env"`, or `"event_prefix"` — anything else returns
    /// an empty list (no error) so the dashboard renders gracefully if the
    /// caller misspells.
    async fn distinct_values(
        &self,
        field: &str,
        limit: u32,
    ) -> Result<Vec<String>, StorageError>;
    async fn drain_older_than(&self, before: DateTime<Utc>) -> Result<EventStream, StorageError>;
    async fn health(&self) -> Result<HotHealth, StorageError>;
}

#[async_trait]
pub trait ColdStore: Send + Sync {
    async fn write_batch(
        &self,
        env: &str,
        service: &str,
        hour: DateTime<Utc>,
        events: &[LogEvent],
    ) -> Result<String, StorageError>;

    /// Cold-tier read with cursor pagination support. Returns a `QueryPage`
    /// (matches `HotStore::query`'s shape) so callers can paginate uniformly
    /// across tiers. Cursor format is RFC3339 timestamp of the last event.
    async fn read_range(&self, params: &QueryParams) -> Result<QueryPage, StorageError>;

    async fn health(&self) -> Result<ColdHealth, StorageError>;
}
