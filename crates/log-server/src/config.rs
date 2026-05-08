use std::env;
use std::sync::Arc;

use crate::routes::auth::{AuthRole, TokenRecord};

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub hot_store: String,
    pub cold_store: String,
    /// Per-consumer named tokens. Source of truth for /ingest auth.
    /// Built from env vars matching `INGEST_TOKEN_<NAME>=<tier>:<token>`.
    pub ingest_tokens: Vec<TokenRecord>,
    pub dashboard_token: Option<String>,
    pub s3_bucket: Option<String>,
    pub aws_region: String,
    /// Allowed CORS origins. Empty = allow any (dev default).
    pub cors_origins: Vec<String>,
    /// Hot → cold rotation cron settings.
    pub rotation_enabled: bool,
    pub rotation_interval_secs: u64,
    pub hot_retention_hours: i64,
    pub rotation_batch_size: u32,
    /// Non-fatal config warnings — surfaced at boot after tracing init.
    /// Includes malformed `INGEST_TOKEN_*` values and deprecated legacy vars.
    pub warnings: Vec<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let mut warnings = Vec::new();
        let ingest_tokens = discover_named_tokens(env::vars(), &mut warnings);
        check_deprecated_vars(&mut warnings);

        Ok(Self {
            port: env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8089),
            database_url: env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".into()),
            hot_store: env::var("HOT_STORE").unwrap_or_else(|_| "memory".into()),
            cold_store: env::var("COLD_STORE").unwrap_or_else(|_| "noop".into()),
            ingest_tokens,
            dashboard_token: env::var("DASHBOARD_TOKEN").ok(),
            s3_bucket: env::var("S3_LOGS_BUCKET").ok(),
            aws_region: env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".into()),
            cors_origins: parse_csv("CORS_ORIGINS"),
            rotation_enabled: env::var("ROTATION_ENABLED")
                .ok()
                .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                .unwrap_or(true),
            rotation_interval_secs: env::var("ROTATION_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),
            hot_retention_hours: env::var("HOT_RETENTION_HOURS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(48),
            rotation_batch_size: env::var("ROTATION_BATCH_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5000),
            warnings,
        })
    }
}

/// Scans an iterator of env-var (key, value) pairs for `INGEST_TOKEN_<NAME>=<tier>:<token>`.
///
/// Naming: env var suffix is `UPPER_SNAKE_CASE`; consumer name in the dashboard is
/// the suffix lowercased with `_` replaced by `-`. So `INGEST_TOKEN_PROD_APP_SERVER`
/// → consumer `prod-app-server`.
///
/// Malformed values (missing tier prefix, unknown tier, empty token, empty suffix)
/// are skipped with a warning collected into `warnings`.
///
/// Decoupled from `std::env::vars()` so tests can pass synthetic input without
/// mutating process env (which is not thread-safe under cargo's parallel test runner).
pub(crate) fn discover_named_tokens<I>(iter: I, warnings: &mut Vec<String>) -> Vec<TokenRecord>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut out = Vec::new();
    for (key, value) in iter {
        let Some(suffix) = key.strip_prefix("INGEST_TOKEN_") else {
            continue;
        };
        if suffix.is_empty() {
            warnings.push(format!("env var '{key}' has empty consumer suffix; skipping"));
            continue;
        }
        let Some((tier_raw, token)) = value.split_once(':') else {
            warnings.push(format!(
                "env var '{key}' value missing tier prefix (expected 'full:...' or 'public:...'); skipping"
            ));
            continue;
        };
        let tier = match tier_raw {
            "full" => AuthRole::Full,
            "public" => AuthRole::Public,
            other => {
                warnings.push(format!(
                    "env var '{key}' has unknown tier '{other}' (expected 'full' or 'public'); skipping"
                ));
                continue;
            }
        };
        if token.trim().is_empty() {
            warnings.push(format!("env var '{key}' has empty token after tier prefix; skipping"));
            continue;
        }
        let name = suffix.to_lowercase().replace('_', "-");
        out.push(TokenRecord {
            name,
            tier,
            token: Arc::new(token.to_string()),
            source_env_var: key.clone(),
        });
    }
    out
}

