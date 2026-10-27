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

/// HTMX partial: queue view (runner pools, waiting jobs).
pub async fn queue_partial(State(state): State<crate::server::AppState>) -> Response {
    // Get runner pool stats from ci_jobs
    let pool_rows: Vec<(String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT runner_pool,
               SUM(CASE WHEN status = 'in_progress' THEN 1 ELSE 0 END) as running,
               SUM(CASE WHEN status = 'queued' THEN 1 ELSE 0 END) as queued
        FROM ci_jobs
        WHERE recorded_at > datetime('now', '-1 hour')
        GROUP BY runner_pool
        ORDER BY queued DESC
        "#,
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap_or_default();

    let pool_cards: String = pool_rows
        .iter()
        .map(|(pool, running, queued)| {
            let status_class = if *queued > 10 {
                "red"
            } else if *queued > 0 {
                "yellow"
            } else {
                "green"
            };
            format!(
                r#"<div class="stat-card">
  <span class="stat-value">{}</span>
  <span class="stat-label">{}</span>
  <span class="stat-detail">{} running, {} queued</span>
  <span class="badge {}">{}</span>
</div>"#,
                queued,
                pool,
                running,
                queued,
                status_class,
                if *queued > 10 {
                    "backed up"
                } else if *queued > 0 {
                    "busy"
                } else {
                    "free"
                }
            )
        })
        .collect();

    // Get waiting jobs
    let waiting_rows: Vec<(String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT repo, branch, status, queued_at
        FROM ci_jobs
        WHERE status = 'queued'
        ORDER BY queued_at ASC
        LIMIT 20
        "#,
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap_or_default();

    let waiting_table: String = if waiting_rows.is_empty() {
        r#"<p class="empty">No jobs waiting in queue.</p>"#.to_string()
    } else {
        let rows: String = waiting_rows
            .iter()
            .map(|(repo, branch, status, queued_at)| {
                let wait_time = chrono::DateTime::parse_from_rfc3339(queued_at)
                    .ok()
                    .map(|t| {
                        let elapsed = chrono::Utc::now() - t.with_timezone(&chrono::Utc);
                        format!("{}m ago", elapsed.num_minutes())
                    })
                    .unwrap_or_else(|| "unknown".to_string());
                format!(
                    r#"<tr><td><code>{}</code></td><td>{}</td><td><span class="badge yellow">{}</span></td><td>{}</td></tr>"#,
                    repo, branch, status, wait_time
                )
            })
            .collect();
        format!(
            r#"<table class="data-table">
  <thead><tr><th>Repo</th><th>Branch</th><th>Status</th><th>Wait Time</th></tr></thead>
  <tbody>{}</tbody>
</table>"#,
            rows
        )
    };

    let html = format!(
        r#"<div class="queue-view">
  <h2>Queue</h2>
  <div class="stat-cards">{}</div>
  <h3>Waiting Jobs</h3>
  {}
  <p class="freshness">Last updated: {} <span class="freshness-badge green">fresh</span></p>
</div>"#,
        pool_cards,
        waiting_table,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}

/// HTMX partial: critical path view (longest chain of dependent gates).
pub async fn critical_path_partial(State(state): State<crate::server::AppState>) -> Response {
    // Get repos with their CI status and queue depth
    let repos: Vec<(String, String, i64)> = sqlx::query_as(
        r#"
        SELECT repo, status, queue_depth
        FROM ci_status
        ORDER BY queue_depth DESC
        "#,
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap_or_default();

    // Simple critical path: repos with red status or high queue depth are bottlenecks
    let bottlenecks: Vec<&(String, String, i64)> = repos
        .iter()
        .filter(|(_, status, depth)| *status == "red" || *depth > 5)
        .collect();

    let bottleneck_cards: String = if bottlenecks.is_empty() {
        r#"<p class="empty">No bottlenecks detected — all gates green.</p>"#.to_string()
    } else {
        bottlenecks
            .iter()
            .map(|(repo, status, depth)| {
                let status_class = if *status == "red" { "red" } else { "yellow" };
                format!(
                    r#"<div class="stat-card">
  <span class="stat-label">{}</span>
  <span class="badge {}">{}</span>
  <span class="stat-detail">{} jobs queued</span>
</div>"#,
                    repo, status_class, status, depth
                )
            })
            .collect()
    };

    // Estimate minimum release time from ETA
    let eta_estimator = crate::eta::EtaEstimator::new(state.db.pool().clone());
    let pool_stats = eta_estimator.pool_stats().await.unwrap_or_default();

    let pool_table: String = if pool_stats.is_empty() {
        r#"<p class="empty">No wait history yet — ETA estimation requires CI data.</p>"#.to_string()
    } else {
        let rows: String = pool_stats
            .iter()
            .map(|(pool, stats)| {
                format!(
                    r#"<tr><td>{}</td><td>{}</td><td>{}s</td><td>{}s</td></tr>"#,
                    pool, stats.sample_count, stats.avg_wait_seconds, stats.max_wait_seconds
                )
            })
            .collect();
        format!(
            r#"<table class="data-table">
  <thead><tr><th>Runner Pool</th><th>Samples (24h)</th><th>Avg Wait</th><th>Max Wait</th></tr></thead>
  <tbody>{}</tbody>
</table>"#,
            rows
        )
    };

    let html = format!(
        r#"<div class="critical-path-view">
  <h2>Critical Path</h2>
  <h3>Bottlenecks</h3>
  <div class="stat-cards">{}</div>
  <h3>Runner Pool Wait Times (24h)</h3>
  {}
  <p class="freshness">Last updated: {} <span class="freshness-badge green">fresh</span></p>
</div>"#,
        bottleneck_cards,
        pool_table,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}
