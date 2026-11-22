//! Dashboard HTML frontend — HTMX-based overview, ask, agents.
//!
//! REQ-DASH-004: The system SHALL serve an HTML dashboard frontend
//! (HTMX-based, like rivet serve) with views:
//! - Overview: all repos, all gates, green/red/pending
//! - Ask: ETA query form with results
//! - Agents: agent registry with health status
//!
//! Views refresh every 30 s and show the age of the data they render.

pub mod airport;
pub mod briefing;
pub mod charts;
pub mod cihistory;
pub mod compliance;
pub mod costs;
pub mod fleet;
pub mod jobs;
pub mod machines;
pub mod skills;

use axum::{
    extract::{Query, State},
    http::header,
    response::{Html, IntoResponse, Response},
};

/// Serve the dashboard HTML page. The idle scene's settings ride on the body
/// as JSON (REQ-DASH-018); the values are booleans and a number, nothing that
/// needs escaping beyond what serde writes.
pub async fn dashboard_page(State(state): State<crate::server::AppState>) -> Response {
    let idle = serde_json::to_string(&state.idle)
        .unwrap_or_else(|_| "{}".into())
        .replace('\'', "&#39;");
    Html(
        include_str!("../../templates/dashboard.html")
            .replace("{{VERSION}}", env!("CARGO_PKG_VERSION"))
            .replace("{{IDLE}}", &idle)
            .replace("{{NAV}}", &state.nav),
    )
    .into_response()
}

/// The idle scene (REQ-DASH-018).
pub async fn idle_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        include_str!("../../templates/idle.js"),
    )
        .into_response()
}

/// HTMX partial: overview content (auto-refreshes via SSE).
pub async fn overview_partial(State(state): State<crate::server::AppState>) -> Response {
    let agents = state.agents.list().await.unwrap_or_default();

    let healthy = agents
        .iter()
        .filter(|a| a.health_status == crate::agent::HealthStatus::Healthy)
        .count();
    let unhealthy = agents
        .iter()
        .filter(|a| a.health_status == crate::agent::HealthStatus::Unhealthy)
        .count();

    // REQ-DASH-017: Outages at a glance, above the fold
    let mut fleet_found = state
        .hub
        .as_ref()
        .and_then(fleet::detected)
        .map(|(l, _)| l)
        .unwrap_or_default();
    fleet_found.extend(fleet::host_detected(&state, crate::hub::now()));
    let gh = state.status.get_status().await.map_err(|e| e.to_string());
    let strip = fleet::outages_summary(&fleet_found, gh.as_ref().map_err(Clone::clone));
    // REQ-NS-001: the North Star in one line, as published
    let strip = match &state.north_star {
        Some(n) => {
            let line = crate::northstar::glance_line(&n.rows(crate::hub::now()));
            if line.is_empty() {
                strip
            } else {
                strip
                    + &format!(
                        r##"<div class="north-star-line"><span class="label">North Star</span> {} <a href="#" hx-get="api/dashboard/north-star-partial" hx-target="#content" hx-swap="innerHTML" class="dim">the North Star view</a></div>"##,
                        fleet::esc(&line)
                    )
            }
        }
        None => strip,
    };
    // REQ-PREDICT-001: what is forecast to reach its limit
    let coming = state
        .machines
        .as_ref()
        .and_then(|m| crate::forecast::of_machines(m, crate::hub::now()));
    let strip = strip + &crate::forecast::panel(coming.as_ref());
    let strip =
        strip + &crate::canary::panel(state.canaries.as_ref().and_then(|c| c.latest()).as_ref());
    let now = crate::hub::now();
    let strip = strip + &crate::expiry::panel(&state.expiry.report(&state.tokens.seen(now)), now);
    // REQ-PREDICT-005: CI health, this week against the week before
    let strip = match &state.hub {
        Some(h) => strip + &crate::cihealth::panel(&*state.ci_health.get(h, now).await),
        None => strip,
    };
    // REQ-CI-005: one cause failing jobs in several repositories
    let strip = match state.incidents.as_ref().and_then(|i| i.latest()) {
        Some(r) => strip + &crate::incidents::panel(&r),
        None => strip,
    };
    let html = format!(
        r#"<div class="overview">
  <h2>Overview</h2>
  {}
  <section class="panel tower"><h3>Tower</h3><div class="kpis">
    <div class="kpi"><div class="l">Agents</div><div class="v">{}</div><div class="s">{} healthy, {} unhealthy</div></div>
    <div class="kpi"><div class="l">Instance</div><div class="v mono">{}</div><div class="s">warte v{}</div></div>
  </div></section>
  <p class="freshness">Rendered {}</p>
</div>"#,
        strip + &fleet::overview(state.hub.as_ref()),
        agents.len(),
        healthy,
        unhealthy,
        fleet::esc(&state.config.instance_id()[..8]),
        env!("CARGO_PKG_VERSION"),
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}

/// HTMX partial: the airport map (REQ-DASH-015). The drawing is airport.js.
pub async fn airport_partial() -> Response {
    Html(r#"<div class="overview"><h2>Airport</h2><div id="airport" class="airport" aria-label="The fleet drawn as an airport"></div><script src="airport.js"></script></div>"#).into_response()
}

/// The airport page script.
pub async fn airport_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../templates/airport.js"),
    )
        .into_response()
}

