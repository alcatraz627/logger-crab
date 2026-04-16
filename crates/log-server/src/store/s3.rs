//! S3 ColdStore — Phase 5 impl. Phase 0 ships a `NoopColdStore` so the
//! binary can boot without AWS credentials.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream;

use super::{ColdStore, EventStream};
use crate::error::StorageError;
use crate::models::{ColdHealth, LogEvent, QueryParams};

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
        tracing::warn!(count = events.len(), "ColdStore=noop: dropping batch (Phase 0)");
        Ok("noop://discarded".into())
    }

    async fn read_range(&self, _params: &QueryParams) -> Result<EventStream, StorageError> {
        Ok(Box::new(stream::empty()))
    }

    async fn health(&self) -> Result<ColdHealth, StorageError> {
        Ok(ColdHealth { ok: true, last_rotation: None })
    }
}

pub struct S3ColdStore {
    // TODO Phase 5: aws_sdk_s3::Client + bucket + key prefix layout
}
