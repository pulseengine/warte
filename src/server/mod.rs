//! HTTP API server (Axum).

pub mod api;
pub mod auth;

use crate::access::AccessControl;
use crate::agent::AgentRegistry;
use crate::artifact::ArtifactStore;
use crate::config::Config;
use crate::db::Database;
use crate::sync::SyncService;
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
}

/// Build the Axum router with all routes.
pub fn build_router(state: AppState) -> Router {
    use axum::middleware;

    Router::new()
        .nest("/api", api::build_routes())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::auth_middleware,
        ))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the server.
pub async fn serve(config: Config, db: Database) -> anyhow::Result<()> {
    let instance_id = config.instance_id();
    let pool = db.pool().clone();

    // Initialize CA for mTLS certificate issuance
    let ca = Arc::new(crate::ca::CertificateAuthority::load_or_create(
        &config.ca_cert_path,
        &config.ca_key_path,
        config.cert_validity_hours,
    )?);
    tracing::info!(ca_cert = %config.ca_cert_path.display(), "CA initialized");

    let state = AppState {
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
        config: config.clone(),
        db,
    };

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;

    tracing::info!(bind = %config.bind, instance = %instance_id, "warte control tower listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
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