/// Surfaces legacy / removed env-var names so an operator who left them in
/// place after migration sees a clear startup warning instead of silent
/// "auth turned off".
fn check_deprecated_vars(warnings: &mut Vec<String>) {
    if env::var("INGEST_TOKEN").is_ok() {
        warnings.push(
            "DEPRECATED: env var 'INGEST_TOKEN' is set but no longer honored — use INGEST_TOKEN_<NAME>=<tier>:<token> per consumer"
                .into(),
        );
    }
    if env::var("INGEST_TOKENS").is_ok() {
        warnings.push(
            "DEPRECATED: env var 'INGEST_TOKENS' is set but no longer honored — use INGEST_TOKEN_<NAME>=<tier>:<token> per consumer"
                .into(),
        );
    }
    // INGEST_TOKEN_PUBLIC overlaps with the new prefix. Only warn if the value
    // doesn't look like the new tier-prefixed form (i.e. it's the legacy bare token).
    if let Ok(value) = env::var("INGEST_TOKEN_PUBLIC") {
        if !(value.starts_with("full:") || value.starts_with("public:")) {
            warnings.push(
                "DEPRECATED: env var 'INGEST_TOKEN_PUBLIC' looks like the legacy bare-token form — change to INGEST_TOKEN_PUBLIC=public:<token> or rename the consumer".into(),
            );
        }
    }
}

fn parse_csv(name: &str) -> Vec<String> {
    env::var(name)
        .ok()
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn discovers_named_tokens_with_tier_prefix() {
        let mut warns = Vec::new();
        let recs = discover_named_tokens(
            vars(&[
                ("INGEST_TOKEN_PROD_APP_SERVER", "full:tok1"),
                ("INGEST_TOKEN_DEV_BROWSER", "public:tok2"),
                ("UNRELATED_VAR", "ignored"),
                ("DATABASE_URL", "sqlite::memory:"),
            ]),
            &mut warns,
        );
        assert_eq!(recs.len(), 2);
        assert!(warns.is_empty(), "no warnings expected, got {warns:?}");

        let prod = recs.iter().find(|r| r.name == "prod-app-server").expect("prod");
        assert!(matches!(prod.tier, AuthRole::Full));
        assert_eq!(prod.token.as_str(), "tok1");
        assert_eq!(prod.source_env_var, "INGEST_TOKEN_PROD_APP_SERVER");

        let dev = recs.iter().find(|r| r.name == "dev-browser").expect("dev");
        assert!(matches!(dev.tier, AuthRole::Public));
        assert_eq!(dev.token.as_str(), "tok2");
    }

    #[test]
    fn malformed_value_warns_and_skips() {
        let mut warns = Vec::new();
        let recs = discover_named_tokens(
            vars(&[
                ("INGEST_TOKEN_BAD_NO_TIER", "just-a-token"),
                ("INGEST_TOKEN_BAD_TIER", "weird:tok"),
                ("INGEST_TOKEN_EMPTY_TOKEN", "full:"),
                ("INGEST_TOKEN_VALID", "full:goodtok"),
            ]),
            &mut warns,
        );
        assert_eq!(recs.len(), 1, "only the valid one should pass");
        assert_eq!(recs[0].name, "valid");
        assert_eq!(warns.len(), 3, "three warnings expected, got {warns:?}");
    }

    #[test]
    fn empty_suffix_warns() {
        let mut warns = Vec::new();
        let recs = discover_named_tokens(
            vars(&[("INGEST_TOKEN_", "full:tok")]),
            &mut warns,
        );
        assert!(recs.is_empty());
        assert_eq!(warns.len(), 1);
        assert!(warns[0].contains("empty consumer suffix"));
    }

    #[test]
    fn token_value_can_contain_colon() {
        // split_once means token can contain ':' (e.g. base64-with-padding-style)
        let mut warns = Vec::new();
        let recs = discover_named_tokens(
            vars(&[("INGEST_TOKEN_X", "full:abc:def:ghi")]),
            &mut warns,
        );
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].token.as_str(), "abc:def:ghi");
    }

    #[test]
    fn name_is_lowercased_and_dasherized() {
        let mut warns = Vec::new();
        let recs = discover_named_tokens(
            vars(&[("INGEST_TOKEN_PROD_APP_SERVER_V2", "full:t")]),
            &mut warns,
        );
        assert_eq!(recs[0].name, "prod-app-server-v2");
    }

    #[test]
    fn unrelated_env_vars_are_ignored() {
        let mut warns = Vec::new();
        let recs = discover_named_tokens(
            vars(&[
                ("PATH", "/usr/bin"),
                ("DATABASE_URL", "sqlite::memory:"),
                ("INGEST_TOKEN_OK", "full:tok"),
                ("INGESTING_OTHER_THING", "full:nope"),
            ]),
            &mut warns,
        );
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].name, "ok");
        assert!(warns.is_empty());
    }
}
