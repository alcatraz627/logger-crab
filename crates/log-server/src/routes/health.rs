use axum::Json;
use axum::extract::State;
use serde::Serialize;

use super::AppState;
use crate::error::AppError;

#[derive(Serialize)]
pub struct HealthResponse {
    ok: bool,
    hot_ok: bool,
    cold_ok: bool,
}

pub async fn get_health(State(state): State<AppState>) -> Result<Json<HealthResponse>, AppError> {
    let hot = state.hot.health().await?;
    let cold = state.cold.health().await?;
    Ok(Json(HealthResponse { ok: hot.ok && cold.ok, hot_ok: hot.ok, cold_ok: cold.ok }))
}
