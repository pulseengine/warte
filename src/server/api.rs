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
        .route("/agents/{id}/github-token", post(api_github_token))
        .route("/agents/{id}/data", post(capture_data))
        .route("/access/grant", post(grant_access))
        .route("/access/revoke", post(revoke_access))
        // a workstation job replaces the notional-cost report whole (REQ-COSTS-001);
        // GET serves it. Larger than axum's 2 MB default: a report carries every session.
        .route(
            "/costs",
            get(api_costs)
                .post(api_costs_post)
                .layer(axum::extract::DefaultBodyLimit::max(crate::costs::MAX_BODY)),
        )
        // the briefing (REQ-BRIEF-001..003): the latest or one period, the
        // facts a model run reads, and its text, checked against them
        .route("/briefing", get(api_briefing).post(api_briefing_post))
        .route("/briefing/facts", get(api_briefing_facts))
        // issues people opened, for the run that replies (REQ-TRIAGE-001/002)
        .route("/triage", get(api_triage))
        .route("/triage/done", post(api_triage_done))
        .route("/news", get(api_news))
        .route("/north-star", get(api_north_star))
        .route("/news/sources", get(api_news_sources))
        .route("/news/state", post(api_news_state))
        // a workstation job replaces its source's skill-usage report whole
        // (REQ-SKILLS-001); GET serves the latest per source.
        .route(
            "/skill-usage",
            get(api_skill_usage).post(api_skill_usage_post).layer(
                axum::extract::DefaultBodyLimit::max(crate::skills::MAX_BODY),
            ),
        );

    // Read-only routes — public (rate-limited at edge proxy)
    let read_routes = Router::new()
        .route("/health", get(health))
        .route("/agents", get(list_agents))
        .route("/agents/{id}", get(get_agent))
        .route("/agents/{id}/data", get(query_data))
        .route("/access/list/{agent_id}", get(list_access))
        .route("/dashboard/overview", get(dashboard_overview))
        // Agents: machine-readable queue and PR admission (#55)
        .route("/queue", get(api_queue))
        .route("/admission", get(api_admission))
        .route("/watch", get(api_watch))
        .route("/eta", get(api_eta))
        .route("/history/fleet", get(api_history_fleet))
        .route("/status.md", get(api_status_md))
        .route("/report", get(api_report))
        .route("/report.md", get(api_report_md))
        .route("/machines", get(api_machines))
        .route("/history/machines", get(api_history_hosts))
        .route("/history/machines/{host}/metrics", get(api_history_metrics))
        .route("/history/machines/{host}/series", get(api_history_series))
        .route("/fleet", get(api_fleet))
        .route("/airport", get(api_airport))
        .route("/jobs", get(api_jobs))
        .route("/probe/enroll", post(api_probe_enroll))
        .route("/probe/report", post(api_probe_report))
        .route("/probe/renew", post(api_probe_renew))
        // temper's report as CCSDS packets: up to 1 MiB, base64 in JSON
        .route(
            "/probe/feed/temper",
            post(api_probe_feed_temper).layer(axum::extract::DefaultBodyLimit::max(3 << 20)),
        )
        // a host's security findings, the same signed transport (REQ-FINDINGS-001)
        .route(
            "/probe/feed/security",
            post(api_probe_feed_security).layer(axum::extract::DefaultBodyLimit::max(3 << 20)),
        )
        .route("/security/accept", post(api_security_accept))
        .route("/compliance", get(api_compliance))
        .route("/pressure", get(api_pressure))
        .route("/forecast", get(api_forecast))
        .route("/canaries", get(api_canaries))
        .route("/expiry", get(api_expiry))
        .route("/ci-health", get(api_ci_health))
        .route("/runner-usage", get(api_runner_usage))
        .route("/attention", get(api_attention_get).put(api_attention_put))
        .route("/architecture", get(api_architecture))
        .route("/glance", get(api_glance))
        .route("/reviews", get(api_reviews))
        .route(
            "/decisions/answer",
            axum::routing::post(api_decisions_answer),
        )
        .route(
            "/reports/seam-check",
            axum::routing::post(api_report_seam_check).layer(axum::extract::DefaultBodyLimit::max(
                crate::reports::MAX_BYTES + 1024,
            )),
        )
        .route("/incidents", get(api_incidents))
        .route("/operator", get(api_operator))
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
        .route("/idle.js", get(crate::dashboard::idle_js))
        .route(
            "/api/dashboard/overview-partial",
            get(crate::dashboard::overview_partial),
        )
        .route(
            "/api/dashboard/agents-partial",
            get(crate::dashboard::agents_partial),
        )
        .route(
            "/api/dashboard/ask-partial",
            // a read-only query: GET, so it needs no auth (REQ-API-002 covers mutations)
            get(crate::dashboard::ask_partial),
        )
        .route("/api/dashboard/ask-live", get(crate::dashboard::ask_live))
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
            "/api/dashboard/ci-history-partial",
            get(crate::dashboard::ci_history_partial),
        )
        .route("/airport.js", get(crate::dashboard::airport_js))
        .route(
            "/api/dashboard/briefing-partial",
            get(crate::dashboard::briefing_partial),
        )
        .route(
            "/api/dashboard/airport-partial",
            get(crate::dashboard::airport_partial),
        )
        .route(
            "/api/dashboard/jobs-partial",
            get(crate::dashboard::jobs_partial),
        )
        .route(
            "/api/dashboard/machines-partial",
            get(crate::dashboard::machines_partial),
        )
        .route(
            "/api/dashboard/machine-history-partial",
            get(crate::dashboard::machine_history_partial),
        )
        .route(
            "/api/dashboard/compliance-partial",
            get(crate::dashboard::compliance_partial),
        )
        .route("/api/dashboard/nav-state", get(crate::dashboard::nav_state))
        .route(
            "/api/dashboard/north-star-partial",
            get(crate::dashboard::north_star_partial),
        )
        .route("/api/dashboard/decisions-partial", get(decisions_partial))
        .route("/api/dashboard/seams-partial", get(seams_partial))
        .route("/api/dashboard/reviews-partial", get(reviews_partial))
        .route("/api/dashboard/security-partial", get(security_partial))
        .route(
            "/api/dashboard/architecture-partial",
            get(crate::dashboard::architecture_partial),
        )
        .route(
            "/api/dashboard/attention-partial",
            get(crate::dashboard::attention_partial),
        )
        .route(
            "/api/dashboard/costs-partial",
            get(crate::dashboard::costs_partial),
        )
        .route(
            "/api/dashboard/skills-partial",
            get(crate::dashboard::skills_partial),
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
        Ok((s, received)) => {
            let now = crate::hub::now();
            let mut v = crate::admission::queue(&s, received, now, repo);
            if let Some(hub) = state.hub.clone() {
                let repo = repo.map(String::from);
                v.not_acquired = tokio::task::spawn_blocking(move || {
                    not_acquired_week(&hub, now, repo.as_deref())
                })
                .await
                .unwrap_or_default();
            }
            Json(v).into_response()
        }
        Err(why) => plain(StatusCode::SERVICE_UNAVAILABLE, why),
    }
}

