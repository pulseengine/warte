//! HTTP API server (Axum).

pub mod api;

use crate::access::AccessControl;
use crate::agent::AgentRegistry;
use crate::artifact::ArtifactStore;
use crate::config::Config;
use crate::db::Database;
use crate::sync::SyncService;
use axum::Router;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Database,
    pub agents: Arc<AgentRegistry>,
    pub access: Arc<AccessControl>,
    pub artifacts: Arc<ArtifactStore>,
    pub sync: Arc<SyncService>,
    pub tokens: Arc<crate::auth::TokenManager>,
    pub realtime: Arc<crate::realtime::EventBroadcaster>,
    pub status: Arc<crate::status::StatusClient>,
    /// The ground station, when `[hub]` is configured.
    pub hub: Option<crate::hub::Hub>,
    /// The latest report per machine (REQ-PROBE-001), with the ground station.
    pub machines: Option<Arc<crate::probe::Machines>>,
    /// warte's CA: signs probe certificates at enrollment (DD-PROBE-002).
    pub ca: Arc<crate::ca::CertificateAuthority>,
    /// The fleet model, or why it could not be read (REQ-FLEET-001).
    pub fleet: Option<Arc<Result<crate::fleet::Fleet, String>>>,
    /// The latest notional-cost report (REQ-COSTS-001), with the ground station.
    pub costs: Option<Arc<crate::costs::Costs>>,
    /// The latest skill-usage report per source (REQ-SKILLS-001).
    pub skills: Option<Arc<crate::skills::Skills>>,
    /// temper's latest report, when `[temper]` and `[hub]` are configured.
    pub temper: Option<Arc<crate::temper::Store>>,
    /// The dashboard's idle scene (REQ-DASH-018).
    pub idle: crate::config::IdleConfig,
    /// Login through the provider, with `[auth]` (DD-AUTH-001).
    pub login: Option<Arc<crate::login::Login>>,
    /// Every briefing with its facts (REQ-BRIEF-001).
    pub briefing: Option<Arc<crate::briefing::Store>>,
    /// Issues people opened, waiting for a first reply (REQ-TRIAGE-001).
    pub triage: Option<Arc<crate::triage::Triage>>,
    /// The latest dependency-canary round (REQ-PREDICT-003).
    pub canaries: Option<Arc<crate::canary::Canaries>>,
    /// What runs out and when (REQ-PREDICT-004).
    pub expiry: Arc<crate::expiry::Expiry>,
    /// CI health trends, recomputed every ten minutes (REQ-PREDICT-005).
    pub ci_health: Arc<crate::cihealth::Cache>,
}

/// Build the Axum router with all routes.
pub fn build_router(state: AppState) -> Router {
    use axum::middleware;

    // build_routes() already places the API under /api and the dashboard at /.
    // (Before v1.4.0 this nested it a second time: /api/api/..., no dashboard.)
    // The ground station's answers are under warte's own API; the absorbed
    // smithy-hub page and API once mounted at /ci are retired (REQ-SMITHY-007).
    // `warte hub` still serves them on its own port.
    Router::new()
        .merge(api::build_routes())
        .route("/auth/login", axum::routing::get(crate::login::login_route))
        .route(
            "/auth/callback",
            axum::routing::get(crate::login::callback_route),
        )
        .route(
            "/auth/logout",
            axum::routing::get(crate::login::logout_route),
        )
        .route("/api/me", axum::routing::get(crate::login::me_route))
        .route("/ci", axum::routing::any(ci_gone))
        .route("/ci/", axum::routing::any(ci_gone))
        .route("/ci/{*rest}", axum::routing::any(ci_gone))
        // a real text 404: an empty body with no content type is saved as
        // "document.txt" by browsers instead of being shown
        .fallback(|| async { (axum::http::StatusCode::NOT_FOUND, "not found") })
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::auth_middleware,
        ))
        // No CORS layer: the dashboard is same-origin and agents use the CLI.
        // A permissive one let any web page a LAN user opened read the whole
        // read API (machines, agents, queue) cross-origin.
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the server.
pub async fn serve(config: Config, db: Database) -> anyhow::Result<()> {
    let state = build_state(config.clone(), db)?;
    let instance_id = state.config.instance_id();
    crate::alerts::spawn(state.clone()); // REQ-ALERT-001
    let history = state.machines.as_ref().and_then(|m| m.history().cloned());
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;

    tracing::info!(bind = %config.bind, instance = %instance_id, "warte control tower listening");

    // peer addresses: /api/watch caps held requests per client
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // buffered history samples become durable (REQ-MHIST-001)
    if let Some(h) = history {
        h.flush();
    }
    Ok(())
}

