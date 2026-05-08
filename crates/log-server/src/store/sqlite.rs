//! SQLite HotStore — Phase 2 impl. Pool + PRAGMAs + boot-time migrate +
//! QueryParams → SQL builder. See PLAN.md §4.2 (schema) and §5.2 (query
//! params).

use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

use super::{EventStream, HotStore};
use crate::error::StorageError;
use crate::models::{HotHealth, IngestSummary, LogEvent, QueryPage, QueryParams};

pub struct SqliteHotStore {
    pool: SqlitePool,
}

impl SqliteHotStore {
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        let opts = SqliteConnectOptions::from_str(database_url)
            .map_err(|e| StorageError::Unavailable(format!("bad DATABASE_URL: {e}")))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new().max_connections(8).connect_with(opts).await?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| StorageError::Schema(format!("migrations failed: {e}")))?;

        Ok(Self { pool })
    }
}

#[async_trait]
impl HotStore for SqliteHotStore {
    async fn ingest(&self, events: &[LogEvent]) -> Result<IngestSummary, StorageError> {
        if events.is_empty() {
            return Ok(IngestSummary::default());
        }

        let mut tx = self.pool.begin().await?;
        let mut accepted = 0u32;
        let mut rejected = 0u32;

        for e in events {
            // Only `event` is required at the storage layer. `request_id`
            // is optional — emitter-stamped when present, empty otherwise.
            if e.event.is_empty() {
                rejected += 1;
                continue;
            }
            let payload = serde_json::to_string(&e.payload).unwrap_or_else(|_| "{}".into());
            let ts = e.ts.to_rfc3339();
            let res = sqlx::query(
                "INSERT INTO events (\
                    request_id, event, severity_number, severity_text, ts, message, \
                    service, env, user_id, session_id, client_id, payload\
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&e.request_id)
            .bind(&e.event)
            .bind(e.severity_number as i64)
            .bind(&e.severity_text)
            .bind(&ts)
            .bind(&e.message)
            .bind(&e.service)
            .bind(&e.env)
            .bind(&e.user_id)
            .bind(&e.session_id)
            .bind(&e.client_id)
            .bind(&payload)
            .execute(&mut *tx)
            .await;

            match res {
                Ok(_) => accepted += 1,
                Err(err) => {
                    tracing::warn!(error = %err, "event insert failed");
                    rejected += 1;
                }
            }
        }

        tx.commit().await?;
        Ok(IngestSummary { accepted, rejected, dropped: 0 })
    }

    async fn query(&self, params: &QueryParams) -> Result<QueryPage, StorageError> {
        let limit = match params.limit {
            0 => 200,
            n if n > 2000 => 2000,
            n => n,
        };

        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT request_id, event, severity_number, severity_text, ts, message, \
             service, env, user_id, session_id, client_id, payload \
             FROM events WHERE 1=1",
        );

        if let Some(rid) = &params.request_id {
            qb.push(" AND request_id = ").push_bind(rid.clone());
        }
        if let Some(uid) = &params.user_id {
            qb.push(" AND user_id = ").push_bind(uid.clone());
        }
        if let Some(sid) = &params.session_id {
            qb.push(" AND session_id = ").push_bind(sid.clone());
        }
        if let Some(svc) = &params.service {
            qb.push(" AND service = ").push_bind(svc.clone());
        }
        if let Some(env) = &params.env {
            qb.push(" AND env = ").push_bind(env.clone());
        }
        if let Some(prefix) = &params.event_prefix {
            let like = format!("{prefix}%");
            qb.push(" AND event LIKE ").push_bind(like);
        }
        if let Some(min) = params.min_severity {
            qb.push(" AND severity_number >= ").push_bind(min as i64);
        }
        if let Some(since) = params.since {
            qb.push(" AND ts >= ").push_bind(since.to_rfc3339());
        }
        if let Some(until) = params.until {
            qb.push(" AND ts <= ").push_bind(until.to_rfc3339());
        }
        if let Some(q) = &params.fts {
            let trimmed = q.trim();
            if !trimmed.is_empty() {
                let phrase = format!("\"{}\"", trimmed.replace('"', "\"\""));
                qb.push(" AND id IN (SELECT rowid FROM events_fts WHERE events_fts MATCH ")
                    .push_bind(phrase)
                    .push(")");
            }
        }

        qb.push(" ORDER BY ts DESC LIMIT ").push_bind(limit as i64);

        let rows = qb.build().fetch_all(&self.pool).await?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            events.push(row_to_event(&row)?);
        }
        Ok(QueryPage { events, next_cursor: None })
    }

    async fn drain_older_than(&self, before: DateTime<Utc>) -> Result<EventStream, StorageError> {
        let before_s = before.to_rfc3339();
        let mut tx = self.pool.begin().await?;

        let rows = sqlx::query(
            "SELECT request_id, event, severity_number, severity_text, ts, message, \
             service, env, user_id, session_id, client_id, payload \
             FROM events WHERE ts < ? ORDER BY ts ASC",
        )
        .bind(&before_s)
        .fetch_all(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM events WHERE ts < ?").bind(&before_s).execute(&mut *tx).await?;

        tx.commit().await?;

        let events: Vec<LogEvent> = rows.iter().filter_map(|r| row_to_event(r).ok()).collect();
        Ok(Box::new(stream::iter(events)))
    }

    async fn health(&self) -> Result<HotHealth, StorageError> {
        let row = sqlx::query("SELECT COUNT(*) AS n, MIN(ts) AS oldest FROM events")
            .fetch_one(&self.pool)
            .await?;
        let rows: i64 = row.try_get("n").unwrap_or(0);
        let oldest: Option<String> = row.try_get("oldest").ok().flatten();
        let oldest_ts = oldest
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        Ok(HotHealth { ok: true, rows: rows as u64, oldest_ts })
    }
}

fn row_to_event(row: &sqlx::sqlite::SqliteRow) -> Result<LogEvent, StorageError> {
    let ts_str: String = row.try_get("ts").map_err(sqlx_err)?;
    let ts = DateTime::parse_from_rfc3339(&ts_str)
        .map_err(|e| StorageError::Schema(format!("bad ts: {e}")))?
        .with_timezone(&Utc);
    let severity_number: i64 = row.try_get("severity_number").map_err(sqlx_err)?;
    let payload_str: String = row.try_get("payload").map_err(sqlx_err)?;
    let payload: serde_json::Value = serde_json::from_str(&payload_str).unwrap_or_default();

    Ok(LogEvent {
        request_id: row.try_get("request_id").map_err(sqlx_err)?,
        event: row.try_get("event").map_err(sqlx_err)?,
        severity_number: severity_number as u8,
        severity_text: row.try_get("severity_text").map_err(sqlx_err)?,
        ts,
        message: row.try_get("message").ok(),
        service: row.try_get("service").ok(),
        env: row.try_get("env").ok(),
        user_id: row.try_get("user_id").ok(),
        session_id: row.try_get("session_id").ok(),
        client_id: row.try_get("client_id").ok(),
        payload,
    })
}

fn sqlx_err(e: sqlx::Error) -> StorageError {
    StorageError::Sqlx(e)
}
