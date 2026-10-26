//! API routes.

use crate::agent::{Agent, RegisterRequest};
use crate::server::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Build all API routes.
pub fn build_routes() -> Router<AppState> {
    Router::new()
        // Health
        .route("/health", get(health))
        // Agents
        .route("/agents", get(list_agents))
        .route("/agents/register", post(register_agent))
        .route("/agents/{id}", get(get_agent))
        .route("/agents/{id}/heartbeat", post(agent_heartbeat))
        .route("/agents/{id}/data", post(capture_data).get(query_data))
        // Access control
        .route("/access/grant", post(grant_access))
        .route("/access/revoke", post(revoke_access))
        .route("/access/list/{agent_id}", get(list_access))
        // Artifacts
        .route("/artifacts", get(list_artifacts))
        .route("/artifacts/{id}/download", get(download_artifact))
        // Dashboard
        .route("/dashboard/overview", get(dashboard_overview))
        .route("/dashboard/ask", get(dashboard_ask))
        // Sync
        .route("/sync/state", get(sync_state))
        .route("/sync/update", post(sync_update))
}

// ── Health ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    instance_id: String,
    version: String,
    uptime_secs: u64,
    timestamp: DateTime<Utc>,
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "healthy".to_string(),
        instance_id: state.config.instance_id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: 0, // TODO: track actual uptime
        timestamp: Utc::now(),
    })
}

// ── Agents ──────────────────────────────────────────────────────────

async fn list_agents(State(state): State<AppState>) -> Result<Json<Vec<Agent>>, StatusCode> {
    state
        .agents
        .list()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn register_agent(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<crate::agent::RegisterResponse>, StatusCode> {
    state.agents.register(req).await.map(Json).map_err(|e| {
        tracing::error!(error = %e, "agent registration failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Agent>, StatusCode> {
    state
        .agents
        .get(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn agent_heartbeat(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.agents.heartbeat(&id).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ── Agent data capture ──────────────────────────────────────────────

#[derive(Deserialize)]
struct CaptureDataRequest {
    data_type: String,
    content: String,
}

async fn capture_data(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<CaptureDataRequest>,
) -> impl IntoResponse {
    let id = uuid::Uuid::now_v7().to_string();
    let now = Utc::now();

    let result = sqlx::query(
        r#"
        INSERT INTO agent_data (id, agent_id, data_type, content, captured_at, instance_id)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(&agent_id)
    .bind(&req.data_type)
    .bind(&req.content)
    .bind(now.to_rfc3339())
    .bind(state.config.instance_id())
    .execute(state.db.pool())
    .await;

    match result {
        Ok(_) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))),
        Err(e) => {
            tracing::error!(error = %e, "failed to capture agent data");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "internal error" })),
            )
        }
    }
}

#[derive(Deserialize)]
struct QueryDataParams {
    data_type: Option<String>,
    limit: Option<i64>,
}

async fn query_data(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(params): Query<QueryDataParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let limit = params.limit.unwrap_or(100).min(1000);

    let rows: Vec<(String, String, String, String)> = if let Some(ref dt) = params.data_type {
        sqlx::query_as(
            r#"
            SELECT id, data_type, content, captured_at FROM agent_data
            WHERE agent_id = ? AND data_type = ?
            ORDER BY captured_at DESC LIMIT ?
            "#,
        )
        .bind(&agent_id)
        .bind(dt)
        .bind(limit)
        .fetch_all(state.db.pool())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        sqlx::query_as(
            r#"
            SELECT id, data_type, content, captured_at FROM agent_data
            WHERE agent_id = ?
            ORDER BY captured_at DESC LIMIT ?
            "#,
        )
        .bind(&agent_id)
        .bind(limit)
        .fetch_all(state.db.pool())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };

    Ok(Json(serde_json::json!({
        "agent_id": agent_id,
        "count": rows.len(),
        "data": rows.into_iter().map(|(id, data_type, content, captured_at)| {
            serde_json::json!({
                "id": id,
                "data_type": data_type,
                "content": content,
                "captured_at": captured_at,
            })
        }).collect::<Vec<_>>(),
    })))
}

// ── Access control ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct GrantRequest {
    agent_id: String,
    rights: Vec<crate::access::AccessRight>,
    granted_by: String,
}

async fn grant_access(
    State(state): State<AppState>,
    Json(req): Json<GrantRequest>,
) -> impl IntoResponse {
    match state
        .access
        .grant(&req.agent_id, &req.rights, &req.granted_by)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(e) => {
            tracing::error!(error = %e, "failed to grant access");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[derive(Deserialize)]
struct RevokeRequest {
    agent_id: String,
    rights: Vec<crate::access::AccessRight>,
    revoked_by: String,
}

async fn revoke_access(
    State(state): State<AppState>,
    Json(req): Json<RevokeRequest>,
) -> impl IntoResponse {
    match state
        .access
        .revoke(&req.agent_id, &req.rights, &req.revoked_by)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(e) => {
            tracing::error!(error = %e, "failed to revoke access");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn list_access(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Vec<crate::access::AccessRight>>, StatusCode> {
    state
        .access
        .list_rights(&agent_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

// ── Artifacts ───────────────────────────────────────────────────────

async fn list_artifacts(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::artifact::Artifact>>, StatusCode> {
    state
        .artifacts
        .list()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn download_artifact(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::artifact::ArtifactDownload>, StatusCode> {
    state
        .artifacts
        .download(&id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "artifact download failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// ── Dashboard ───────────────────────────────────────────────────────

async fn dashboard_overview(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.agents.list().await.unwrap_or_default();
    let artifacts = state.artifacts.list().await.unwrap_or_default();

    let healthy = agents
        .iter()
        .filter(|a| a.health_status == crate::agent::HealthStatus::Healthy)
        .count();
    let unhealthy = agents
        .iter()
        .filter(|a| a.health_status == crate::agent::HealthStatus::Unhealthy)
        .count();

    Json(serde_json::json!({
        "instance_id": state.config.instance_id(),
        "timestamp": Utc::now(),
        "agents": {
            "total": agents.len(),
            "healthy": healthy,
            "unhealthy": unhealthy,
        },
        "artifacts": {
            "total": artifacts.len(),
        },
        "freshness": "green", // TODO: compute from actual data age
    }))
}

#[derive(Deserialize)]
struct AskParams {
    repo: String,
    pr: Option<u64>,
}

async fn dashboard_ask(
    State(_state): State<AppState>,
    Query(params): Query<AskParams>,
) -> impl IntoResponse {
    // TODO: implement ETA estimation from measured waits
    Json(serde_json::json!({
        "repo": params.repo,
        "pr": params.pr,
        "estimate": "no estimate — insufficient history",
        "confidence": "low",
    }))
}

// ── Sync ────────────────────────────────────────────────────────────

async fn sync_state(
    State(state): State<AppState>,
) -> Result<Json<crate::sync::SyncState>, StatusCode> {
    state
        .sync
        .get_state()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn sync_update(
    State(state): State<AppState>,
    Json(update): Json<crate::sync::SyncUpdate>,
) -> impl IntoResponse {
    match state.sync.apply_remote_update(&update).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(e) => {
            tracing::error!(error = %e, "failed to apply sync update");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