/// HTMX partial: machines (REQ-PROBE-001/003).
pub async fn machines_partial(
    State(state): State<crate::server::AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let model = machines::model_panel(
        state.fleet.as_deref(),
        &crate::server::api::reporting_hosts(&state),
    );
    let range = q.get("range").map(String::as_str);
    let (m, hub) = (state.machines.clone(), state.hub.clone());
    let range = range.map(String::from);
    // histories the viewer has open (at most a few, host names only)
    let open: Vec<String> = q
        .get("open")
        .map(|o| {
            o.split(',')
                .filter(|h| !h.is_empty() && h.len() <= 255)
                .take(8)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let page = tokio::task::spawn_blocking(move || {
        machines::view_open(m.as_deref(), hub.as_ref(), range.as_deref(), &open)
    })
    .await
    .unwrap_or_default();
    // the model panel and the modelled-but-silent cards go before the live cards
    Html(page.replacen(
        r#"<div class="machines">"#,
        &format!(r#"{model}<div class="machines">"#),
        1,
    ))
    .into_response()
}

/// HTMX partial: one machine's full history charts (REQ-MHIST-003).
pub async fn machine_history_partial(
    State(state): State<crate::server::AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(h) = state.machines.as_ref().and_then(|m| m.history()).cloned() else {
        return Html(machines::HISTORY_UNAVAILABLE.to_string()).into_response();
    };
    let host = q.get("host").cloned().unwrap_or_default();
    let range = q.get("range").cloned();
    // the latest report's process list goes with the charts (a snapshot)
    let latest = state.machines.as_ref().and_then(|m| {
        m.list()
            .into_iter()
            .find(|s| s.report.host == host)
            .map(|s| (s.report.health, s.report.hardware))
    });
    let (health, hw) = latest.unwrap_or_default();
    let html = tokio::task::spawn_blocking(move || {
        machines::history_panel_full(&h, &host, range.as_deref(), health.as_ref(), hw.as_ref())
    })
    .await
    .unwrap_or_else(|_| machines::HISTORY_UNAVAILABLE.to_string());
    Html(html).into_response()
}

/// HTMX partial: fleet trends and the monthly conveyor-belt report, from the
/// ground station (REQ-SMITHY-006).
pub async fn ci_history_partial(
    State(state): State<crate::server::AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(hub) = state.hub.clone() else {
        return Html(format!(
            r#"<div class="overview"><h2>CI history</h2>{}</div>"#,
            fleet::unavailable(None)
        ))
        .into_response();
    };
    let now = crate::hub::now();
    let (key, span) = cihistory::range(q.get("range").map(String::as_str));
    let months = cihistory::months(now);
    let month = q
        .get("month")
        .filter(|m| months.contains(m))
        .cloned()
        .unwrap_or_else(|| months[0].clone());
    let points = hub.history_since(now - span);
    let self_hosted = state
        .config
        .github
        .as_ref()
        .map(|g| g.self_hosted_labels.clone())
        .unwrap_or_default();
    let trends = cihistory::trends(&points, key, span, now, &month)
        + &cihistory::pool_trends(
            &points,
            now - span,
            now,
            &state.config.runners,
            &self_hosted,
        );
    let belt = match hub.month_report(&month).await {
        Ok(b) => cihistory::belt(&b, &months, key),
        Err(e) => format!(
            r#"<section class="panel"><p class="empty">{}</p></section>"#,
            fleet::esc(e)
        ),
    };
    Html(format!(
        r#"<div class="overview"><h2>CI history</h2>{trends}{belt}<p class="note">Fleet status as Markdown, for terminals and agents: <a href="api/status.md" target="_blank" rel="noopener">api/status.md</a> (one PR: <code>api/status.md?repo=spar&amp;pr=511</code>).</p></div>"#
    ))
    .into_response()
}

/// HTMX partial: scheduled jobs (REQ-JOBS-001).
pub async fn jobs_partial(State(state): State<crate::server::AppState>) -> Response {
    let jobs = crate::server::api::jobs_now(&state);
    Html(jobs::view(jobs.as_deref(), crate::hub::now())).into_response()
}

/// The strip's items (REQ-DASH-020): a disk 95% full or more (counted, no
/// host named: the strip is public) and each workflow red on a public
/// repository's default branch. Both are always-open kinds, shown at any
/// hour; an amber disk, a red scheduled run or another finding is not an
/// item. Routine kinds wait for the owner's attention schedule (#181).
pub fn attention(
    findings: &[crate::probe::Finding],
    red: &[crate::briefing::RedRun],
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let disks = findings
        .iter()
        .filter(|f| f.severity == "red" && f.text.starts_with("disk "))
        .count();
    if disks > 0 {
        out.push(serde_json::json!({
            "kind": "critical-disk",
            "text": format!("{disks} disk(s) 95% full or more"),
            "view": "machines",
        }));
    }
    for r in red.iter().filter(|r| r.kind == "main") {
        out.push(serde_json::json!({
            "kind": "red-main",
            "text": format!("{} is red on main: {}", r.repo, r.workflow),
            "view": "ci-history",
        }));
    }
    out
}

/// Workflows red on a public repository's default branch now (REQ-BRIEF-007's
/// rule over the last two days of the ledger), with the public repositories
/// and default branches the latest briefing read from GitHub. Without a
/// ground station or a briefing there is nothing to say, never a guess.
pub(crate) async fn red_runs_now(
    state: &crate::server::AppState,
    now: i64,
) -> Vec<crate::briefing::RedRun> {
    let (Some(hub), Some(b)) = (
        state.hub.clone(),
        state.briefing.as_ref().and_then(|s| s.latest()),
    ) else {
        return vec![];
    };
    let day = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    };
    let (from, until) = (day(now - 2 * 86_400), day(now));
    let recs = tokio::task::spawn_blocking(move || hub.ledger_days(&from, &until))
        .await
        .unwrap_or_default();
    let public: std::collections::BTreeSet<String> = b.facts.known_repos.iter().cloned().collect();
    crate::briefing::red_runs(&recs, &public, &b.facts.default_branches).0
}

/// REQ-DASH-019: what the grouped menu needs: the views to hide while empty
/// and red and amber counts per group, from the same sources as the views.
/// Public like the views it counts; it carries counts and view names only.
pub async fn nav_state(State(state): State<crate::server::AppState>) -> Response {
    let now = crate::hub::now();
    // an unanswered count shows the view: hiding needs a known zero
    let friction: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM agent_data WHERE data_type = 'friction'")
            .fetch_one(state.db.pool())
            .await
            .unwrap_or((1,));
    let mut hidden = Vec::new();
    if friction.0 == 0 {
        hidden.push("friction");
    }
    let outages = state
        .hub
        .as_ref()
        .and_then(fleet::detected)
        .map(|(l, _)| l.len())
        .unwrap_or(0)
        + fleet::host_detected(&state, now).len();
    let forecasts = state
        .machines
        .as_ref()
        .and_then(|m| crate::forecast::of_machines(m, now))
        .map(|f| f.items.len())
        .unwrap_or(0);
    let stale_after = state
        .config
        .temper
        .as_ref()
        .map(|t| t.stale_after_s)
        .unwrap_or(26 * 3600);
    let (temper_red, temper_amber) = match state.temper.as_ref().map(|t| t.latest()) {
        Some(Some(s)) => (
            s.report.failures().len(),
            usize::from(now - s.received_at > stale_after),
        ),
        Some(None) => (0, 1),
        None => (0, 0),
    };
    let findings = state
        .machines
        .as_ref()
        .map(|m| crate::probe::findings(&m.list(), now))
        .unwrap_or_default();
    let attention = attention(&findings, &red_runs_now(&state, now).await);
    axum::Json(serde_json::json!({
        "attention": attention,
        "hidden": hidden,
        "badges": {
            "now": {"red": outages, "amber": 0},
            "ci": {"red": 0, "amber": 0},
            "machines": {"red": 0, "amber": forecasts},
            "agents": {"red": 0, "amber": 0},
            "compliance": {"red": temper_red, "amber": temper_amber},
        },
    }))
    .into_response()
}

/// REQ-NS-005: the North Star view, as published plus the trend warte observed.
pub async fn north_star_partial(State(state): State<crate::server::AppState>) -> Response {
    let Some(n) = state.north_star.as_deref() else {
        return Html(
            r#"<div class="overview north-star"><h2>North Star</h2><p class="empty">No [north_star] is configured on this warte.</p></div>"#,
        )
        .into_response();
    };
    let now = crate::hub::now();
    let last = n
        .last_poll
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Html(crate::northstar::view::page(
        n.board().as_ref(),
        &n.rows(now),
        &n.store.all(),
        last.as_ref(),
        now,
        n.config.stale_after_days,
    ))
    .into_response()
}

/// HTMX partial: compliance from temper's report (REQ-TEMPER-002).
pub async fn compliance_partial(State(state): State<crate::server::AppState>) -> Response {
    let latest = state.temper.as_ref().and_then(|t| t.latest());
    let stale = state
        .config
        .temper
        .as_ref()
        .map(|t| t.stale_after_s)
        .unwrap_or(26 * 3600);
    Html(compliance::view(
        state.temper.is_some(),
        latest.as_ref(),
        stale,
        crate::hub::now(),
    ))
    .into_response()
}

/// HTMX partial: skills (REQ-SKILLS-001).
pub async fn skills_partial(State(state): State<crate::server::AppState>) -> Response {
    Html(skills::view(state.skills.as_deref(), crate::hub::now())).into_response()
}

/// HTMX partial: the briefing (REQ-BRIEF-003), the latest or `?from=`.
pub async fn briefing_partial(
    State(state): State<crate::server::AppState>,
    axum::extract::RawQuery(q): axum::extract::RawQuery,
) -> Response {
    let from = q
        .as_deref()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("from=")))
        .map(String::from);
    let store = state.briefing.clone();
    let html =
        tokio::task::spawn_blocking(move || briefing::view(store.as_deref(), from.as_deref()))
            .await
            .unwrap_or_default();
    Html(html).into_response()
}

