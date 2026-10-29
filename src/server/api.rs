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
        .route("/sync/update", post(sync_update))
        // a workstation job replaces the notional-cost report whole (REQ-COSTS-001);
        // GET serves it. Larger than axum's 2 MB default: a report carries every session.
        .route(
            "/costs",
            get(api_costs)
                .post(api_costs_post)
                .layer(axum::extract::DefaultBodyLimit::max(crate::costs::MAX_BODY)),
        );

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
        .route("/sync/state", get(sync_state))
        // Agents: machine-readable queue and PR admission (#55)
        .route("/queue", get(api_queue))
        .route("/admission", get(api_admission))
        .route("/watch", get(api_watch))
        .route("/machines", get(api_machines))
        .route("/openapi.json", get(api_openapi));
    // /api/ws and /api/events are not mounted: they broadcast agent and
    // access events without authentication, and nothing uses them (agents
    // use /api/watch). They return with auth in v1.5.0 (REQ-REALTIME-001).

    Router::new()
        .nest("/api", read_routes.merge(mutating_routes))
        // Dashboard HTML frontend
        .route("/", get(crate::dashboard::dashboard_page))
        .route("/dashboard.css", get(crate::dashboard::dashboard_css))
        .route("/htmx.min.js", get(crate::dashboard::htmx_js))
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
            // a read-only query: GET, so it needs no auth (REQ-API-002 covers mutations)
            get(crate::dashboard::ask_partial),
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
            "/api/dashboard/machines-partial",
            get(crate::dashboard::machines_partial),
        )
        .route(
            "/api/dashboard/costs-partial",
            get(crate::dashboard::costs_partial),
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

// ── Agents: queue and admission (#55) ─────────────────────────────

fn plain(code: StatusCode, msg: &str) -> axum::response::Response {
    (code, msg.to_string()).into_response()
}

/// The hub's latest snapshot and when it arrived, or why there is none (a 503).
fn fleet(state: &AppState) -> Result<(crate::hub::model::Snapshot, i64), &'static str> {
    let hub = state.hub.as_ref().ok_or("no ground station configured")?;
    hub.snapshot().ok_or("no snapshot received yet")
}

async fn api_queue(
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    let repo = q.get("repo").map(String::as_str);
    if repo.is_some_and(|r| !crate::admission::valid_repo(r)) {
        return plain(
            StatusCode::BAD_REQUEST,
            "repo must be a repository name without the org, e.g. spar",
        );
    }
    match fleet(&state) {
        Ok((s, received)) => Json(crate::admission::queue(
            &s,
            received,
            crate::hub::now(),
            repo,
        ))
        .into_response(),
        Err(why) => plain(StatusCode::SERVICE_UNAVAILABLE, why),
    }
}

async fn api_admission(
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    let Some(repo) = q.get("repo").filter(|r| crate::admission::valid_repo(r)) else {
        return plain(
            StatusCode::BAD_REQUEST,
            "repo is required: a repository name without the org",
        );
    };
    let Some(kind) = q.get("kind").and_then(|k| crate::admission::Kind::parse(k)) else {
        return plain(
            StatusCode::BAD_REQUEST,
            "kind must be one of docs, fix, feature, security, red-main",
        );
    };
    match fleet(&state) {
        Ok((s, received)) => Json(crate::admission::admit(
            &s,
            received,
            crate::hub::now(),
            repo,
            kind,
        ))
        .into_response(),
        Err(why) => plain(StatusCode::SERVICE_UNAVAILABLE, why),
    }
}

