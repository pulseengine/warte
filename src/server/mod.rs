//! HTTP API server (Axum).

pub mod api;
pub mod auth;

use crate::access::AccessControl;
use crate::agent::AgentRegistry;
use crate::artifact::ArtifactStore;
use crate::config::Config;
use crate::db::Database;
use crate::sync::SyncService;
use axum::response::IntoResponse;
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
    /// The fleet model, or why it could not be read (REQ-FLEET-001).
    pub fleet: Option<Arc<Result<crate::fleet::Fleet, String>>>,
    /// The latest notional-cost report (REQ-COSTS-001), with the ground station.
    pub costs: Option<Arc<crate::costs::Costs>>,
}

/// Build the Axum router with all routes.
pub fn build_router(state: AppState) -> Router {
    use axum::middleware;

    // The hub API is read-only and served without warte's auth, exactly as on
    // smithy hub today (https://<host>/ci/ behind Caddy).
    let hub_api = state.hub.as_ref().map(|h| h.router());
    // build_routes() already places the API under /api and the dashboard at /.
    // (Before v1.4.0 this nested it a second time: /api/api/..., no dashboard.)
    let app = Router::new()
        .merge(api::build_routes())
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
        .with_state(state);
    match hub_api {
        // logged like the rest; no CORS (smithy hub had none, and it is GET-only)
        // axum's nest matches "/ci" and "/ci/<path>" but not "/ci/" itself,
        // which is where a browser lands for the hub's web view (and where
        // Caddy forwards ".../ci/"): serve the same index there.
        Some(hub) => app
            .route(
                "/ci/",
                axum::routing::get(|| async {
                    axum::response::Html(crate::hub::INDEX).into_response()
                }),
            )
            .nest("/ci", hub.layer(TraceLayer::new_for_http())),
        None => app,
    }
}

/// Start the server.
pub async fn serve(config: Config, db: Database) -> anyhow::Result<()> {
    let state = build_state(config.clone(), db)?;
    let instance_id = state.config.instance_id();
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
            tracing::info!(%udp, data_dir = %h.data_dir, "ground station up, API at /ci");
            Some(hub)
        }
        None => match &config.github {
            // No smithy agent: GitHub's API is the CI source (#81).
            Some(g) => {
                std::fs::create_dir_all(&g.data_dir)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", g.data_dir))?;
                let hub = crate::hub::Hub::open(&g.data_dir, &g.org).map_err(anyhow::Error::msg)?;
                crate::ghsource::spawn(hub.clone(), g.clone()).map_err(anyhow::Error::msg)?;
                tracing::info!(api = %g.api_url, org = %g.org, every_s = g.interval_s, "CI source: GitHub API");
                Some(hub)
            }
            None => None,
        },
    };
    let machines = match (&config.hub, &config.github) {
        (Some(h), _) => Some((h.data_dir.clone(), h.probe_self)),
        (None, Some(g)) => Some((g.data_dir.clone(), true)),
        (None, None) => None,
    }
    .map(|(dir, probe_self)| {
        let m = Arc::new(crate::probe::Machines::open(&dir));
        if probe_self {
            crate::probe::spawn_self(m.clone(), std::time::Duration::from_secs(60));
        }
        m
    });

    let costs = config.hub.as_ref().map(|h| {
        Arc::new(crate::costs::Costs::open(
            &h.data_dir,
            h.costs_file.as_deref(),
        ))
    });

    // A model that cannot be read is shown as a problem, not a startup failure:
    // the CI views do not depend on it.
    let fleet = config.fleet_model.as_ref().map(|p| {
        Arc::new(
            std::fs::read_to_string(p)
                .map_err(|e| format!("{}: {e}", p.display()))
                .and_then(|t| crate::fleet::from_spar(&t)),
        )
    });
    let state = AppState {
        hub,
        machines,
        fleet,
        costs,
        agents: Arc::new(AgentRegistry::with_ca(
            pool.clone(),
            instance_id.clone(),
            ca,
        )),
        access: Arc::new(AccessControl::new(pool.clone(), instance_id.clone())),
        artifacts: Arc::new(ArtifactStore::new(pool.clone())),
        sync: Arc::new(SyncService::new(pool.clone(), instance_id.clone())),
        tokens: Arc::new(crate::auth::TokenManager::new(24)),
        realtime: Arc::new(crate::realtime::EventBroadcaster::new()),
        status: Arc::new(
            match config.github.as_ref().map(|g| g.status_url.as_str()) {
                Some(url) if url != "off" => crate::status::StatusClient::new(url),
                _ => crate::status::StatusClient::github(),
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
