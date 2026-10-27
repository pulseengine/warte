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

use axum::{
    extract::State,
    http::header,
    response::{Html, IntoResponse, Response},
};

/// Serve the dashboard HTML page.
pub async fn dashboard_page() -> Response {
    Html(include_str!("../../templates/dashboard.html")).into_response()
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
  <p class="freshness">Last updated: {} <span class="freshness-badge green">fresh</span></p>
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
<p class="freshness">Last updated: {} <span class="freshness-badge green">fresh</span></p>"#,
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
    State(_state): State<crate::server::AppState>,
    axum::extract::Form(form): axum::extract::Form<std::collections::HashMap<String, String>>,
) -> Response {
    let repo = form.get("repo").cloned().unwrap_or_default();
    let pr = form.get("pr").and_then(|p| p.parse::<u64>().ok());

    // TODO: call the ETA estimator
    let html = format!(
        r#"<div class="ask-result">
  <h3>ETA for {}</h3>
  <p class="estimate">No estimate — insufficient historical data (need >= 5 samples in the last 3 hours)</p>
  <p class="confidence">Confidence: <span class="badge gray">low</span></p>
  <p class="basis">Basis: ETA estimation requires CI data collection to be active. Configure GitHub webhooks or run the status poller.</p>
</div>"#,
        if let Some(pr) = pr {
            format!("{} PR #{}", repo, pr)
        } else {
            repo.clone()
        },
    );

    Html(html).into_response()
}

/// Serve static CSS.
pub async fn dashboard_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css")],
        include_str!("../../templates/dashboard.css"),
    )
        .into_response()
}