/// Long-poll: hold until the PR's or branch's CI state differs from `since`,
/// or `timeout` seconds pass. Without `since`, the state at the call is the
/// baseline.
async fn api_watch(
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    req: axum::extract::Request,
) -> axum::response::Response {
    use crate::watch;
    let Some(repo) = q.get("repo").filter(|r| crate::admission::valid_repo(r)) else {
        return plain(
            StatusCode::BAD_REQUEST,
            "repo is required: a repository name without the org",
        );
    };
    let pr = match q.get("pr").map(|p| p.parse::<u32>()) {
        None => None,
        Some(Ok(n)) => Some(n),
        Some(Err(_)) => return plain(StatusCode::BAD_REQUEST, "pr must be a number"),
    };
    let branch = q.get("branch").map(String::as_str);
    if pr.is_some() == branch.is_some() || branch.is_some_and(|b| b.is_empty() || b.len() > 255) {
        return plain(
            StatusCode::BAD_REQUEST,
            "give exactly one of pr or branch (at most 255 bytes)",
        );
    }
    let since = q.get("since").map(String::as_str);
    if since.is_some_and(|c| !watch::valid_cursor(c)) {
        return plain(
            StatusCode::BAD_REQUEST,
            "since must be a cursor from a previous answer",
        );
    }
    let hold = match q.get("timeout").map(|t| t.parse::<u64>()) {
        None => watch::DEFAULT_HOLD_S,
        Some(Ok(t)) => t.min(watch::MAX_HOLD_S),
        Some(Err(_)) => return plain(StatusCode::BAD_REQUEST, "timeout must be seconds"),
    };
    let Some(hub) = state.hub.as_ref() else {
        return plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "no ground station configured",
        );
    };
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip());
    let xff = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok());
    let _slot = match hub.watch_slots().take(watch::client_of(peer, xff)) {
        Ok(slot) => slot,
        Err(true) => {
            return plain(
                StatusCode::TOO_MANY_REQUESTS,
                "too many held watches from this client; retry later",
            )
        }
        Err(false) => {
            return plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "too many watchers; retry later",
            )
        }
    };
    let mut tick = hub.subscribe();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(hold);
    let mut baseline = since.map(String::from);
    loop {
        let Some((s, received)) = hub.snapshot() else {
            return plain(StatusCode::SERVICE_UNAVAILABLE, "no snapshot received yet");
        };
        let p = watch::picture(&s, hub.org(), repo, pr, branch);
        let since = baseline.get_or_insert_with(|| p.cursor()).clone();
        let a = watch::answer(&p, &since, received, s.taken_at, crate::hub::now());
        if a.changed {
            return Json(a).into_response();
        }
        tokio::select! {
            r = tick.changed() => if r.is_err() { return Json(a).into_response() },
            _ = tokio::time::sleep_until(deadline) => return Json(a).into_response(),
        }
    }
}

/// What `/api/machines` publishes of a report. The read API has no auth
/// (warte#46), so it leaves out what only helps an attacker map a machine:
/// tool paths, tool error output, process ids and working directories.
fn public_report(r: &crate::probe::Report) -> serde_json::Value {
    let mut sessions: std::collections::BTreeMap<&str, usize> = Default::default();
    for s in &r.sessions {
        *sessions.entry(s.agent.as_str()).or_default() += 1;
    }
    serde_json::json!({
        "host": r.host,
        "taken_at": r.taken_at,
        "os": r.os,
        "uptime_s": r.uptime_s,
        "cpu": r.cpu,
        "memory": r.memory,
        "disks": r.disks,
        "tools": r.tools.iter().map(|t| serde_json::json!({
            "name": t.name,
            "installed": t.path.is_some(),
            "version": t.version,
        })).collect::<Vec<_>>(),
        "sessions": sessions,
    })
}

/// The latest notional-cost report, or `report: null` before the first push.
async fn api_costs(State(state): State<AppState>) -> axum::response::Response {
    let Some(c) = state.costs.as_ref() else {
        return plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "no ground station configured",
        );
    };
    let now = crate::hub::now();
    match c.latest() {
        Some(s) => Json(serde_json::json!({
            "label": crate::costs::LABEL,
            "received_at": s.received_at,
            "age_s": now - s.received_at,
            "stale": now - s.received_at > crate::costs::STALE_S,
            "report": s.report,
        }))
        .into_response(),
        None => Json(serde_json::json!({
            "label": crate::costs::LABEL,
            "received_at": null,
            "report": null,
            "rejected": c.rejected(),
        }))
        .into_response(),
    }
}

/// Replace the notional-cost report. The read API is public (warte#46), so a
/// report must hold numbers and names only: unknown fields and free text are
/// rejected with a 400 and nothing is stored.
async fn api_costs_post(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(c) = state.costs.as_ref() else {
        return plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "no ground station configured",
        );
    };
    let report: crate::costs::CostReport = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return plain(StatusCode::BAD_REQUEST, &format!("not a costs report: {e}")),
    };
    if let Err(why) = report.validate() {
        return plain(StatusCode::BAD_REQUEST, &why);
    }
    let sessions = report.sessions.len();
    match c.record(report, crate::hub::now()) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "stored": true, "sessions": sessions })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to store the costs report");
            plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not store the report",
            )
        }
    }
}

/// Every machine the ground station hears from, with drift findings.
async fn api_machines(State(state): State<AppState>) -> axum::response::Response {
    let Some(m) = state.machines.as_ref() else {
        return plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "no ground station configured",
        );
    };
    let now = crate::hub::now();
    let seen = m.list();
    let machines: Vec<serde_json::Value> = seen
        .iter()
        .map(|s| {
            serde_json::json!({
                "host": s.report.host,
                "via": s.via,
                "received_at": s.received_at,
                "age_s": now - s.received_at,
                "stale": now - s.received_at > crate::probe::MACHINE_STALE_S,
                "report": public_report(&s.report),
            })
        })
        .collect();
    Json(serde_json::json!({
        "machines": machines,
        "findings": crate::probe::findings(&seen, now),
    }))
    .into_response()
}

async fn api_openapi() -> axum::response::Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        include_str!("../admission/openapi.json"),
    )
        .into_response()
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
