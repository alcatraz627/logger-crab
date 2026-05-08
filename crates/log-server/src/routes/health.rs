use axum::extract::State;
use axum::Json;
use serde::Serialize;

use super::AppState;
use crate::error::AppError;
use crate::models::{ColdHealth, HotHealth};

/// Rich /health response. `ok` is the AND of hot + cold so a single check
/// catches either tier going down. Per-tier detail surfaces below for ops.
#[derive(Serialize)]
pub struct HealthResponse {
    ok: bool,
    hot: HotHealth,
    cold: ColdHealth,
}

pub async fn get_health(State(state): State<AppState>) -> Result<Json<HealthResponse>, AppError> {
    let hot = state.hot.health().await?;
    let cold = state.cold.health().await?;
    let ok = hot.ok && cold.ok;
    Ok(Json(HealthResponse { ok, hot, cold }))
}
