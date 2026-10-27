//! Dashboard HTML frontend — HTMX-based overview, ask, agents, artifacts.
//!
//! REQ-DASH-004: The system SHALL serve an HTML dashboard frontend
//! (HTMX-based, like rivet serve) with views:
//! - Overview: all repos, all gates, green/red/pending
//! - Ask: ETA query form with results
//! - Agents: agent registry with health status
//! - Artifacts: artifact list with download links
//!
//! The frontend uses SSE (/api/events) for real-time updates and
//! shows freshness indicators for all data.

pub mod fleet;

use axum::{
    extract::State,
    http::header,
    response::{Html, IntoResponse, Response},
};

/// Serve the dashboard HTML page.
pub async fn dashboard_page() -> Response {
    Html(
        include_str!("../../templates/dashboard.html")
            .replace("{{VERSION}}", env!("CARGO_PKG_VERSION")),
    )
    .into_response()
}

/// HTMX partial: overview content (auto-refreshes via SSE).
pub async fn overview_partial(State(state): State<crate::server::AppState>) -> Response {
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

    let html = format!(
        r#"<div class="overview" hx-swap-oob="true" id="overview-data">
  <div class="stat-cards">
    <div class="stat-card">
      <span class="stat-value">{}</span>
      <span class="stat-label">Agents</span>
      <span class="stat-detail">{} healthy, {} unhealthy</span>
    </div>
    <div class="stat-card">
      <span class="stat-value">{}</span>
      <span class="stat-label">Artifacts</span>
    </div>
    <div class="stat-card">
      <span class="stat-value">{}</span>
      <span class="stat-label">Instance</span>
      <span class="stat-detail">{}</span>
    </div>
  </div>
  <p class="freshness">Rendered {}</p>
</div>"#,
        agents.len(),
        healthy,
        unhealthy,
        artifacts.len(),
        state.config.instance_id(),
        &state.config.instance_id()[..8],
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}

/// HTMX partial: agents list.
pub async fn agents_partial(State(state): State<crate::server::AppState>) -> Response {
    let agents = state.agents.list().await.unwrap_or_default();

    let rows: String = agents
        .iter()
        .map(|a| {
            let health_class = match a.health_status {
                crate::agent::HealthStatus::Healthy => "green",
                crate::agent::HealthStatus::Degraded => "yellow",
                crate::agent::HealthStatus::Unhealthy => "red",
                crate::agent::HealthStatus::Unknown => "gray",
            };
            let last_seen = a
                .last_heartbeat
                .map(|t| t.format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "never".to_string());
            format!(
                r#"<tr>
  <td><code>{}</code></td>
  <td>{}</td>
  <td>{}</td>
  <td><span class="badge {}">{}</span></td>
  <td>{}</td>
</tr>"#,
                &a.id[..8],
                a.name,
                a.repo.as_deref().unwrap_or("—"),
                health_class,
                a.health_status,
                last_seen,
            )
        })
        .collect();

    let html = format!(
        r#"<table class="data-table">
  <thead><tr><th>ID</th><th>Name</th><th>Repo</th><th>Health</th><th>Last Seen</th></tr></thead>
  <tbody>{}</tbody>
</table>
<p class="freshness">Rendered {}</p>"#,
        rows,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}

/// HTMX partial: artifacts list.
pub async fn artifacts_partial(State(state): State<crate::server::AppState>) -> Response {
    let artifacts = state.artifacts.list().await.unwrap_or_default();

    let rows: String = artifacts
        .iter()
        .map(|a| {
            format!(
                r#"<tr>
  <td><code>{}</code></td>
  <td>{}</td>
  <td>{}</td>
  <td><code>{}</code></td>
  <td><a href="/api/artifacts/{}/download" class="btn-small">Download</a></td>
</tr>"#,
                &a.id[..8],
                a.name,
                a.version,
                &a.content_hash[..12],
                a.id,
            )
        })
        .collect();

    let html = format!(
        r#"<table class="data-table">
  <thead><tr><th>ID</th><th>Name</th><th>Version</th><th>SHA-256</th><th></th></tr></thead>
  <tbody>{}</tbody>
</table>"#,
        rows,
    );

    Html(html).into_response()
}

/// HTMX partial: ask form result.
pub async fn ask_partial(
    State(state): State<crate::server::AppState>,
    axum::extract::Query(form): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let repo = form.get("repo").map(|r| r.trim()).unwrap_or_default();
    // "pulseengine/spar" and "spar" both mean the repo; the hub wants the bare name
    let repo = repo.rsplit('/').next().unwrap_or_default();
    let pr = form
        .get("pr")
        .and_then(|p| p.trim().parse::<u32>().ok())
        .filter(|n| *n > 0);
    let branch = form
        .get("branch")
        .map(|b| b.trim())
        .filter(|b| !b.is_empty());
    Html(fleet::ask(state.hub.as_ref(), repo, pr, branch)).into_response()
}

/// Serve static CSS.
pub async fn dashboard_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css")],
        include_str!("../../templates/dashboard.css"),
    )
        .into_response()
}

