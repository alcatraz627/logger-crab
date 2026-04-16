use axum::http::HeaderMap;

use crate::error::AppError;

pub fn require_bearer(headers: &HeaderMap, expected: Option<&str>) -> Result<(), AppError> {
    let Some(expected) = expected else {
        // No token configured — tokens are optional in dev. Warn once at boot;
        // here we just let the request through.
        return Ok(());
    };
    let got = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized)?;
    if constant_time_eq(got.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
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