/// HTMX partial: costs (REQ-COSTS-001).
pub async fn costs_partial(State(state): State<crate::server::AppState>) -> Response {
    Html(costs::view(state.costs.as_deref(), crate::hub::now())).into_response()
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

/// The Ask query: repo (bare name), PR, branch, from `?repo=&pr=&branch=`.
fn ask_query(
    form: &std::collections::HashMap<String, String>,
) -> (String, Option<u32>, Option<String>) {
    let repo = form.get("repo").map(|r| r.trim()).unwrap_or_default();
    // "pulseengine/spar" and "spar" both mean the repo; the hub wants the bare name
    let repo = repo.rsplit('/').next().unwrap_or_default().to_string();
    let pr = form
        .get("pr")
        .and_then(|p| p.trim().parse::<u32>().ok())
        .filter(|n| *n > 0);
    let branch = form
        .get("branch")
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty());
    (repo, pr, branch)
}

/// The part of Ask that refreshes on its own: the answer and the in-flight
/// list. The form is outside it, so what is typed stays (REQ-DASH-016).
fn ask_live_html(
    state: &crate::server::AppState,
    form: &std::collections::HashMap<String, String>,
) -> String {
    let (repo, pr, branch) = ask_query(form);
    let result = if repo.is_empty() {
        String::new()
    } else {
        fleet::ask(state.hub.as_ref(), &repo, pr, branch.as_deref())
    };
    let (picker, _) = fleet::ask_picker(state.hub.as_ref());
    format!(r#"<div id="ask-result">{result}</div>{picker}"#)
}

/// HTMX partial: Ask's answer and in-flight list only, for its 30 s refresh.
pub async fn ask_live(
    State(state): State<crate::server::AppState>,
    axum::extract::Query(form): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    Html(ask_live_html(&state, &form)).into_response()
}

/// HTMX partial: ask form result.
pub async fn ask_partial(
    State(state): State<crate::server::AppState>,
    axum::extract::Query(form): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let (repo, pr, branch) = ask_query(&form);
    let v = |k: &str| fleet::esc(form.get(k).map(|s| s.trim()).unwrap_or_default());
    let (_, repos) = fleet::ask_picker(state.hub.as_ref());
    let live = ask_live_html(&state, &form);
    // the refresh asks the same question again, read back from the query
    let mut q = format!("repo={}", fleet::qv(&repo));
    if let Some(n) = pr {
        q += &format!("&amp;pr={n}");
    }
    if let Some(b) = &branch {
        q += &format!("&amp;branch={}", fleet::qv(b));
    }
    Html(format!(
        r##"<div class="overview">
  <h2>Ask</h2>
  <div class="ask-form">
  <h3>When will this PR run?</h3>
  <form hx-get="api/dashboard/ask-partial" hx-target="#content" hx-swap="innerHTML">
    <label>Repo: <input type="text" name="repo" placeholder="repository" value="{}" list="ask-repos" autocomplete="off" required></label>
    <label>PR: <input type="number" name="pr" placeholder="number" value="{}"></label>
    <label>or branch: <input type="text" name="branch" placeholder="branch" value="{}"></label>
    <button type="submit" class="btn">Estimate</button>
  </form>
  <datalist id="ask-repos">{repos}</datalist>
  </div>
  <div id="ask-live" hx-get="api/dashboard/ask-live?{q}" hx-trigger="every 30s [!document.hidden], refresh" hx-swap="innerHTML">{live}</div>
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
  {}
</div>"#,
        fleet::critical_chain(state.hub.as_ref(), &state.config.repo_dependencies),
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
    let gh = state.status.get_status().await.map_err(|e| e.to_string());
    Html(format!(
        r#"<div class="overview">
  <h2>Outages</h2>
  {}
  {}
  <section class="panel"><h3>Registered agents</h3>{agents_line}</section>
</div>"#,
        fleet::incidents(state.hub.as_ref())
            + &fleet::host_incidents(&fleet::host_detected(&state, crate::hub::now())),
        fleet::github_status(gh.as_ref().map_err(Clone::clone)),
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
  {}
  <p class="freshness">Rendered {}</p>
</div>"#,
        // REQ-OPS-003: what the operator still owes, first
        crate::operator::panel(&crate::operator::judge(&crate::operator::inputs(
            &state,
            crate::hub::now(),
        ))),
        total,
        healthy,
        unhealthy,
        fleet::health_cards(state.hub.as_ref()),
        fleet::health_view(state.hub.as_ref()),
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );

    Html(html).into_response()
}

/// HTMX partial: My hours, the signed-in user's attention schedule
/// (REQ-ATTN-001). The schedule is edited as its JSON, so a span's days or an
/// off period's note are never lost by a narrower form; the server validates
/// it and names a broken field. Signed-in only (login::SIGNED_IN).
pub async fn attention_partial(
    State(state): State<crate::server::AppState>,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some((session, who)) = state
        .login
        .as_ref()
        .and_then(|l| l.identity_of(&headers, crate::hub::now()))
    else {
        return Html(crate::login::sign_in_panel()).into_response();
    };
    let doc = state.attention.get(&who.sub).await.ok().flatten();
    let set = doc.is_some();
    let body = doc.unwrap_or_else(|| {
        serde_json::json!({
            "schema_version": 1,
            "timezone": "UTC",
            "windows": ["09:00-11:00", "16:00-17:00"],
            "available": ["08:00-18:00"]
        })
    });
    let pretty = serde_json::to_string_pretty(&body).unwrap_or_default();
    Html(format!(
        r#"<div class="overview my-hours"><h2>My hours</h2>
<p>Your attention schedule ({state_line}). Agents whose token names you with <code>--owner {sub}</code> read it at <code>GET /api/attention</code>; the night window is the time outside <code>available</code>.</p>
<form id="my-hours" data-csrf="{csrf}">
<textarea name="schedule" rows="16" spellcheck="false" aria-label="Attention schedule as JSON">{json}</textarea>
<p><button type="submit" class="btn-small">Save</button> <span class="my-hours-status" role="status"></span></p>
</form>
<script>
(function () {{
  var f = document.getElementById("my-hours");
  if (!f) return;
  f.addEventListener("submit", function (e) {{
    e.preventDefault();
    var s = f.querySelector(".my-hours-status");
    var body;
    try {{ body = JSON.parse(f.schedule.value); }} catch (x) {{ s.textContent = "Not valid JSON."; return; }}
    fetch("api/attention", {{method: "PUT", headers: {{"Content-Type": "application/json", "X-Warte-CSRF": f.dataset.csrf}}, body: JSON.stringify(body)}})
      .then(function (r) {{ return r.text().then(function (t) {{ s.textContent = r.ok ? "Saved." : t; }}); }})
      .catch(function () {{ s.textContent = "Could not reach warte."; }});
  }});
}})();
</script></div>"#,
        state_line = if set { "saved" } else { "not set yet: agents use their local file" },
        sub = fleet::esc(&who.sub),
        csrf = fleet::esc(&state.tokens.csrf_token(&session)),
        json = fleet::esc(&pretty),
    ))
    .into_response()
}

/// HTMX partial: the architecture view (REQ-ARCHVIEW-001). It shows where
/// every credential lives, so it renders data only to a signed-in user; the
/// login gate classes it signed-in. Without `[auth]` it says so and shows
/// nothing.
pub async fn architecture_partial(State(state): State<crate::server::AppState>) -> Response {
    if state.login.is_none() {
        return Html(r#"<div class="overview"><h2>Architecture</h2><p class="empty">The architecture view shows where credentials live, so it needs a login: configure <code>[auth]</code>. A script can read <code>GET /api/architecture</code> with a token holding <code>architecture:read</code>.</p></div>"#.to_string()).into_response();
    }
    let body = match crate::architecture::load(state.config.architecture.as_ref()) {
        None => r#"<div class="overview"><h2>Architecture</h2><p class="empty">No architecture file: set <code>[architecture] file</code> (<code>warte docs architecture-view</code>).</p></div>"#.to_string(),
        Some(Ok(c)) => crate::architecture::view(&c),
        Some(Err(errs)) => format!(
            r#"<div class="overview"><h2>Architecture</h2><p>The architecture file does not check out; nothing is shown until it does.</p><ul>{}</ul></div>"#,
            errs.iter().map(|e| format!("<li>{}</li>", fleet::esc(e))).collect::<String>()
        ),
    };
    Html(body).into_response()
}
