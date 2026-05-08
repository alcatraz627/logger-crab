use std::fmt;
use std::sync::Arc;

use axum::http::HeaderMap;

use crate::error::AppError;

/// Which token tier authenticated the request.
///
/// `Full` = trusted server-side emitter.
/// `Public` = browser-side emitter, treat as untrusted; rate-limit candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthRole {
    Full,
    Public,
}

/// A named ingest token. Each known emitter (prod-app, staging-server,
/// dev-aakarsh, ...) has its own record so leaks isolate to one consumer
/// and dashboard events can be attributed via server-stamped `_auth_consumer`.
///
/// `source_env_var` is the env var name the record came from (e.g.
/// `INGEST_TOKEN_PROD_APP_SERVER`) so the settings modal can flag which
/// row in Render's UI to edit when rotating.
///
/// Custom `Debug` redacts the token so it never lands in boot/audit logs.
#[derive(Clone)]
pub struct TokenRecord {
    pub name: String,
    pub tier: AuthRole,
    pub token: Arc<String>,
    pub source_env_var: String,
}

impl fmt::Debug for TokenRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenRecord")
            .field("name", &self.name)
            .field("tier", &self.tier)
            .field("token", &"***")
            .field("source_env_var", &self.source_env_var)
            .finish()
    }
}

/// Result of an /ingest auth check.
#[derive(Debug, Clone, Copy)]
pub enum AuthOutcome<'a> {
    /// A configured token matched. The caller should stamp `record.name`
    /// onto every accepted event as `_auth_consumer`.
    Authenticated(&'a TokenRecord),
    /// No tokens configured anywhere → dev mode, auth gate disabled.
    /// Caller should stamp `_auth_consumer = "_unauth"`.
    Unauthenticated,
}

/// Single-token bearer check. Used by /logs (DASHBOARD_TOKEN). Returns Ok if no token configured.
pub fn require_bearer(headers: &HeaderMap, expected: Option<&str>) -> Result<(), AppError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let got = extract_bearer(headers)?;
    if constant_time_eq(got.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

/// Multi-token bearer check returning the matching record (for attribution).
///
/// - Empty `tokens` slice → returns `Unauthenticated` (dev mode, no auth gate).
/// - Header missing or no record matches → 401.
/// - All comparisons constant-time within a record.
pub fn require_bearer_record<'a>(
    headers: &HeaderMap,
    tokens: &'a [TokenRecord],
) -> Result<AuthOutcome<'a>, AppError> {
    if tokens.is_empty() {
        return Ok(AuthOutcome::Unauthenticated);
    }
    let got = extract_bearer(headers)?;
    let got_bytes = got.as_bytes();

    for rec in tokens {
        if constant_time_eq(got_bytes, rec.token.as_bytes()) {
            return Ok(AuthOutcome::Authenticated(rec));
        }
    }
    Err(AppError::Unauthorized)
}

fn extract_bearer(headers: &HeaderMap) -> Result<&str, AppError> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn rec(name: &str, tier: AuthRole, tok: &str) -> TokenRecord {
        TokenRecord {
            name: name.into(),
            tier,
            token: Arc::new(tok.into()),
            source_env_var: format!("INGEST_TOKEN_{}", name.to_uppercase().replace('-', "_")),
        }
    }

    fn auth_headers(tok: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("authorization", HeaderValue::from_str(&format!("Bearer {tok}")).unwrap());
        h
    }

    #[test]
    fn empty_tokens_yields_unauthenticated() {
        let outcome = require_bearer_record(&HeaderMap::new(), &[]).expect("ok");
        assert!(matches!(outcome, AuthOutcome::Unauthenticated));
    }

    #[test]
    fn matches_first_record() {
        let recs = vec![rec("a", AuthRole::Full, "tok-a"), rec("b", AuthRole::Public, "tok-b")];
        let outcome = require_bearer_record(&auth_headers("tok-a"), &recs).expect("ok");
        match outcome {
            AuthOutcome::Authenticated(r) => {
                assert_eq!(r.name, "a");
                assert!(matches!(r.tier, AuthRole::Full));
            }
            _ => panic!("expected authenticated"),
        }
    }

    #[test]
    fn matches_later_record() {
        let recs = vec![rec("a", AuthRole::Full, "tok-a"), rec("b", AuthRole::Public, "tok-b")];
        let outcome = require_bearer_record(&auth_headers("tok-b"), &recs).expect("ok");
        match outcome {
            AuthOutcome::Authenticated(r) => assert_eq!(r.name, "b"),
            _ => panic!("expected authenticated"),
        }
    }

    #[test]
    fn unknown_token_is_unauthorized() {
        let recs = vec![rec("a", AuthRole::Full, "tok-a")];
        let result = require_bearer_record(&auth_headers("wrong"), &recs);
        assert!(matches!(result, Err(AppError::Unauthorized)));
    }

    #[test]
    fn missing_header_when_tokens_required_is_unauthorized() {
        let recs = vec![rec("a", AuthRole::Full, "tok-a")];
        let result = require_bearer_record(&HeaderMap::new(), &recs);
        assert!(matches!(result, Err(AppError::Unauthorized)));
    }

    #[test]
    fn malformed_header_is_unauthorized() {
        let recs = vec![rec("a", AuthRole::Full, "tok-a")];
        let mut h = HeaderMap::new();
        h.insert("authorization", HeaderValue::from_static("NotBearer xyz"));
        let result = require_bearer_record(&h, &recs);
        assert!(matches!(result, Err(AppError::Unauthorized)));
    }

    #[test]
    fn debug_redacts_token() {
        let r = rec("prod-app", AuthRole::Full, "supersecret");
        let dbg = format!("{r:?}");
        assert!(dbg.contains("prod-app"));
        assert!(dbg.contains("***"));
        assert!(!dbg.contains("supersecret"), "token leaked into debug output: {dbg}");
    }

    #[test]
    fn require_bearer_passes_when_no_token_configured() {
        let result = require_bearer(&HeaderMap::new(), None);
        assert!(result.is_ok());
    }

    #[test]
    fn require_bearer_rejects_wrong_token() {
        let result = require_bearer(&auth_headers("wrong"), Some("right"));
        assert!(matches!(result, Err(AppError::Unauthorized)));
    }
}