/// HTMX partial: queue view (runner pools, waiting jobs).
pub async fn queue_partial(State(state): State<crate::server::AppState>) -> Response {
    // Pools and waiting jobs come from the ground station (DD-SMITHY-001).
    let fleet = fleet::queue(state.hub.as_ref());

    // Get hardware catalog (all known boards) and claims
    let hw_catalog: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT content FROM agent_data
        WHERE data_type = 'hardware-catalog'
        ORDER BY content
        "#,
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap_or_default();

    let hw_claims: Vec<(String, String, String)> = sqlx::query_as(
        r#"
        SELECT agent_id, content, captured_at FROM agent_data
        WHERE data_type = 'hardware-claim'
        ORDER BY captured_at DESC
        LIMIT 50
        "#,
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap_or_default();

    // Build a map of claimed boards
    let claimed: std::collections::HashMap<String, (String, String)> = hw_claims
        .iter()
        .map(|(agent_id, board, captured_at)| {
            (board.clone(), (agent_id.clone(), captured_at.clone()))
        })
        .collect();

    let hw_table: String = if hw_catalog.is_empty() && hw_claims.is_empty() {
        r#"<p class="empty">No hardware catalog or claims. Agents can report boards via agent_data with data_type='hardware-catalog' or 'hardware-claim'.</p>"#.to_string()
    } else {
        // Show all boards from catalog + any claimed boards not in catalog
        let mut all_boards: Vec<String> = hw_catalog.iter().map(|(b,)| b.clone()).collect();
        for board in claimed.keys() {
            if !all_boards.contains(board) {
                all_boards.push(board.clone());
            }
        }
        all_boards.sort();

        let rows: String = all_boards
            .iter()
            .map(|board| {
                if let Some((agent_id, captured_at)) = claimed.get(board) {
                    let duration = chrono::DateTime::parse_from_rfc3339(captured_at)
                        .ok()
                        .map(|t| {
                            let elapsed = chrono::Utc::now() - t.with_timezone(&chrono::Utc);
                            format!("{}h {}m", elapsed.num_hours(), elapsed.num_minutes() % 60)
                        })
                        .unwrap_or_else(|| "unknown".to_string());
                    format!(
                        r#"<tr><td><span class="badge red">CLAIMED</span></td><td><code>{}</code></td><td>{}</td><td>{}</td></tr>"#,
                        board,
                        &agent_id[..8.min(agent_id.len())],
                        duration,
                    )
                } else {
                    format!(
                        r#"<tr><td><span class="badge green">FREE</span></td><td><code>{}</code></td><td>—</td><td>—</td></tr>"#,
                        board,
                    )
                }
            })
            .collect();
        format!(
            r#"<table class="data-table">
  <thead><tr><th>Status</th><th>Board</th><th>Agent</th><th>Duration</th></tr></thead>
  <tbody>{}</tbody>
</table>"#,
            rows
        )
    };

    let html = format!(
        r#"<div class="queue-view">
  <h2>Queue</h2>
  {}
  <h3>Hardware Availability</h3>
  {}
</div>"#,
        fleet, hw_table,
    );

    Html(html).into_response()
}

/// HTMX partial: critical path view (longest chain of dependent gates).
pub async fn critical_path_partial(State(state): State<crate::server::AppState>) -> Response {
    let html = format!(
        r#"<div class="critical-path-view">
  <h2>Critical Path</h2>
  {}
</div>"#,
        fleet::bottlenecks(state.hub.as_ref()),
    );
    Html(html).into_response()
}

/// HTMX partial: resources view (agent resource needs, hardware claims).
pub async fn resources_partial(State(state): State<crate::server::AppState>) -> Response {
    let agents = state.agents.list().await.unwrap_or_default();

    // Get hardware claims from agent_data
    let claims: Vec<(String, String, String)> = sqlx::query_as(
        r#"
        SELECT agent_id, content, captured_at FROM agent_data
        WHERE data_type = 'hardware-claim'
        ORDER BY captured_at DESC
        LIMIT 20
        "#,
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap_or_default();

    let agent_cards: String = agents
        .iter()
        .map(|a| {
            let health_class = match a.health_status {
                crate::agent::HealthStatus::Healthy => "green",
                crate::agent::HealthStatus::Degraded => "yellow",
                crate::agent::HealthStatus::Unhealthy => "red",
                crate::agent::HealthStatus::Unknown => "gray",
            };
            format!(
                r#"<div class="stat-card">
  <span class="stat-label">{}</span>
  <span class="stat-detail">{}</span>
  <span class="badge {}">{}</span>
</div>"#,
                a.name,
                a.repo.as_deref().unwrap_or("—"),
                health_class,
                a.health_status,
            )
        })
        .collect();

    let claims_table: String = if claims.is_empty() {
        r#"<p class="empty">No hardware claims.</p>"#.to_string()
    } else {
        let rows: String = claims
            .iter()
            .map(|(agent_id, content, captured_at)| {
                format!(
                    r#"<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>"#,
                    &agent_id[..8.min(agent_id.len())],
                    content,
                    captured_at,
                )
            })
            .collect();
        format!(
            r#"<table class="data-table">
  <thead><tr><th>Agent</th><th>Claim</th><th>Since</th></tr></thead>
  <tbody>{}</tbody>
</table>"#,
            rows
        )
    };

    let html = format!(
        r#"<div class="resources-view">
  <h2>Resources</h2>
  <h3>Agents</h3>
  <div class="stat-cards">{}</div>
  <h3>Hardware Claims</h3>
  {}
  <p class="freshness">Rendered {}</p>
</div>"#,
        agent_cards,
        claims_table,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}

/// HTMX partial: outages view (GitHub status, fleet health).
pub async fn outages_partial(State(state): State<crate::server::AppState>) -> Response {
    let agents = state.agents.list().await.unwrap_or_default();
    let healthy = agents
        .iter()
        .filter(|a| a.health_status == crate::agent::HealthStatus::Healthy)
        .count();
    let unhealthy = agents
        .iter()
        .filter(|a| a.health_status == crate::agent::HealthStatus::Unhealthy)
        .count();

    let fleet_status = if unhealthy > 0 {
        "degraded"
    } else {
        "operational"
    };
    let fleet_class = if unhealthy > 0 { "yellow" } else { "green" };

    let html = format!(
        r#"<div class="outages-view">
  <h2>Outages & Health</h2>
  <div class="stat-cards">
    <div class="stat-card">
      <span class="stat-label">Fleet Status</span>
      <span class="badge {}">{}</span>
      <span class="stat-detail">{} healthy, {} unhealthy</span>
    </div>
    <div class="stat-card">
      <span class="stat-label">GitHub</span>
      <span class="badge green">operational</span>
      <span class="stat-detail">status.github.com</span>
    </div>
    <div class="stat-card">
      <span class="stat-label">Instance</span>
      <span class="stat-detail">{}</span>
    </div>
  </div>
  <p class="freshness">Rendered {}</p>
</div>"#,
        fleet_class,
        fleet_status,
        healthy,
        unhealthy,
        &state.config.instance_id()[..8.min(state.config.instance_id().len())],
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}

/// HTMX partial: friction view (tool errors, workarounds).
pub async fn friction_partial(State(state): State<crate::server::AppState>) -> Response {
    let friction: Vec<(String, String, String)> = sqlx::query_as(
        r#"
        SELECT agent_id, content, captured_at FROM agent_data
        WHERE data_type = 'friction'
        ORDER BY captured_at DESC
        LIMIT 20
        "#,
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap_or_default();

    let table: String = if friction.is_empty() {
        r#"<p class="empty">No friction reports. Tools are working well.</p>"#.to_string()
    } else {
        let rows: String = friction
            .iter()
            .map(|(agent_id, content, captured_at)| {
                format!(
                    r#"<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>"#,
                    &agent_id[..8.min(agent_id.len())],
                    content,
                    captured_at,
                )
            })
            .collect();
        format!(
            r#"<table class="data-table">
  <thead><tr><th>Agent</th><th>Friction</th><th>Reported</th></tr></thead>
  <tbody>{}</tbody>
</table>"#,
            rows
        )
    };

    let html = format!(
        r#"<div class="friction-view">
  <h2>Friction</h2>
  {}
  <p class="freshness">Rendered {}</p>
</div>"#,
        table,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}

/// HTMX partial: schedule view (weekly cron table).
pub async fn schedule_partial(State(state): State<crate::server::AppState>) -> Response {
    let schedules: Vec<(String, String, String)> = sqlx::query_as(
        r#"
        SELECT agent_id, content, captured_at FROM agent_data
        WHERE data_type = 'schedule'
        ORDER BY captured_at DESC
        LIMIT 20
        "#,
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap_or_default();

    let table: String = if schedules.is_empty() {
        r#"<p class="empty">No schedules configured. Use the capture-session-learnings skill to record schedules.</p>"#.to_string()
    } else {
        let rows: String = schedules
            .iter()
            .map(|(agent_id, content, captured_at)| {
                format!(
                    r#"<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>"#,
                    &agent_id[..8.min(agent_id.len())],
                    content,
                    captured_at,
                )
            })
            .collect();
        format!(
            r#"<table class="data-table">
  <thead><tr><th>Agent</th><th>Schedule</th><th>Recorded</th></tr></thead>
  <tbody>{}</tbody>
</table>"#,
            rows
        )
    };

    let html = format!(
        r#"<div class="schedule-view">
  <h2>Schedule</h2>
  {}
  <p class="freshness">Rendered {}</p>
</div>"#,
        table,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}

/// HTMX partial: health view (factory-wide metrics).
pub async fn health_partial(State(state): State<crate::server::AppState>) -> Response {
    let agents = state.agents.list().await.unwrap_or_default();
    let healthy = agents
        .iter()
        .filter(|a| a.health_status == crate::agent::HealthStatus::Healthy)
        .count();
    let unhealthy = agents
        .iter()
        .filter(|a| a.health_status == crate::agent::HealthStatus::Unhealthy)
        .count();
    let total = agents.len();

    let html = format!(
        r#"<div class="health-view">
  <h2>Health</h2>
  <div class="stat-cards">
    <div class="stat-card">
      <span class="stat-value">{}</span>
      <span class="stat-label">Agents</span>
      <span class="stat-detail">{} healthy, {} unhealthy</span>
    </div>
    {}
  </div>
  <p class="freshness">Rendered {}</p>
</div>"#,
        total,
        healthy,
        unhealthy,
        fleet::health_cards(state.hub.as_ref()),
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}