/// Not-acquired hosted jobs per pool and day, the last 7 UTC days (REQ-QUEUE-003).
pub(crate) fn not_acquired_week(
    hub: &crate::hub::Hub,
    now: i64,
    repo: Option<&str>,
) -> Vec<crate::requeue::PoolDay> {
    use crate::hub::report::day_of;
    let recs = hub.ledger_days(&day_of(now - 6 * 86_400), &day_of(now));
    crate::requeue::counts(&recs, repo)
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

/// When a PR's or branch's next job starts: the ground station's estimate
/// from measured waits (REQ-SMITHY-005).
async fn api_eta(
    State(state): State<AppState>,
    axum::extract::RawQuery(q): axum::extract::RawQuery,
) -> axum::response::Response {
    state
        .hub
        .as_ref()
        .map_or_else(no_hub, |h| h.eta_response(&q.unwrap_or_default()))
}

fn no_hub() -> axum::response::Response {
    plain(
        StatusCode::SERVICE_UNAVAILABLE,
        "no ground station configured",
    )
}

/// Per-minute fleet points, `hours=` 1 to 168 (REQ-SMITHY-006).
async fn api_history_fleet(
    State(state): State<AppState>,
    axum::extract::RawQuery(q): axum::extract::RawQuery,
) -> axum::response::Response {
    state
        .hub
        .as_ref()
        .map_or_else(no_hub, |h| h.history_response(&q.unwrap_or_default()))
}

/// Fleet status as Markdown, or one PR's start estimate (REQ-SMITHY-006).
async fn api_status_md(
    State(state): State<AppState>,
    axum::extract::RawQuery(q): axum::extract::RawQuery,
) -> axum::response::Response {
    state
        .hub
        .as_ref()
        .map_or_else(no_hub, |h| h.status_md_response(&q.unwrap_or_default()))
}

/// The monthly conveyor-belt report as JSON (REQ-SMITHY-006).
async fn api_report(
    State(state): State<AppState>,
    axum::extract::RawQuery(q): axum::extract::RawQuery,
) -> axum::response::Response {
    match state.hub.as_ref() {
        Some(h) => h.report_response(q, false).await,
        None => no_hub(),
    }
}

/// The monthly conveyor-belt report as Markdown (REQ-SMITHY-006).
async fn api_report_md(
    State(state): State<AppState>,
    axum::extract::RawQuery(q): axum::extract::RawQuery,
) -> axum::response::Response {
    match state.hub.as_ref() {
        Some(h) => h.report_response(q, true).await,
        None => no_hub(),
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

/// Report fields `/api/machines` does not publish as they are, each on
/// purpose (REQ-PROBE-008: every field is exposed or listed here, checked by
/// a test). `tools` and `sessions` are published reduced.
pub const HIDDEN_REPORT_FIELDS: [&str; 5] = [
    // reduced to name, installed, version (no paths, no error output)
    "tools",
    // reduced to a count per agent (no pids, no directories, no activity)
    "sessions",
    // reduced: binaries' origins instead of paths, a count of what was not read
    "inventory",
    // published without error text (a count of what was not read)
    "health",
    // published without error text (a count of what was not read)
    "hardware",
];

/// What `/api/machines` publishes of a report. The read API has no auth
/// (warte#46), so it leaves out what only helps an attacker map a machine:
/// tool paths, tool error output, process ids and working directories.
pub fn public_report(r: &crate::probe::Report) -> serde_json::Value {
    let mut sessions: std::collections::BTreeMap<&str, usize> = Default::default();
    for s in &r.sessions {
        *sessions.entry(s.agent.as_str()).or_default() += 1;
    }
    serde_json::json!({
        "host": r.host,
        "taken_at": r.taken_at,
        // REQ-PROBE-013: shown only when it is a plain version string
        "probe_version": crate::probe::version::clean(&r.probe_version),
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
        "io": r.io,
        "inventory": r.inventory.as_ref().map(crate::probe::inventory::public),
        "health": r.health.as_ref().map(crate::probe::health::public),
        "hardware": r.hardware.as_ref().map(crate::probe::hardware::public),
        // GB totals per kind per volume, no paths (REQ-PRESSURE-001)
        "pressure": r.pressure,
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
            "stale": now - s.received_at > crate::costs::STALE_S
                || s.report.data_lag_days().is_some(),
            // the newest day with data: a fresh report can carry old data
            "data_through": s.report.data_through(),
            "data_lag_days": s.report.data_lag_days(),
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

/// The triage queue: what the run may take now, and what waits.
async fn api_triage(State(state): State<AppState>) -> axum::response::Response {
    match state.triage.as_deref() {
        Some(t) => Json(t.offer(crate::hub::now())).into_response(),
        None => plain(StatusCode::SERVICE_UNAVAILABLE, "no [triage] configured"),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TriageDone {
    repo: String,
    number: u64,
    outcome: String,
}

/// The run is done with an issue (replied, asked or skipped).
async fn api_triage_done(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(t) = state.triage.as_deref() else {
        return plain(StatusCode::SERVICE_UNAVAILABLE, "no [triage] configured");
    };
    let d: TriageDone = match serde_json::from_slice(&body) {
        Ok(d) => d,
        Err(e) => {
            return plain(
                StatusCode::BAD_REQUEST,
                &format!(
                    "not a triage result ({:?} error at line {} column {})",
                    e.classify(),
                    e.line(),
                    e.column()
                ),
            )
        }
    };
    match t.done(&d.repo, d.number, &d.outcome, crate::hub::now()) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "done": true })),
        )
            .into_response(),
        Err(why) => plain(StatusCode::UNPROCESSABLE_ENTITY, &why),
    }
}

fn no_news() -> axum::response::Response {
    plain(StatusCode::SERVICE_UNAVAILABLE, "no [news] configured")
}

/// News items, newest first: `?since=YYYY-MM-DD&source=&grep=&state=` (news v1).
/// Exchange the agent's warte token for a GitHub App installation token
/// (REQ-GHTOKEN-001). The middleware has checked the right and that `{id}` is
/// the token's own agent; the broker checks the operator-set scope.
async fn api_github_token(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(b) = state.github_tokens.as_deref() else {
        return plain(
            StatusCode::NOT_FOUND,
            "no [github_app] configured on this warte",
        );
    };
    let req: crate::ghtoken::TokenRequest = if body.iter().all(u8::is_ascii_whitespace) {
        Default::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "the body is not a token request"})),
                )
                    .into_response()
            }
        }
    };
    match b.mint(&id, req, chrono::Utc::now().timestamp()).await {
        Ok(t) => (
            StatusCode::OK,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(t),
        )
            .into_response(),
        Err(r) => (
            StatusCode::from_u16(r.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({"error": r.why})),
        )
            .into_response(),
    }
}

/// The North Star's objectives with their published indicators (REQ-NS-001).
/// Public: the North Star and its indicator files are public.
async fn api_north_star(State(state): State<AppState>) -> axum::response::Response {
    let Some(n) = state.north_star.as_deref() else {
        return plain(
            StatusCode::NOT_FOUND,
            "no [north_star] configured on this warte",
        );
    };
    let now = crate::hub::now();
    let rows = n.rows(now);
    let last = n.last_poll.lock().map(|g| g.clone()).unwrap_or(None);
    Json(serde_json::json!({
        "rows": rows,
        "line": crate::northstar::glance_line(&rows),
        // REQ-NS-005: the published North Star, commitments and members
        "board": n.board(),
        "stale_after_days": n.config.stale_after_days,
        "last_poll": last.map(|(at, result)| serde_json::json!({"at": at, "result": result})),
    }))
    .into_response()
}

async fn api_news(
    State(state): State<AppState>,
    q: Result<axum::extract::Query<crate::news::Query>, axum::extract::rejection::QueryRejection>,
) -> axum::response::Response {
    let Some(n) = state.news.as_deref() else {
        return no_news();
    };
    let Ok(axum::extract::Query(q)) = q else {
        return plain(
            StatusCode::BAD_REQUEST,
            "the query takes since, source, grep and state only",
        );
    };
    match crate::news::query(&n.pool, &q).await {
        Ok(items) => Json(serde_json::json!({
            "version": crate::news::VERSION,
            "items": items,
        }))
        .into_response(),
        Err(why) => plain(StatusCode::BAD_REQUEST, &why),
    }
}

/// Each feed's last poll.
async fn api_news_sources(State(state): State<AppState>) -> axum::response::Response {
    let Some(n) = state.news.as_deref() else {
        return no_news();
    };
    let st = n.status.lock().map(|s| s.clone()).unwrap_or_default();
    Json(serde_json::json!({ "version": crate::news::VERSION, "sources": st })).into_response()
}

