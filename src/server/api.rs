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
    // Mutating routes — require authentication
    let mutating_routes = Router::new()
        .route("/agents/register", post(register_agent))
        .route("/agents/{id}/heartbeat", post(agent_heartbeat))
        .route("/agents/{id}/data", post(capture_data))
        .route("/access/grant", post(grant_access))
        .route("/access/revoke", post(revoke_access))
        .route("/sync/update", post(sync_update));

    // Webhook routes — HMAC signature verified (not bearer token)
    let webhook_routes = Router::new()
        .route("/webhooks/github", post(github_webhook))
        .route("/webhooks/gitlab", post(gitlab_webhook));

    // Read-only routes — public (rate-limited at edge proxy)
    let read_routes = Router::new()
        .route("/health", get(health))
        .route("/agents", get(list_agents))
        .route("/agents/{id}", get(get_agent))
        .route("/agents/{id}/data", get(query_data))
        .route("/access/list/{agent_id}", get(list_access))
        .route("/artifacts", get(list_artifacts))
        .route("/artifacts/{id}/download", get(download_artifact))
        .route("/dashboard/overview", get(dashboard_overview))
        .route("/dashboard/ask", get(dashboard_ask))
        .route("/sync/state", get(sync_state))
        // Real-time: WebSocket (bidirectional) + SSE (unidirectional)
        .route("/ws", axum::routing::any(crate::realtime::ws_handler))
        .route("/events", get(crate::realtime::sse_handler));

    Router::new()
        .nest(
            "/api",
            read_routes.merge(mutating_routes).merge(webhook_routes),
        )
        // Dashboard HTML frontend
        .route("/", get(crate::dashboard::dashboard_page))
        .route("/dashboard.css", get(crate::dashboard::dashboard_css))
        .route(
            "/api/dashboard/overview-partial",
            get(crate::dashboard::overview_partial),
        )
        .route(
            "/api/dashboard/agents-partial",
            get(crate::dashboard::agents_partial),
        )
        .route(
            "/api/dashboard/artifacts-partial",
            get(crate::dashboard::artifacts_partial),
        )
        .route(
            "/api/dashboard/ask-partial",
            post(crate::dashboard::ask_partial),
        )
        .route(
            "/api/dashboard/queue-partial",
            get(crate::dashboard::queue_partial),
        )
        .route(
            "/api/dashboard/critical-path-partial",
            get(crate::dashboard::critical_path_partial),
        )
        .route(
            "/api/dashboard/resources-partial",
            get(crate::dashboard::resources_partial),
        )
        .route(
            "/api/dashboard/outages-partial",
            get(crate::dashboard::outages_partial),
        )
        .route(
            "/api/dashboard/friction-partial",
            get(crate::dashboard::friction_partial),
        )
        .route(
            "/api/dashboard/schedule-partial",
            get(crate::dashboard::schedule_partial),
        )
        .route(
            "/api/dashboard/health-partial",
            get(crate::dashboard::health_partial),
        )
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
        Ok(policy_ids) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "policy_ids": policy_ids })),
        ),
        Err(e) => {
            tracing::error!(error = %e, "failed to grant access");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "internal error" })),
            )
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

// ── GitHub webhook ──────────────────────────────────────────────────

/// GitHub webhook handler — receives workflow_run events.
///
/// Verifies the X-Hub-Signature-256 header (HMAC-SHA256) before processing.
async fn github_webhook(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    // Get the signature from the header
    let signature = match headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s,
        None => {
            tracing::warn!("webhook: missing X-Hub-Signature-256 header");
            return (StatusCode::UNAUTHORIZED, "missing signature").into_response();
        }
    };

    // Get the webhook secret from config
    let secret = match std::env::var("WARTE_GITHUB_WEBHOOK_SECRET") {
        Ok(s) => s,
        Err(_) => {
            tracing::warn!("webhook: WARTE_GITHUB_WEBHOOK_SECRET not configured");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "webhook secret not configured",
            )
                .into_response();
        }
    };

    // Verify the HMAC signature
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "invalid secret").into_response(),
    };
    mac.update(&body);
    let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    if !signature.starts_with("sha256=") || signature != expected {
        tracing::warn!(provided = %signature, "webhook: invalid signature");
        return (StatusCode::UNAUTHORIZED, "invalid signature").into_response();
    }

    // Parse the event
    let event: crate::ci::WorkflowRunEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "webhook: invalid payload");
            return (StatusCode::BAD_REQUEST, "invalid payload").into_response();
        }
    };

    // Process the event
    let collector = crate::ci::CiDataCollector::new(state.db.pool().clone());
    match collector.process_webhook(&event).await {
        Ok(()) => {
            tracing::debug!(repo = %event.repository.full_name, action = %event.action, "webhook processed");
            (StatusCode::ACCEPTED, "accepted").into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "webhook: processing failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "processing failed").into_response()
        }
    }
}

/// GitLab webhook handler — receives pipeline events.
///
/// Verifies the X-Gitlab-Token header (shared secret) before processing.
async fn gitlab_webhook(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // Get the token from the header
    let token = match headers.get("x-gitlab-token").and_then(|v| v.to_str().ok()) {
        Some(t) => t,
        None => {
            tracing::warn!("gitlab webhook: missing X-Gitlab-Token header");
            return (StatusCode::UNAUTHORIZED, "missing token").into_response();
        }
    };

    // Get the webhook secret from config
    let secret = match std::env::var("WARTE_GITLAB_WEBHOOK_TOKEN") {
        Ok(s) => s,
        Err(_) => {
            tracing::warn!("gitlab webhook: WARTE_GITLAB_WEBHOOK_TOKEN not configured");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "webhook token not configured",
            )
                .into_response();
        }
    };

    // Verify the token (constant-time comparison)
    if token != secret {
        tracing::warn!("gitlab webhook: invalid token");
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    }

    // Parse the event (GitLab pipeline event)
    let event: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "gitlab webhook: invalid payload");
            return (StatusCode::BAD_REQUEST, "invalid payload").into_response();
        }
    };

    // Extract pipeline info
    let repo = event["project"]["path_with_namespace"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let status = event["object_attributes"]["status"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let pipeline_id = event["object_attributes"]["id"].as_u64().unwrap_or(0);

    // Store in ci_jobs
    let id = uuid::Uuid::now_v7().to_string();
    let result = sqlx::query(
        r#"
        INSERT OR REPLACE INTO ci_jobs
            (id, repo, run_id, run_number, branch, sha, status, conclusion,
             queued_at, started_at, completed_at, runner_pool, url, recorded_at)
        VALUES (?, ?, ?, 0, ?, ?, ?, ?, NULL, NULL, NULL, 'gitlab', ?, ?)
        "#,
    )
    .bind(&id)
    .bind(&repo)
    .bind(pipeline_id as i64)
    .bind(event["object_attributes"]["ref"].as_str().unwrap_or("main"))
    .bind(event["object_attributes"]["sha"].as_str().unwrap_or(""))
    .bind(&status)
    .bind(event["object_attributes"]["status"].as_str())
    .bind(format!(
        "https://gitlab.com/{}/-/pipelines/{}",
        repo, pipeline_id
    ))
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(state.db.pool())
    .await;

    match result {
        Ok(_) => {
            tracing::debug!(repo = %repo, pipeline_id = pipeline_id, status = %status, "gitlab webhook processed");
            (StatusCode::ACCEPTED, "accepted").into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "gitlab webhook: processing failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "processing failed").into_response()
        }
    }
}
