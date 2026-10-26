//! mTLS authentication middleware (placeholder — full implementation pending).

use axum::{extract::Request, middleware::Next, response::Response};

/// Placeholder auth middleware. In production, this extracts the client
/// certificate from the mTLS handshake and validates it against the CA.
pub async fn auth_middleware(req: Request, next: Next) -> Response {
    // TODO: extract client cert from TLS handshake
    // TODO: validate cert against CA
    // TODO: extract agent ID from cert CN
    // TODO: check cert expiry
    next.run(req).await
}
