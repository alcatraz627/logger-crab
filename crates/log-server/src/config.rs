use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub hot_store: String,
    pub cold_store: String,
    pub ingest_token: Option<String>,
    pub dashboard_token: Option<String>,
    pub s3_bucket: Option<String>,
    pub aws_region: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            port: env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8080),
            database_url: env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".into()),
            hot_store: env::var("HOT_STORE").unwrap_or_else(|_| "memory".into()),
            cold_store: env::var("COLD_STORE").unwrap_or_else(|_| "noop".into()),
            ingest_token: env::var("INGEST_TOKEN").ok(),
            dashboard_token: env::var("DASHBOARD_TOKEN").ok(),
            s3_bucket: env::var("S3_LOGS_BUCKET").ok(),
            aws_region: env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".into()),
        })
    }
}