/// A reader marks an item found, decided, landed or dismissed.
async fn api_news_state(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(n) = state.news.as_deref() else {
        return no_news();
    };
    let c: crate::news::StateChange = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => {
            return plain(
                StatusCode::BAD_REQUEST,
                &format!("not a state change ({:?} error)", e.classify()),
            )
        }
    };
    match crate::news::set_state(&n.pool, &c, &crate::news::now()).await {
        Ok(()) => Json(serde_json::json!({ "id": c.id, "state": c.state })).into_response(),
        Err(why) => plain(StatusCode::UNPROCESSABLE_ENTITY, &why),
    }
}

fn no_briefing() -> axum::response::Response {
    plain(StatusCode::SERVICE_UNAVAILABLE, "no [briefing] configured")
}

fn period_query(q: &Option<String>) -> Option<String> {
    q.as_deref()?
        .split('&')
        .find_map(|kv| kv.strip_prefix("from="))
        .map(String::from)
}

/// The latest briefing (or `?from=YYYY-MM-DD`), and the periods kept.
async fn api_briefing(
    State(state): State<AppState>,
    axum::extract::RawQuery(q): axum::extract::RawQuery,
) -> axum::response::Response {
    let Some(s) = state.briefing.as_deref() else {
        return no_briefing();
    };
    let b = match period_query(&q) {
        Some(p) => match s.get(&p) {
            Some(b) => Some(b),
            None => return plain(StatusCode::NOT_FOUND, "no briefing for that period"),
        },
        None => s.latest(),
    };
    Json(serde_json::json!({ "briefing": b, "periods": s.periods() })).into_response()
}

/// The facts of the latest period, for the model run that writes the text.
async fn api_briefing_facts(State(state): State<AppState>) -> axum::response::Response {
    let Some(s) = state.briefing.as_deref() else {
        return no_briefing();
    };
    match s.latest() {
        Some(b) => Json(serde_json::json!({
            "from": b.facts.from,
            "until": b.facts.until,
            "source": b.source,
            "replaces": b.replaces,
            "facts": b.facts,
            "rules": {
                "bullets": 3,
                "max_words": crate::briefing::MAX_WORDS,
                "only_from_facts": "name only repositories, #numbers, versions and counts that are in the facts",
            },
        }))
        .into_response(),
        None => plain(StatusCode::NOT_FOUND, "no briefing yet"),
    }
}

/// A model run's text for a period: kept when every repository, number and
/// version in it is in that period's facts; otherwise 422 and the template
/// stays (REQ-BRIEF-002).
async fn api_briefing_post(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(s) = state.briefing.as_deref() else {
        return no_briefing();
    };
    let sub: crate::briefing::Submission = match serde_json::from_slice(&body) {
        Ok(x) => x,
        Err(e) => {
            return plain(
                StatusCode::BAD_REQUEST,
                &format!(
                    "not a briefing ({:?} error at line {} column {})",
                    e.classify(),
                    e.line(),
                    e.column()
                ),
            )
        }
    };
    match s.submit(sub, crate::hub::now()) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "stored": true })),
        )
            .into_response(),
        Err(why) => plain(StatusCode::UNPROCESSABLE_ENTITY, &why),
    }
}

/// The latest skill-usage report of every source; `reports` is empty before
/// the first push.
async fn api_skill_usage(State(state): State<AppState>) -> axum::response::Response {
    let Some(s) = state.skills.as_ref() else {
        return plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "no ground station configured",
        );
    };
    let now = crate::hub::now();
    let reports: Vec<_> = s
        .all()
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "source": r.report.source,
                "received_at": r.received_at,
                "age_s": now - r.received_at,
                "stale": now - r.received_at > crate::skills::STALE_S,
                "report": r.report,
            })
        })
        .collect();
    Json(serde_json::json!({ "reports": reports, "rejected": s.rejected() })).into_response()
}

/// Replace the skill-usage report of its source. The read API is public
/// (warte#46), so a report must hold counts and names only: unknown fields and
/// free text are rejected with a 400, nothing is stored, and the reason never
/// quotes the input.
async fn api_skill_usage_post(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(s) = state.skills.as_ref() else {
        return plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "no ground station configured",
        );
    };
    let report = match crate::skills::parse(&body) {
        Ok(r) => r,
        Err(why) => return plain(StatusCode::BAD_REQUEST, &why),
    };
    let (source, skills) = (report.source.clone(), report.skills.len());
    match s.record(report, crate::hub::now()) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "stored": true, "source": source, "skills": skills })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to store the skill-usage report");
            plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not store the report",
            )
        }
    }
}

/// Accept a signed report from an enrolled probe (REQ-PROBE-006). One answer
/// for every refusal; the reason goes to the server log.
async fn api_probe_report(
    State(state): State<AppState>,
    Json(body): Json<crate::probe::send::SignedReport>,
) -> axum::response::Response {
    let refused = || plain(StatusCode::FORBIDDEN, "report refused");
    let Some(machines) = state.machines.clone() else {
        return plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "machine supervision is off",
        );
    };
    if body.report.len() > 256 * 1024 || body.cert.len() > 16 * 1024 || body.sig.len() > 512 {
        return refused();
    }
    let now = crate::hub::now();
    let v = match crate::probe::send::verify(state.ca.ca_cert_der(), &body, now) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "probe report refused");
            return refused();
        }
    };
    match crate::probe::enroll::revoked(&state.db, &v.report.host, v.issued_at).await {
        Ok(false) => {}
        Ok(true) => {
            tracing::warn!(host = %v.report.host, "probe report refused: host revoked");
            return refused();
        }
        Err(e) => {
            tracing::error!(error = %e, "probe report: revocation store");
            return plain(StatusCode::SERVICE_UNAVAILABLE, "reports unavailable");
        }
    }
    let last = machines
        .list()
        .into_iter()
        .find(|m| m.report.host.eq_ignore_ascii_case(&v.report.host) && m.via == "probe")
        .map(|m| m.report.taken_at);
    if last.is_some_and(|t| v.report.taken_at <= t) {
        tracing::warn!(host = %v.report.host, "probe report refused: not newer than the last (replay?)");
        return refused();
    }
    machines.record_cert(v.report, "probe", now, Some(v.cert_until));
    StatusCode::NO_CONTENT.into_response()
}

/// Renew a probe's certificate (REQ-PROBE-007): no token, the current
/// certificate and its key vouch for the new CSR. An expired or revoked
/// certificate is refused; that machine enrolls again.
async fn api_probe_renew(
    State(state): State<AppState>,
    Json(req): Json<crate::probe::send::RenewRequest>,
) -> axum::response::Response {
    use crate::probe::enroll;
    let refused = || plain(StatusCode::FORBIDDEN, "renewal refused");
    if req.cert.len() > 16 * 1024 || req.csr.len() > 16 * 1024 || req.sig.len() > 512 {
        return refused();
    }
    let now = crate::hub::now();
    let r = match crate::probe::send::verify_renew(state.ca.ca_cert_der(), &req, now) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "probe renewal refused");
            return refused();
        }
    };
    match enroll::revoked(&state.db, &r.host, r.issued_at).await {
        Ok(false) => {}
        Ok(true) => {
            tracing::warn!(host = %r.host, "probe renewal refused: host revoked");
            return refused();
        }
        Err(e) => {
            tracing::error!(error = %e, "probe renewal: revocation store");
            return plain(StatusCode::SERVICE_UNAVAILABLE, "renewal unavailable");
        }
    }
    match state
        .ca
        .sign_probe_csr(&req.csr, &r.host, enroll::CERT_DAYS)
        .await
    {
        Ok(cert) => {
            tracing::info!(host = %r.host, "probe certificate renewed");
            Json(enroll::EnrollResponse {
                cert,
                ca: state.ca.ca_cert_pem().to_string(),
                spi: None,
                sa_key: None,
            })
            .into_response()
        }
        Err(e) => {
            tracing::warn!(host = %r.host, error = %e, "probe renewal: bad certificate request");
            plain(StatusCode::BAD_REQUEST, "not a valid certificate request")
        }
    }
}

