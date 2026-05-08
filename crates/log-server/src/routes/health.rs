use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Serialize;

use super::AppState;
use crate::error::AppError;
use crate::models::{ColdHealth, HotHealth};

/// Thin health response — public, intentionally cheap. Render's healthCheckPath
/// hits this without sending any auth header. Returns just the boolean state
/// so external monitoring can poll without leaking bucket names, error
/// messages, or row counts.
#[derive(Serialize)]
pub struct ThinHealth {
    ok: bool,
}

/// Rich health response — same shape as before. Auth-gated with the same
/// DASHBOARD_TOKEN bearer/basic the dashboard uses.
#[derive(Serialize)]
pub struct FullHealth {
    ok: bool,
    hot: HotHealth,
    cold: ColdHealth,
}

/// Public, unauthenticated. Returns `{ok}` only.
pub async fn get_health(State(state): State<AppState>) -> Result<Json<ThinHealth>, AppError> {
    let hot = state.hot.health().await?;
    let cold = state.cold.health().await?;
    Ok(Json(ThinHealth { ok: hot.ok && cold.ok }))
}

/// Auth-gated detailed health. Same auth as the dashboard (DASHBOARD_TOKEN
/// via Bearer or HTTP Basic). Returns the per-tier detail useful for ops.
pub async fn get_health_full(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<FullHealth>, AppError> {
    super::auth::require_dashboard_auth(
        &headers,
        state.dashboard_token.as_deref().map(|s| s.as_str()),
    )?;

    let hot = state.hot.health().await?;
    let cold = state.cold.health().await?;
    let ok = hot.ok && cold.ok;
    Ok(Json(FullHealth { ok, hot, cold }))
}
