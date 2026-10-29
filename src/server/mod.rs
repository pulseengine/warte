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
use tower_http::cors::CorsLayer;
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
        .layer(CorsLayer::permissive())
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
        None => None,
    };
    let machines = config.hub.as_ref().map(|h| {
        let m = Arc::new(crate::probe::Machines::open(&h.data_dir));
        if h.probe_self {
            crate::probe::spawn_self(m.clone(), std::time::Duration::from_secs(60));
        }
        m
    });

    let state = AppState {
        hub,
        machines,
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
        status: Arc::new(crate::status::StatusClient::github()),
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