/// temper's report, forwarded by the configured host's probe as a signed CCSDS
/// sequence (REQ-TEMPER-002). One answer for every refusal; the reason goes
/// to the server log, never the input.
async fn api_probe_feed_temper(
    State(state): State<AppState>,
    Json(f): Json<crate::probe::send::SignedFeed>,
) -> axum::response::Response {
    let refused = || plain(StatusCode::FORBIDDEN, "feed refused");
    let (Some(store), Some(cfg)) = (state.temper.clone(), state.config.temper.as_ref()) else {
        return plain(StatusCode::SERVICE_UNAVAILABLE, "no temper feed configured");
    };
    if f.name != "temper" || f.cert.len() > 16 * 1024 || f.sig.len() > 512 {
        return refused();
    }
    let now = crate::hub::now();
    let v = match crate::probe::send::verify_feed(state.ca.ca_cert_der(), &f, now) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "temper feed refused");
            return refused();
        }
    };
    if !v.host.eq_ignore_ascii_case(&cfg.host) {
        tracing::warn!(host = %v.host, "temper feed refused: not from the configured host");
        return refused();
    }
    match crate::probe::enroll::revoked(&state.db, &v.host, v.issued_at).await {
        Ok(false) => {}
        Ok(true) => {
            tracing::warn!(host = %v.host, "temper feed refused: host revoked");
            return refused();
        }
        Err(e) => {
            tracing::error!(error = %e, "temper feed: revocation store");
            return plain(StatusCode::SERVICE_UNAVAILABLE, "feed unavailable");
        }
    }
    let report = match crate::temper::report::parse(&v.data) {
        Ok(r) => r,
        Err(why) => {
            tracing::warn!(%why, "temper feed refused: report");
            return plain(
                StatusCode::UNPROCESSABLE_ENTITY,
                &format!("report rejected: {why}"),
            );
        }
    };
    match store.record(crate::temper::Stored {
        received_at: now,
        at: f.at,
        host: v.host,
        report,
    }) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => {
            tracing::warn!("temper feed refused: not newer than the last (replay?)");
            refused()
        }
        Err(e) => {
            tracing::error!(error = %e, "temper feed: store");
            plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not store the report",
            )
        }
    }
}

/// The latest temper report and how old it is (REQ-TEMPER-002).
async fn api_compliance(State(state): State<AppState>) -> axum::response::Response {
    let Some(store) = state.temper.as_ref() else {
        return Json(serde_json::json!({"configured": false})).into_response();
    };
    let now = crate::hub::now();
    let stale_after = state
        .config
        .temper
        .as_ref()
        .map(|t| t.stale_after_s)
        .unwrap_or(26 * 3600);
    match store.latest() {
        None => Json(serde_json::json!({"configured": true, "report": null})).into_response(),
        Some(s) => {
            let sweep_age = s
                .report
                .source
                .last_sweep_at
                .as_deref()
                .and_then(crate::temper::report::time)
                .map(|t| now - t);
            Json(serde_json::json!({
                "configured": true,
                "received_at": s.received_at,
                "host": s.host,
                "sweep_age_s": sweep_age,
                "stale": sweep_age.is_none_or(|a| a > stale_after),
                "failures": s.report.failures().len(),
                "report": s.report,
            }))
            .into_response()
        }
    }
}

/// Enroll a probe: a one-time token for this host, and a CSR (DD-PROBE-002).
/// One answer for every refusal, so a caller learns nothing about tokens.
async fn api_probe_enroll(
    State(state): State<AppState>,
    Json(req): Json<crate::probe::enroll::EnrollRequest>,
) -> axum::response::Response {
    use crate::probe::enroll;
    let refused = || plain(StatusCode::FORBIDDEN, "enrollment refused");
    if !enroll::valid_host(&req.host) || req.token.len() > 200 || req.csr.len() > 16 * 1024 {
        return refused();
    }
    match enroll::consume(&state.db, &req.token, &req.host, crate::hub::now()).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(host = %req.host, "probe enrollment refused: token unknown, used, expired or for another host");
            return refused();
        }
        Err(e) => {
            tracing::error!(error = %e, "probe enrollment: token store");
            return plain(StatusCode::SERVICE_UNAVAILABLE, "enrollment unavailable");
        }
    }
    match state
        .ca
        .sign_probe_csr(&req.csr, &req.host, enroll::CERT_DAYS)
        .await
    {
        Ok(cert) => {
            // a fresh security association for transport v2 (REQ-SEC-001);
            // enrolling again replaces the old one
            let sa = crate::hub::quic::SaStore::new(state.db.pool().clone())
                .issue(&req.host)
                .await;
            let (spi, sa_key) = match sa {
                Ok((s, k)) => (Some(s), Some(k)),
                Err(e) => {
                    tracing::error!(error = %e, "probe enrollment: security association");
                    return plain(StatusCode::SERVICE_UNAVAILABLE, "enrollment unavailable");
                }
            };
            tracing::info!(host = %req.host, "probe enrolled");
            Json(enroll::EnrollResponse {
                cert,
                ca: state.ca.ca_cert_pem().to_string(),
                spi,
                sa_key,
            })
            .into_response()
        }
        Err(e) => {
            tracing::warn!(host = %req.host, error = %e, "probe enrollment: bad certificate request");
            plain(StatusCode::BAD_REQUEST, "not a valid certificate request")
        }
    }
}

/// The airport map's data (REQ-DASH-015).
async fn api_airport(State(state): State<AppState>) -> axum::response::Response {
    let fleet = state.fleet.as_deref().and_then(|r| r.as_ref().ok());
    let machines = state
        .machines
        .as_ref()
        .map(|m| m.list())
        .unwrap_or_default();
    let hub = state.hub.as_ref().and_then(|h| h.snapshot());
    let jobs = jobs_now(&state).unwrap_or_default();
    Json(crate::dashboard::airport::build(
        fleet,
        &machines,
        hub,
        &jobs,
        crate::hub::now(),
    ))
    .into_response()
}

/// The jobs now, with job host names mapped to machine names
/// ([jobs] hosts); None when no jobs directory is configured.
pub fn jobs_now(state: &AppState) -> Option<Vec<crate::jobs::Job>> {
    let cfg = state.config.jobs.as_ref()?;
    let manifest = cfg
        .expected
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let mut jobs = crate::jobs::read(&cfg.dir, manifest.as_deref(), crate::hub::now());
    for j in &mut jobs {
        if let Some(m) = cfg.hosts.get(&j.host) {
            j.host = m.clone();
        }
    }
    Some(jobs)
}

/// Scheduled jobs (REQ-JOBS-001), worst first, with the count that need you.
async fn api_jobs(State(state): State<AppState>) -> axum::response::Response {
    match jobs_now(&state) {
        None => Json(serde_json::json!({ "configured": false, "jobs": [], "overdue": 0 })),
        Some(jobs) => {
            let overdue = jobs.iter().filter(|j| j.bad()).count();
            Json(serde_json::json!({ "configured": true, "jobs": jobs, "overdue": overdue }))
        }
    }
    .into_response()
}

/// The hosts reporting now: probes, and the CI host's agent.
pub fn reporting_hosts(state: &AppState) -> Vec<String> {
    let mut hosts: Vec<String> = state
        .machines
        .as_ref()
        .map(|m| m.list().into_iter().map(|s| s.report.host).collect())
        .unwrap_or_default();
    if let Some((s, _)) = state.hub.as_ref().and_then(|h| h.snapshot()) {
        if !s.host.is_empty() && !hosts.contains(&s.host) {
            hosts.push(s.host);
        }
    }
    hosts
}

