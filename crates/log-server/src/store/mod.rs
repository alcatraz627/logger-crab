//! Pluggable storage traits. `HotStore` holds the last ~24–48h for fast
//! query. `ColdStore` holds everything else as hourly NDJSON gzip on S3.
//! Selection is env-driven (`HOT_STORE`, `COLD_STORE`). See PLAN.md §4.5.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;

use crate::error::StorageError;
use crate::models::{ColdHealth, HotHealth, IngestSummary, LogEvent, QueryPage, QueryParams};

pub mod memory;
pub mod s3;
pub mod sqlite;

pub type EventStream = Box<dyn Stream<Item = LogEvent> + Send + Unpin>;

#[async_trait]
pub trait HotStore: Send + Sync {
    async fn ingest(&self, events: &[LogEvent]) -> Result<IngestSummary, StorageError>;
    async fn query(&self, params: &QueryParams) -> Result<QueryPage, StorageError>;
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

    async fn read_range(&self, params: &QueryParams) -> Result<EventStream, StorageError>;

    async fn health(&self) -> Result<ColdHealth, StorageError>;
}