/// Everything the routes need, including the ground station when `[hub]` is set.
pub fn build_state(mut config: Config, db: Database) -> anyhow::Result<AppState> {
    // instance_id() mints a fresh id when none is configured: fix it once
    let instance_id = config.instance_id();
    config.instance_id = Some(instance_id.clone());
    let pool = db.pool().clone();

    // Initialize CA for mTLS certificate issuance
    let ca = Arc::new(crate::ca::CertificateAuthority::load_or_create(
        &config.ca_cert_path,
        &config.ca_key_path,
        config.cert_validity_hours,
    )?);
    tracing::info!(ca_cert = %config.ca_cert_path.display(), "CA initialized");

    if config.hub.is_some() && config.github.is_some() {
        anyhow::bail!(
            "[hub] and [github] are both set: choose one CI source (the smithy agent's telemetry, or GitHub's API)"
        );
    }
    let hub = match &config.hub {
        Some(h) => {
            crate::hub::check_allow(&h.allow).map_err(anyhow::Error::msg)?;
            let hub = crate::hub::Hub::open(&h.data_dir, &h.org).map_err(anyhow::Error::msg)?;
            let udp = hub
                .spawn_ingest(&h.udp, h.allow.clone())
                .map_err(anyhow::Error::msg)?;
            tracing::info!(%udp, data_dir = %h.data_dir, "ground station up");
            Some(hub)
        }
        None => match &config.github {
            // No smithy agent: GitHub's API is the CI source (#81).
            Some(g) => {
                std::fs::create_dir_all(&g.data_dir)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", g.data_dir))?;
                let hub = crate::hub::Hub::open(&g.data_dir, &g.org).map_err(anyhow::Error::msg)?;
                crate::ghsource::spawn(hub.clone(), g.clone(), config.runners.clone())
                    .map_err(anyhow::Error::msg)?;
                tracing::info!(api = %g.api_url, org = %g.org, every_s = g.interval_s, "CI source: GitHub API");
                Some(hub)
            }
            None => None,
        },
    };
    if let Some(h) = &hub {
        h.set_runners(
            config.runners.clone(),
            config
                .github
                .as_ref()
                .map(|g| g.self_hosted_labels.clone())
                .unwrap_or_default(),
        );
    }
    let machines = match (&config.hub, &config.github) {
        (Some(h), _) => Some((h.data_dir.clone(), h.probe_self)),
        (None, Some(g)) => Some((g.data_dir.clone(), true)),
        (None, None) => None,
    }
    .map(|(dir, probe_self)| {
        let m = Arc::new(crate::probe::Machines::open(&dir));
        // history next to machines.json; without it warte works as before
        match crate::history::History::open(&dir) {
            Ok(h) => {
                if let Some(d) = config.history.as_ref().and_then(|c| c.keep_days) {
                    h.set_keep_days(d);
                }
                // upkeep starts below, after the ground station's import
                m.attach_history(h);
            }
            Err(e) => tracing::warn!(error = %e, "machine history unavailable"),
        }
        if probe_self {
            crate::probe::spawn_self(m.clone(), std::time::Duration::from_secs(60));
        }
        m
    });

    if let Some(h) = machines.as_ref().and_then(|m| m.history()) {
        if let Some(hub) = &hub {
            hub.attach_history(h.clone());
        }
        h.spawn_upkeep();
    }

    let costs = config.hub.as_ref().map(|h| {
        Arc::new(crate::costs::Costs::open(
            &h.data_dir,
            h.costs_file.as_deref(),
        ))
    });

    let skills = config.hub.as_ref().map(|h| {
        Arc::new(crate::skills::Skills::open(
            &h.data_dir,
            h.skill_usage_file.as_deref(),
        ))
    });

    // the briefing keeps its files in its own data_dir, or the ground
    // station's, or the GitHub source's
    let briefing = match &config.briefing {
        Some(b) => {
            let dir = b
                .data_dir
                .clone()
                .or_else(|| config.hub.as_ref().map(|h| h.data_dir.clone()))
                .or_else(|| config.github.as_ref().map(|g| g.data_dir.clone()))
                .ok_or_else(|| anyhow::anyhow!("[briefing] needs data_dir, [hub] or [github]"))?;
            let store = Arc::new(crate::briefing::Store::open(&dir).map_err(anyhow::Error::msg)?);
            crate::briefing::spawn(store.clone(), b.clone(), hub.clone(), costs.clone());
            Some(store)
        }
        None => None,
    };

    let triage = match &config.triage {
        Some(t) => {
            let dir = t
                .data_dir
                .clone()
                .or_else(|| config.hub.as_ref().map(|h| h.data_dir.clone()))
                .or_else(|| config.github.as_ref().map(|g| g.data_dir.clone()))
                .ok_or_else(|| anyhow::anyhow!("[triage] needs data_dir, [hub] or [github]"))?;
            let q =
                Arc::new(crate::triage::Triage::open(&dir, t.clone()).map_err(anyhow::Error::msg)?);
            crate::triage::spawn(q.clone());
            Some(q)
        }
        None => None,
    };

    let canaries = match &config.canaries {
        Some(c) => {
            let resolvers = c.resolvers().map_err(anyhow::Error::msg)?;
            let store = Arc::new(crate::canary::Canaries::default());
            crate::canary::spawn(store.clone(), c.clone(), resolvers);
            Some(store)
        }
        None => None,
    };

    let expiry = {
        let cfg = config.expiry.clone().unwrap_or_default();
        cfg.check().map_err(anyhow::Error::msg)?;
        let own = crate::expiry::Own {
            ca: Some(config.ca_cert_path.clone()),
            tls: config.tls_enabled.then(|| config.tls_cert_path.clone()),
        };
        let e = Arc::new(crate::expiry::Expiry::new(cfg, own));
        crate::expiry::spawn(e.clone());
        e
    };

    let temper = match (&config.temper, &config.hub) {
        (Some(_), Some(h)) => Some(Arc::new(crate::temper::Store::open(&h.data_dir))),
        _ => None,
    };

    // A model that cannot be read is shown as a problem, not a startup failure:
    // the CI views do not depend on it.
    // The detail (path, OS error) goes to the server log only: the read API
    // has no auth (#46), so callers get a generic message.
    let fleet = config.fleet_model.as_ref().map(|p| {
        Arc::new(
            std::fs::read_to_string(p)
                .map_err(|e| e.to_string())
                .and_then(|t| crate::fleet::from_spar(&t))
                .map_err(|e| {
                    tracing::warn!(path = %p.display(), error = %e, "fleet model unreadable");
                    "the configured fleet model could not be read; see the server log".to_string()
                }),
        )
    });
    // A stored seed, not a fresh key per start: tokens survive a restart.
    let token_seed = crate::auth::load_or_create_seed(&config.token_seed_file())?;
    let state = AppState {
        idle: config.idle.clone(),
        login: match &config.auth {
            Some(a) => Some(Arc::new(
                crate::login::Login::new(a.clone()).map_err(anyhow::Error::msg)?,
            )),
            None => None,
        },
        briefing,
        triage,
        canaries,
        expiry,
        ci_health: Default::default(),
        ca: ca.clone(),
        hub,
        machines,
        fleet,
        costs,
        skills,
        temper,
        agents: Arc::new(AgentRegistry::with_ca(
            pool.clone(),
            instance_id.clone(),
            ca,
        )),
        access: Arc::new(AccessControl::new(pool.clone(), instance_id.clone())),
        artifacts: Arc::new(ArtifactStore::new(pool.clone())),
        sync: Arc::new(SyncService::new(pool.clone(), instance_id.clone())),
        tokens: Arc::new(crate::auth::TokenManager::from_seed(&token_seed, 24)),
        realtime: Arc::new(crate::realtime::EventBroadcaster::new()),
        status: Arc::new(
            match config.github.as_ref().map(|g| g.status_url.as_str()) {
                Some(url) => crate::status::StatusClient::new(url),
                None => crate::status::StatusClient::github(),
            },
        ),
        config: config.clone(),
        db,
    };
    Ok(state)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("received Ctrl+C, shutting down"),
        _ = terminate => tracing::info!("received SIGTERM, shutting down"),
    }
}

/// The retired `/ci` paths (REQ-SMITHY-007) answer 410 Gone with their
/// replacement under warte's own API, never an empty 404 that an agent could
/// read as "nothing queued". `see` is relative to warte's base URL and keeps
/// the query.
async fn ci_gone(uri: axum::http::Uri) -> impl axum::response::IntoResponse {
    let see = ci_replacement(uri.path());
    let see = match uri.query() {
        Some(q) if see != "/" => format!("{see}?{q}"),
        _ => see.to_string(),
    };
    (
        axum::http::StatusCode::GONE,
        axum::Json(serde_json::json!({
            "error": "gone",
            "see": see,
            "reason": "/ci was retired in warte v1.6.3; its answers are under warte's own API, relative to warte's base URL",
        })),
    )
}

fn ci_replacement(path: &str) -> &'static str {
    match path.trim_end_matches('/') {
        "/ci/api/eta" => "/api/eta",
        "/ci/status.md" => "/api/status.md",
        "/ci/api/report" => "/api/report",
        "/ci/report.md" => "/api/report.md",
        "/ci/api/history" => "/api/history/fleet",
        "/ci/api/state" => "/api/queue",
        _ => "/",
    }
}