/// The fleet model and its differences from what reports in (REQ-FLEET-001).
async fn api_fleet(State(state): State<AppState>) -> axum::response::Response {
    match state.fleet.as_deref() {
        None => plain(
            StatusCode::NOT_FOUND,
            "no fleet model configured (fleet_model / WARTE_FLEET_MODEL)",
        ),
        Some(Err(e)) => plain(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("fleet model unreadable: {e}"),
        ),
        Some(Ok(f)) => Json(serde_json::json!({
            "model": f,
            "differences": crate::fleet::differences(f, &reporting_hosts(&state)),
        }))
        .into_response(),
    }
}

// ── Machine history (REQ-MHIST-002) ────────────────────────────────

fn history(state: &AppState) -> Option<&crate::history::History> {
    state.machines.as_ref().and_then(|m| m.history())
}

fn no_history() -> axum::response::Response {
    plain(StatusCode::NOT_FOUND, "no machine history on this warte")
}

/// A host the store knows. Hosts are keys, never paths.
fn known_host(h: &crate::history::History, host: &str) -> bool {
    h.hosts().iter().any(|k| k == host)
}

fn unknown_host() -> axum::response::Response {
    plain(StatusCode::NOT_FOUND, "no history for this machine")
}

async fn api_history_hosts(State(state): State<AppState>) -> axum::response::Response {
    match history(&state) {
        Some(h) => Json(serde_json::json!({ "machines": h.hosts() })).into_response(),
        None => no_history(),
    }
}

async fn api_history_metrics(
    State(state): State<AppState>,
    axum::extract::Path(host): axum::extract::Path<String>,
) -> axum::response::Response {
    let Some(h) = history(&state) else {
        return no_history();
    };
    if !known_host(h, &host) {
        return unknown_host();
    }
    let metrics: Vec<serde_json::Value> = h
        .metrics(&host)
        .into_iter()
        .map(|m| serde_json::json!({ "unit": crate::history::unit(&m), "metric": m }))
        .collect();
    Json(serde_json::json!({ "host": host, "metrics": metrics })).into_response()
}

#[derive(Deserialize)]
struct SeriesQuery {
    metric: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
}

/// `from`/`to` defaults and limits, shared with the dashboard. Err: a 400 message.
pub fn history_range(
    from: Option<i64>,
    to: Option<i64>,
    now: i64,
) -> Result<(i64, i64), &'static str> {
    // times are unix seconds between 1970 and a day from now; checked before
    // any arithmetic, so no request can overflow
    let ok = |t: i64| (0..=now + 86_400).contains(&t);
    if !from.is_none_or(ok) || !to.is_none_or(ok) {
        return Err("from and to are unix seconds between 0 and now");
    }
    let to = to.unwrap_or(now);
    let from = from.unwrap_or((to - 86_400).max(0));

    if from > to {
        return Err("from must not be after to");
    }
    if to - from > crate::history::MAX_RANGE_S {
        return Err("the range is limited to 400 days");
    }
    Ok((from, to))
}

async fn api_history_series(
    State(state): State<AppState>,
    axum::extract::Path(host): axum::extract::Path<String>,
    Query(q): Query<SeriesQuery>,
) -> axum::response::Response {
    let Some(h) = history(&state) else {
        return no_history();
    };
    if !known_host(h, &host) {
        return unknown_host();
    }
    let metrics: Vec<String> = q
        .metric
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(String::from)
        .collect();
    if metrics.is_empty() || metrics.len() > 32 {
        return plain(StatusCode::BAD_REQUEST, "give 1 to 32 metrics: metric=a,b");
    }
    let (from, to) = match history_range(q.from, q.to, crate::hub::now()) {
        Ok(r) => r,
        Err(m) => return plain(StatusCode::BAD_REQUEST, m),
    };
    let h = h.clone();
    let host2 = host.clone();
    match tokio::task::spawn_blocking(move || h.series(&host2, &metrics, from, to)).await {
        Ok(series) => {
            Json(serde_json::json!({ "host": host, "from": from, "to": to, "series": series }))
                .into_response()
        }
        Err(_) => plain(StatusCode::INTERNAL_SERVER_ERROR, "history read failed"),
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
                "cert_until": s.cert_until,
                "report": public_report(&s.report),
            })
        })
        .collect();
    let mut findings = m.findings(now);
    findings.extend(m.history().and_then(|h| h.finding(now)));
    Json(serde_json::json!({
        "machines": machines,
        "findings": findings,
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

/// When the server state was built; `/api/health` reports the time since.
static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Record the server's start (the first call wins).
pub(crate) fn mark_started() {
    STARTED.get_or_init(std::time::Instant::now);
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "healthy".to_string(),
        instance_id: state.config.instance_id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: STARTED.get().map_or(0, |t| t.elapsed().as_secs()),
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

/// A heartbeat, optionally with `{"status": "ended" | "idle" | "working"}`
/// (seam warte-plugin/heartbeat-status, REQ-GLANCE-001); a bodyless beat is
/// as before. Any other body is 422.
async fn agent_heartbeat(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // Older clients send `{}` or other JSON with a beat: anything without a
    // `status` field is a beat as before. Only a `status` outside the three
    // known values is refused.
    let status = match serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("status").cloned())
    {
        None => None,
        Some(serde_json::Value::String(s))
            if ["ended", "idle", "working"].contains(&s.as_str()) =>
        {
            Some(s)
        }
        Some(_) => return StatusCode::UNPROCESSABLE_ENTITY,
    };
    match state.agents.heartbeat_with(&id, status.as_deref()).await {
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
    // No such agent is a 404, not the foreign-key failure's 500.
    match state.agents.get(&agent_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "unknown agent" })),
            )
        }
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "internal error" })),
            )
        }
    }
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

// ── Dashboard ───────────────────────────────────────────────────────

async fn dashboard_overview(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.agents.list().await.unwrap_or_default();

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
        "freshness": "green", // TODO: compute from actual data age
    }))
}

/// Whether a host's disks are low (REQ-PRESSURE-001): a verdict per volume
/// from its latest report, the host's being the worst, with the GB agents
/// hold there as totals per kind. No names or paths (DD-PRESSURE-001).
/// Which operator duties are met, late or not set up (REQ-OPS-003).
async fn api_operator(State(state): State<AppState>) -> Json<serde_json::Value> {
    let now = crate::hub::now();
    Json(serde_json::json!({
        "generated_at": now,
        "duties": crate::operator::judge(&crate::operator::inputs(&state, now)),
    }))
}

/// What is forecast to reach its limit (REQ-PREDICT-001).
async fn api_forecast(State(state): State<AppState>) -> axum::response::Response {
    match state
        .machines
        .as_ref()
        .and_then(|m| crate::forecast::of_machines(m, crate::hub::now()))
    {
        Some(f) => Json(f).into_response(),
        None => plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "no ground station with history configured",
        ),
    }
}

/// Do the names CI depends on resolve (REQ-PREDICT-003)?
async fn api_canaries(State(state): State<AppState>) -> axum::response::Response {
    match &state.canaries {
        None => plain(StatusCode::SERVICE_UNAVAILABLE, "no canaries configured"),
        Some(c) => match c.latest() {
            Some(r) => Json(serde_json::json!({
                "report": r,
                "findings": crate::canary::judge(&r, crate::hub::now())
                    .iter()
                    .map(|d| serde_json::json!({"key": d.key, "sev": d.sev, "text": d.text}))
                    .collect::<Vec<_>>(),
            }))
            .into_response(),
            None => plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "the first canary round has not finished",
            ),
        },
    }
}

/// What runs out and when (REQ-PREDICT-004).
async fn api_expiry(State(state): State<AppState>) -> axum::response::Response {
    let now = crate::hub::now();
    let r = state.expiry.report(&state.tokens.seen(now));
    let findings: Vec<_> = crate::expiry::detected(&r, now)
        .iter()
        .map(|d| serde_json::json!({"key": d.key, "sev": d.sev, "text": d.text}))
        .collect();
    Json(serde_json::json!({"report": r, "findings": findings})).into_response()
}

