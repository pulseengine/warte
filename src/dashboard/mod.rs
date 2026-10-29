//! Dashboard HTML frontend — HTMX-based overview, ask, agents, artifacts.
//!
//! REQ-DASH-004: The system SHALL serve an HTML dashboard frontend
//! (HTMX-based, like rivet serve) with views:
//! - Overview: all repos, all gates, green/red/pending
//! - Ask: ETA query form with results
//! - Agents: agent registry with health status
//! - Artifacts: artifact list with download links
//!
//! Views refresh every 30 s and show the age of the data they render.

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
        r#"<div class="overview">
  <h2>Overview</h2>
  {}
  <section class="panel tower"><h3>Tower</h3><div class="kpis">
    <div class="kpi"><div class="l">Agents</div><div class="v">{}</div><div class="s">{} healthy, {} unhealthy</div></div>
    <div class="kpi"><div class="l">Artifacts</div><div class="v">{}</div><div class="s">stored by warte</div></div>
    <div class="kpi"><div class="l">Instance</div><div class="v mono">{}</div><div class="s">warte v{}</div></div>
  </div></section>
  <p class="freshness">Rendered {}</p>
</div>"#,
        fleet::overview(state.hub.as_ref()),
        agents.len(),
        healthy,
        unhealthy,
        artifacts.len(),
        fleet::esc(&state.config.instance_id()[..8]),
        env!("CARGO_PKG_VERSION"),
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
                fleet::esc(&a.id[..8]),
                fleet::esc(&a.name),
                fleet::esc(a.repo.as_deref().unwrap_or("—")),
                health_class,
                a.health_status,
                fleet::esc(&last_seen),
            )
        })
        .collect();

    let body = if agents.is_empty() {
        r#"<p class="empty">No agents registered with warte yet. An agent registers with <code>POST /api/agents/register</code> and sends heartbeats (<code>warte docs agent-registration</code>). Agent sessions on each machine (Claude Code, Copilot CLI, Codex) will come from <code>warte probe</code>; try <code>warte probe report</code> on a machine.</p>"#.to_string()
    } else {
        format!(
            r#"<table class="data-table">
  <thead><tr><th>ID</th><th>Name</th><th>Repo</th><th>Health</th><th>Last Seen</th></tr></thead>
  <tbody>{rows}</tbody>
</table>"#
        )
    };
    let html = format!(
        r#"<div class="overview"><h2>Agents</h2>{body}<p class="freshness">Rendered {}</p></div>"#,
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
  <td><a href="api/artifacts/{}/download" class="btn-small">Download</a></td>
</tr>"#,
                fleet::esc(&a.id[..8]),
                fleet::esc(&a.name),
                fleet::esc(&a.version),
                fleet::esc(&a.content_hash[..12]),
                fleet::esc(&a.id),
            )
        })
        .collect();

    let body = if artifacts.is_empty() {
        r#"<p class="empty">No artifacts stored. Agents publish build outputs here for others to download by hash (<code>warte artifact</code>, <code>warte docs artifact-distribution</code>).</p>"#.to_string()
    } else {
        format!(
            r#"<table class="data-table">
  <thead><tr><th>ID</th><th>Name</th><th>Version</th><th>SHA-256</th><th></th></tr></thead>
  <tbody>{rows}</tbody>
</table>"#
        )
    };
    let html = format!(r#"<div class="overview"><h2>Artifacts</h2>{body}</div>"#);

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
    let result = if repo.is_empty() {
        String::new()
    } else {
        fleet::ask(state.hub.as_ref(), repo, pr, branch)
    };
    let v = |k: &str| fleet::esc(form.get(k).map(|s| s.trim()).unwrap_or_default());
    let (picker, repos) = fleet::ask_picker(state.hub.as_ref());
    Html(format!(
        r##"<div class="overview">
  <h2>Ask</h2>
  <div class="ask-form">
  <h3>When will this PR run?</h3>
  <form hx-get="api/dashboard/ask-partial" hx-target="#content" hx-swap="innerHTML">
    <label>Repo: <input type="text" name="repo" placeholder="rivet" value="{}" list="ask-repos" autocomplete="off" required></label>
    <label>PR: <input type="number" name="pr" placeholder="511" value="{}"></label>
    <label>or branch: <input type="text" name="branch" placeholder="main" value="{}"></label>
    <button type="submit" class="btn">Estimate</button>
  </form>
  <datalist id="ask-repos">{repos}</datalist>
  <div id="ask-result">{result}</div>
  </div>
  {picker}
</div>"##,
        v("repo"),
        v("pr"),
        v("branch"),
    ))
    .into_response()
}

/// Serve the vendored htmx (2.0.4, Zero-Clause BSD; templates/vendor/).
pub async fn htmx_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_str!("../../templates/vendor/htmx-2.0.4.min.js"),
    )
        .into_response()
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
                        fleet::esc(board),
                        fleet::esc(&agent_id.chars().take(8).collect::<String>()),
                        fleet::esc(&duration),
                    )
                } else {
                    format!(
                        r#"<tr><td><span class="badge green">FREE</span></td><td><code>{}</code></td><td>—</td><td>—</td></tr>"#,
                        fleet::esc(board),
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
        r#"<div class="overview">
  <h2>Queue</h2>
  {fleet}
  <section class="panel"><h3>Hardware availability</h3>{hw_table}</section>
</div>"#,
    );

    Html(html).into_response()
}

/// HTMX partial: critical path view (longest chain of dependent gates).
pub async fn critical_path_partial(State(state): State<crate::server::AppState>) -> Response {
    let html = format!(
        r#"<div class="overview">
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
                fleet::esc(&a.name),
                fleet::esc(a.repo.as_deref().unwrap_or("—")),
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
                    fleet::esc(&agent_id.chars().take(8).collect::<String>()),
                    fleet::esc(content),
                    fleet::esc(captured_at),
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
        r#"<div class="overview">
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

/// HTMX partial: outages view: what the ground station detects as wrong.
pub async fn outages_partial(State(state): State<crate::server::AppState>) -> Response {
    let agents = state.agents.list().await.unwrap_or_default();
    let unhealthy: Vec<String> = agents
        .iter()
        .filter(|a| a.health_status == crate::agent::HealthStatus::Unhealthy)
        .map(|a| fleet::esc(&a.name))
        .collect();
    let agents_line = if unhealthy.is_empty() {
        format!(
            r#"<p class="empty">{} registered agent(s), none unhealthy.</p>"#,
            agents.len()
        )
    } else {
        format!(
            r#"<div class="incident sev2"><span class="badge red">down</span>Unhealthy agents: {}.</div>"#,
            unhealthy.join(", ")
        )
    };
    Html(format!(
        r#"<div class="overview">
  <h2>Outages</h2>
  {}
  <section class="panel"><h3>Registered agents</h3>{agents_line}</section>
</div>"#,
        fleet::incidents(state.hub.as_ref()),
    ))
    .into_response()
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
                    fleet::esc(&agent_id.chars().take(8).collect::<String>()),
                    fleet::esc(content),
                    fleet::esc(captured_at),
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
        r#"<div class="overview">
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
                    fleet::esc(&agent_id.chars().take(8).collect::<String>()),
                    fleet::esc(content),
                    fleet::esc(captured_at),
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
        r#"<div class="overview">
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
        r#"<div class="overview">
  <h2>Health</h2>
  {}
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
        fleet::health_view(state.hub.as_ref()),
        total,
        healthy,
        unhealthy,
        fleet::health_cards(state.hub.as_ref()),
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}