/// CI health trends, this week against the week before (REQ-PREDICT-005).
async fn api_ci_health(State(state): State<AppState>) -> axum::response::Response {
    match &state.hub {
        Some(h) => Json(
            state
                .ci_health
                .get(h, crate::hub::now())
                .await
                .as_ref()
                .clone(),
        )
        .into_response(),
        None => plain(StatusCode::SERVICE_UNAVAILABLE, "no CI source configured"),
    }
}

/// The attention schedule (REQ-ATTN-001, warte#181): a signed-in user reads
/// their own; a warte token with `agents:read` reads its owner's (`--owner`);
/// 404 when none is set or the token names no owner, 401 otherwise.
async fn api_attention_get(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let now = crate::hub::now();
    let subject = if let Some((_, who)) = state
        .login
        .as_ref()
        .and_then(|l| l.identity_of(&headers, now))
    {
        Some(who.sub)
    } else {
        let bearer = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        let Some(p) = bearer.and_then(|b| state.tokens.verify_token(b).ok()) else {
            return plain(StatusCode::UNAUTHORIZED, "sign in, or use an agent token");
        };
        if !p.rights.iter().any(|r| r == crate::auth::RIGHT_AGENTS_READ) {
            return plain(StatusCode::FORBIDDEN, "the token needs agents:read");
        }
        p.owner
    };
    let Some(subject) = subject else {
        return plain(StatusCode::NOT_FOUND, "no attention schedule");
    };
    match state.attention.get(&subject).await {
        Ok(Some(doc)) => Json(doc).into_response(),
        Ok(None) => plain(StatusCode::NOT_FOUND, "no attention schedule"),
        Err(e) => {
            tracing::warn!("attention: {e}");
            plain(StatusCode::INTERNAL_SERVER_ERROR, "database error")
        }
    }
}

/// Is the request's Origin this server (same scheme-less authority as Host)?
fn same_origin(h: &axum::http::HeaderMap) -> bool {
    let get = |k: axum::http::header::HeaderName| h.get(k).and_then(|v| v.to_str().ok());
    let (Some(origin), Some(host)) = (
        get(axum::http::header::ORIGIN),
        get(axum::http::header::HOST),
    ) else {
        return false;
    };
    origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .is_some_and(|authority| authority == host)
}

/// Replace the signed-in user's own attention schedule (REQ-ATTN-001). Only
/// with a login: from this server's origin, with the session's CSRF token in
/// `X-Warte-CSRF`. Logged as who changed which fields, never the values.
async fn api_attention_put(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(login) = state.login.as_ref() else {
        return plain(StatusCode::FORBIDDEN, "editing a schedule needs [auth]");
    };
    let Some((session, who)) = login.identity_of(&headers, crate::hub::now()) else {
        return plain(StatusCode::UNAUTHORIZED, "sign in first");
    };
    let csrf = headers.get("x-warte-csrf").and_then(|v| v.to_str().ok());
    if !same_origin(&headers) || csrf != Some(state.tokens.csrf_token(&session).as_str()) {
        return plain(StatusCode::FORBIDDEN, "not from this page");
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return plain(StatusCode::UNPROCESSABLE_ENTITY, "the body is not JSON");
    };
    match state.attention.put(&who.sub, &v).await {
        Ok(doc) => {
            let fields: Vec<&str> = doc
                .as_object()
                .map(|m| m.keys().map(String::as_str).collect())
                .unwrap_or_default();
            tracing::info!(user = %who.name, ?fields, "attention schedule changed");
            Json(doc).into_response()
        }
        Err(e) if e.starts_with("database error") => {
            tracing::warn!("attention: {e}");
            plain(StatusCode::INTERNAL_SERVER_ERROR, "database error")
        }
        Err(e) => plain(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}

/// Runner usage of one repository over a window of ledger days (REQ-CI-009):
/// runner-minutes and runner wait per pool, with no runner names.
async fn api_runner_usage(
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    let Some(hub) = state.hub.clone() else {
        return plain(StatusCode::SERVICE_UNAVAILABLE, "no CI source configured");
    };
    let arg = |k: &str| q.get(k).cloned().unwrap_or_default();
    let (repo, from, until) = (arg("repo"), arg("from"), arg("until"));
    if let Err(e) = crate::cihealth::usage::check(&repo, &from, &until) {
        return plain(StatusCode::BAD_REQUEST, &e);
    }
    // Only public repositories are answered, from the same `type=public`
    // listing the attention strip uses. Private, archived and unknown names
    // get one answer, so the endpoint never confirms that a repository exists.
    // No briefing yet means no public list, so nothing is answered.
    let public = state
        .briefing
        .as_ref()
        .and_then(|s| s.latest())
        .is_some_and(|b| b.facts.known_repos.iter().any(|r| r == &repo));
    if !public {
        return plain(StatusCode::NOT_FOUND, "no such public repository");
    }
    let (f, u) = (from.clone(), until.clone());
    let recs = tokio::task::spawn_blocking(move || hub.ledger_days(&f, &u))
        .await
        .unwrap_or_default();
    Json(crate::cihealth::usage::usage(&recs, &repo, &from, &until)).into_response()
}

/// Shared incidents: one error failing jobs in several repositories (REQ-CI-005).
async fn api_incidents(State(state): State<AppState>) -> axum::response::Response {
    match (&state.hub, &state.incidents) {
        (Some(h), Some(i)) => {
            let r = match i.latest() {
                Some(r) => r,
                None => i.refresh(h, crate::hub::now()).await,
            };
            Json(r.as_ref().clone()).into_response()
        }
        _ => plain(StatusCode::SERVICE_UNAVAILABLE, "no CI source configured"),
    }
}

async fn api_pressure(
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use crate::probe::pressure::{free_gb, worst, Verdict};
    let Some(m) = state.machines.as_ref() else {
        return plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "no ground station configured",
        );
    };
    let seen = m.list();
    let Some(host) = q.get("host").filter(|h| !h.is_empty()) else {
        let hosts: Vec<&str> = seen.iter().map(|s| s.report.host.as_str()).collect();
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "host is required", "hosts": hosts})),
        )
            .into_response();
    };
    let t = state.config.pressure;
    let now = crate::hub::now();
    let Some(s) = seen.iter().find(|s| &s.report.host == host) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "unknown host",
                "hosts": seen.iter().map(|s| s.report.host.as_str()).collect::<Vec<_>>(),
            })),
        )
            .into_response();
    };
    let r = &s.report;
    let stale = now - s.received_at > crate::probe::MACHINE_STALE_S;
    let held = |mount: &str| {
        r.pressure
            .as_ref()
            .and_then(|p| p.volumes.iter().find(|v| v.mount == mount))
            .map(|v| {
                serde_json::json!({
                    "scratchpad": v.scratchpad_gb,
                    "target": v.target_gb,
                    "worktree": v.worktree_gb,
                })
            })
    };
    let vols: Vec<(Verdict, serde_json::Value, f64, &str)> = r
        .disks
        .iter()
        .map(|d| {
            let free = free_gb(d);
            let v = if stale {
                Verdict::Unknown
            } else {
                t.verdict(free)
            };
            (
                v,
                serde_json::json!({
                    "mount": d.mount,
                    "free_gb": free,
                    "total_gb": d.size_gb,
                    "verdict": v,
                    "held_gb": held(&d.mount),
                }),
                free,
                d.mount.as_str(),
            )
        })
        .collect();
    let verdict = if stale {
        Verdict::Unknown
    } else {
        worst(vols.iter().map(|v| v.0))
    };
    let reason = if stale {
        format!(
            "no report from {host} for {} min",
            (now - s.received_at) / 60
        )
    } else {
        match vols
            .iter()
            .filter(|v| v.0 == verdict)
            .min_by(|a, b| a.2.total_cmp(&b.2))
        {
            Some((_, _, free, mount)) if verdict != Verdict::Ok => format!(
                "{mount} has {free} GB free (low under {} GB, critical under {} GB)",
                t.low_gb, t.critical_gb
            ),
            _ => format!("every volume has at least {} GB free", t.low_gb),
        }
    };
    Json(serde_json::json!({
        "host": r.host,
        "verdict": verdict,
        "reason": reason,
        "measured_at": r.taken_at,
        "held_measured_at": r.pressure.as_ref().map(|p| p.measured_at),
        "recheck_s": verdict.recheck_s(),
        "volumes": vols.into_iter().map(|v| v.1).collect::<Vec<_>>(),
    }))
    .into_response()
}

/// The architecture view's data (REQ-ARCHVIEW-001): with a login, or without
/// one for a token holding `architecture:read` (TOKEN_READ_ROUTES). 404
/// without `[architecture]`; 422 with the errors when the file does not check.
async fn api_architecture(State(state): State<AppState>) -> axum::response::Response {
    match crate::architecture::load(state.config.architecture.as_ref()) {
        None => plain(StatusCode::NOT_FOUND, "no [architecture] file"),
        Some(Ok(c)) => Json(c).into_response(),
        Some(Err(errs)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "errors": errs })),
        )
            .into_response(),
    }
}

/// Everything the night glance reads, now (REQ-GLANCE-001).
async fn glance_now(state: &AppState) -> crate::glance::Glance {
    use crate::probe::pressure::{free_gb, Verdict};
    let now = crate::hub::now();
    let notices: Vec<serde_json::Value> = sqlx::query_as::<_, (String,)>(
        "SELECT content FROM agent_data WHERE data_type = 'notice.v1' ORDER BY captured_at DESC LIMIT 5000",
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap_or_default()
    .into_iter()
    .filter_map(|(c,)| serde_json::from_str(&c).ok())
    .collect();
    let red_mains = crate::dashboard::red_runs_now(state, now)
        .await
        .into_iter()
        .filter(|r| r.kind == "main")
        .map(|r| (r.repo, r.workflow))
        .collect();
    let failed_releases = match state.hub.clone() {
        Some(hub) => {
            let day = |t: i64| crate::hub::report::day_of(t);
            let (from, until) = (day(now - 7 * 86_400), day(now + 86_400));
            let recs = tokio::task::spawn_blocking(move || hub.ledger_days(&from, &until))
                .await
                .unwrap_or_default();
            crate::glance::failed_releases(&recs)
        }
        None => vec![],
    };
    let t = state.config.pressure;
    let critical_disks = state
        .machines
        .as_ref()
        .map(|m| m.list())
        .unwrap_or_default()
        .into_iter()
        .filter(|s| now - s.received_at <= crate::probe::MACHINE_STALE_S)
        .filter(|s| {
            s.report
                .disks
                .iter()
                .any(|d| t.verdict(free_gb(d)) == Verdict::Critical)
        })
        .map(|s| s.report.host.clone())
        .collect();
    let heartbeats = state
        .agents
        .list()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|a| a.last_heartbeat.map(|t| t.timestamp()))
        .collect();
    let always_open = state
        .config
        .glance
        .as_ref()
        .map(|g| g.always_open.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| crate::glance::ALWAYS_OPEN.map(String::from).to_vec());
    let findings = crate::findings::Store::new(state.db.pool().clone());
    let security_findings = findings
        .open()
        .await
        .into_iter()
        .filter(|o| o.finding.severity >= crate::findings::Severity::High)
        .map(|o| (o.host, o.finding.rule))
        .collect();
    let silent_detectors = findings.silent(now).await;
    crate::glance::glance(&crate::glance::Inputs {
        notices,
        red_mains,
        failed_releases,
        critical_disks,
        security_findings,
        silent_detectors,
        heartbeats,
        always_open,
        now,
    })
}

/// The night glance as JSON (REQ-GLANCE-001): signed-in only; without
/// [auth] it is 401, since it reads notices.
async fn api_glance(State(state): State<AppState>) -> axum::response::Response {
    if state.login.is_none() {
        return plain(
            StatusCode::UNAUTHORIZED,
            "the glance needs a login: configure [auth]",
        );
    }
    Json(glance_now(&state).await).into_response()
}

/// The night glance as one phone-sized page (REQ-GLANCE-001).
pub async fn glance_page(State(state): State<AppState>) -> axum::response::Response {
    if state.login.is_none() {
        return plain(
            StatusCode::UNAUTHORIZED,
            "the glance needs a login: configure [auth]",
        );
    }
    axum::response::Html(crate::glance::page(&glance_now(&state).await)).into_response()
}

/// Every notice and answer row, for the decisions (REQ-DECIDE-001).
async fn decision_rows(
    state: &AppState,
) -> (Vec<(String, serde_json::Value)>, Vec<serde_json::Value>) {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as::<_, (String, String)>(
        "SELECT agent_id, content FROM agent_data WHERE data_type = 'notice.v1' ORDER BY captured_at DESC LIMIT 5000",
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap_or_default()
    .into_iter()
    .filter_map(|(a, c)| serde_json::from_str(&c).ok().map(|v| (a, v)))
    .collect();
    let answers: Vec<serde_json::Value> = sqlx::query_as::<_, (String,)>(
        "SELECT content FROM agent_data WHERE data_type = 'answer.v1' ORDER BY captured_at DESC LIMIT 5000",
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap_or_default()
    .into_iter()
    .filter_map(|(c,)| serde_json::from_str(&c).ok())
    .collect();
    (rows, answers)
}

fn always_open(state: &AppState) -> Vec<String> {
    state
        .config
        .glance
        .as_ref()
        .map(|g| g.always_open.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| crate::glance::ALWAYS_OPEN.map(String::from).to_vec())
}

/// HTMX partial: the open decisions (REQ-DECIDE-001), signed-in only.
pub async fn decisions_partial(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let Some((session, _)) = state
        .login
        .as_ref()
        .and_then(|l| l.identity_of(&headers, crate::hub::now()))
    else {
        return axum::response::Html(r#"<div class="overview"><h2>Decisions</h2><p class="empty">Answering the agents' decisions needs a login: configure <code>[auth]</code> and sign in.</p></div>"#.to_string()).into_response();
    };
    let (rows, answers) = decision_rows(&state).await;
    let ds = crate::decisions::open(&rows, &answers, &always_open(&state));
    axum::response::Html(crate::decisions::view(
        &ds,
        &state.tokens.csrf_token(&session),
    ))
    .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerRequest {
    notice_id: String,
    revision: u64,
    option_id: String,
    #[serde(default)]
    note: Option<String>,
}

/// Answer a decision (REQ-DECIDE-001): the owner role only, from this
/// origin with the session's CSRF token. Stores an `answer.v1` row on the
/// agent that asked; logged with who and which notice, never the note.
async fn api_decisions_answer(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(login) = state.login.as_ref() else {
        return plain(StatusCode::UNAUTHORIZED, "answering needs [auth]");
    };
    let Some((session, who)) = login.identity_of(&headers, crate::hub::now()) else {
        return plain(StatusCode::UNAUTHORIZED, "sign in first");
    };
    let csrf = headers.get("x-warte-csrf").and_then(|v| v.to_str().ok());
    if !same_origin(&headers) || csrf != Some(state.tokens.csrf_token(&session).as_str()) {
        return plain(StatusCode::FORBIDDEN, "not from this page");
    }
    if who.role != crate::login::Role::Owner {
        return plain(StatusCode::FORBIDDEN, "only the owner answers decisions");
    }
    let Ok(req) = serde_json::from_slice::<AnswerRequest>(&body) else {
        return plain(StatusCode::UNPROCESSABLE_ENTITY, "not an answer");
    };
    let (rows, answers) = decision_rows(&state).await;
    let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (agent_row, answer) = match crate::decisions::answer(
        &rows,
        &answers,
        &req.notice_id,
        req.revision,
        &req.option_id,
        req.note.as_deref(),
        &now,
    ) {
        Ok(x) => x,
        Err(e) => return plain(StatusCode::CONFLICT, &e),
    };
    let r = sqlx::query(
        "INSERT INTO agent_data (id, agent_id, data_type, content, captured_at, instance_id) VALUES (?, ?, 'answer.v1', ?, ?, ?)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(&agent_row)
    .bind(answer.to_string())
    .bind(Utc::now().to_rfc3339())
    .bind(state.config.instance_id())
    .execute(state.db.pool())
    .await;
    match r {
        Ok(_) => {
            tracing::info!(user = %who.name, notice = %req.notice_id, option = %req.option_id, "decision answered");
            (StatusCode::CREATED, Json(answer)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "decision answer");
            plain(StatusCode::INTERNAL_SERVER_ERROR, "database error")
        }
    }
}

/// A producer posts its report (REQ-REPORTS-001): the token's right is
/// checked by the middleware (MUTATING_ROUTES); the body against the kind.
async fn api_report_seam_check(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let kind = crate::reports::SEAM_CHECK;
    match crate::reports::check(&kind, &body) {
        Ok(v) => match crate::reports::Store::new(state.db.pool().clone())
            .put(&kind, &v)
            .await
        {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => {
                tracing::error!(error = %e, "report store");
                plain(StatusCode::INTERNAL_SERVER_ERROR, "database error")
            }
        },
        Err(e) => plain(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}

/// HTMX partial: the Seams view (#76), signed-in only: seam issues can come
/// from private repositories.
pub async fn seams_partial(State(state): State<AppState>) -> axum::response::Response {
    if state.login.is_none() {
        return axum::response::Html(r#"<div class="overview"><h2>Seams</h2><p class="empty">The Seams view lists issues from private repositories too, so it needs a login: configure <code>[auth]</code>.</p></div>"#.to_string()).into_response();
    }
    let r = crate::reports::Store::new(state.db.pool().clone())
        .get(&crate::reports::SEAM_CHECK)
        .await;
    axum::response::Html(crate::reports::seams_view(r.as_ref(), crate::hub::now())).into_response()
}

/// Who may read the reviews (REQ-PRREVIEW-001): with `[auth]` the login gate
/// already required a signed-in user; without it, only a warte bound to
/// loopback, since PR titles and authors are private work data.
fn reviews_allowed(state: &AppState) -> bool {
    state.login.is_some()
        || state
            .config
            .bind
            .parse::<std::net::SocketAddr>()
            .is_ok_and(|a| a.ip().is_loopback())
}

async fn api_reviews(State(state): State<AppState>) -> axum::response::Response {
    if !reviews_allowed(&state) {
        return plain(
            StatusCode::UNAUTHORIZED,
            "the reviews need [auth], or a warte bound to loopback",
        );
    }
    match &state.reviews {
        None => plain(StatusCode::NOT_FOUND, "no [reviews] configured"),
        Some(s) => Json(s.lock().map(|g| g.clone()).unwrap_or_default()).into_response(),
    }
}

pub async fn reviews_partial(State(state): State<AppState>) -> axum::response::Response {
    let body = if !reviews_allowed(&state) {
        r#"<div class="overview"><h2>Reviews</h2><p class="empty">The reviews show private work data: configure <code>[auth]</code>, or run this warte on loopback.</p></div>"#.to_string()
    } else {
        match &state.reviews {
            None => r#"<div class="overview"><h2>Reviews</h2><p class="empty">No <code>[reviews]</code> configured: it needs your own read-only GitHub token.</p></div>"#.to_string(),
            Some(s) => crate::reviews::view(&s.lock().map(|g| g.clone()).unwrap_or_default()),
        }
    };
    axum::response::Html(body).into_response()
}

/// A host's security findings, sent by its enrolled probe as the signed
/// `security` feed (REQ-FINDINGS-001). Any enrolled, unrevoked host may send
/// its own; one answer for every refusal, the reason to the log.
async fn api_probe_feed_security(
    State(state): State<AppState>,
    Json(f): Json<crate::probe::send::SignedFeed>,
) -> axum::response::Response {
    let refused = || plain(StatusCode::FORBIDDEN, "feed refused");
    if f.name != "security" || f.cert.len() > 16 * 1024 || f.sig.len() > 512 {
        return refused();
    }
    let now = crate::hub::now();
    let v = match crate::probe::send::verify_feed(state.ca.ca_cert_der(), &f, now) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "security feed refused");
            return refused();
        }
    };
    match crate::probe::enroll::revoked(&state.db, &v.host, v.issued_at).await {
        Ok(false) => {}
        Ok(true) => {
            tracing::warn!(host = %v.host, "security feed refused: host revoked");
            return refused();
        }
        Err(e) => {
            tracing::error!(error = %e, "security feed: revocation store");
            return plain(StatusCode::SERVICE_UNAVAILABLE, "feed unavailable");
        }
    }
    let report = match crate::findings::parse(&v.data) {
        Ok(r) => r,
        Err(why) => {
            return plain(
                StatusCode::UNPROCESSABLE_ENTITY,
                &format!("report rejected: {why}"),
            )
        }
    };
    match crate::findings::Store::new(state.db.pool().clone())
        .record(&v.host, f.at, now, &report)
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => {
            tracing::warn!(host = %v.host, "security feed refused: not newer than the last (replay?)");
            refused()
        }
        Err(e) => {
            tracing::error!(error = %e, "security feed: store");
            plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not store the findings",
            )
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptRequest {
    host: String,
    source: String,
    rule: String,
    subject: String,
}

/// Accept one finding into its host's baseline (REQ-FINDINGS-001): the owner
/// role only, from this origin with the session's CSRF token.
async fn api_security_accept(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(login) = state.login.as_ref() else {
        return plain(StatusCode::UNAUTHORIZED, "accepting needs [auth]");
    };
    let Some((session, who)) = login.identity_of(&headers, crate::hub::now()) else {
        return plain(StatusCode::UNAUTHORIZED, "sign in first");
    };
    let csrf = headers.get("x-warte-csrf").and_then(|v| v.to_str().ok());
    if !same_origin(&headers) || csrf != Some(state.tokens.csrf_token(&session).as_str()) {
        return plain(StatusCode::FORBIDDEN, "not from this page");
    }
    if who.role != crate::login::Role::Owner {
        return plain(StatusCode::FORBIDDEN, "only the owner accepts findings");
    }
    let Ok(r) = serde_json::from_slice::<AcceptRequest>(&body) else {
        return plain(StatusCode::UNPROCESSABLE_ENTITY, "not an accept request");
    };
    match crate::findings::Store::new(state.db.pool().clone())
        .accept(&r.host, &r.source, &r.rule, &r.subject, &who.name)
        .await
    {
        Ok(true) => {
            tracing::info!(user = %who.name, host = %r.host, rule = %r.rule, "finding accepted into the baseline");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => plain(StatusCode::CONFLICT, "no such open finding"),
        Err(e) => {
            tracing::error!(error = %e, "accept finding");
            plain(StatusCode::INTERNAL_SERVER_ERROR, "database error")
        }
    }
}

/// HTMX partial: open security findings (REQ-FINDINGS-001), signed-in only.
pub async fn security_partial(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let Some(login) = state.login.as_ref() else {
        return axum::response::Html(r#"<div class="overview"><h2>Security findings</h2><p class="empty">Security findings need a login: configure <code>[auth]</code>.</p></div>"#.to_string()).into_response();
    };
    let ident = login.identity_of(&headers, crate::hub::now());
    let store = crate::findings::Store::new(state.db.pool().clone());
    let open = store.open().await;
    let silent = store.silent(crate::hub::now()).await;
    let csrf = ident
        .filter(|(_, who)| who.role == crate::login::Role::Owner)
        .map(|(s, _)| state.tokens.csrf_token(&s));
    axum::response::Html(crate::findings::view(&open, &silent, csrf.as_deref())).into_response()
}
