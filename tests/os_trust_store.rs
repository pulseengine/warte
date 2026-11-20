//! Oracles for the trust roots (REQ-TLS-001): public servers are checked
//! against the operating system's trust store, never a bundled list, and a
//! configured private CA is still accepted. The negative control is a local
//! HTTPS server whose CA is in neither place.

use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Ca {
    pem: String,
    cert: rcgen::Certificate,
    key: rcgen::KeyPair,
}

fn ca(name: &str) -> Ca {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name.push(rcgen::DnType::CommonName, name);
    p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let cert = p.self_signed(&key).unwrap();
    Ca {
        pem: cert.pem(),
        cert,
        key,
    }
}

/// An HTTPS server on 127.0.0.1 with a leaf issued by `ca`; it answers every
/// request with a token answer, so a refusal can only come from the client.
async fn serve(ca: &Ca) -> String {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = rcgen::CertificateParams::new(vec!["127.0.0.1".to_string()]).unwrap();
    p.distinguished_name
        .push(rcgen::DnType::CommonName, "127.0.0.1");
    let leaf = p.signed_by(&key, &ca.cert, &ca.key).unwrap();
    let chain = vec![leaf.der().clone()];
    let pk = rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(chain, pk)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("https://{}", l.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let (tcp, _) = l.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut s) = acceptor.accept(tcp).await else {
                    return; // the client refused the handshake
                };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = s.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text[..end]
                            .lines()
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                k.eq_ignore_ascii_case("content-length")
                                    .then(|| v.trim().parse::<usize>().ok())?
                            })
                            .unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let body = r#"{"token":"ghs_planted","expires_at":"2026-11-20T19:00:00Z","repositories":["warte"],"permissions":{"contents":"write"}}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    base
}

fn warte_token() -> String {
    let payload = URL_SAFE_NO_PAD.encode(br#"{"agent_id":"a1"}"#);
    format!("warte1.{payload}.sig")
}

// rivet: verifies REQ-TLS-001
#[tokio::test]
async fn a_ca_in_neither_the_os_store_nor_the_config_is_refused() {
    let private = ca("warte test CA, not in any trust store");
    let base = serve(&private).await;
    let err = warte::ghtoken::fetch(&base, None, &warte_token(), &[])
        .await
        .unwrap_err();
    assert!(err.starts_with("warte could not be reached"), "{err}");

    // a different configured CA does not vouch for this server either
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.pem");
    std::fs::write(&other, ca("another CA").pem).unwrap();
    let err = warte::ghtoken::fetch(&base, Some(&other), &warte_token(), &[])
        .await
        .unwrap_err();
    assert!(err.starts_with("warte could not be reached"), "{err}");
}

// rivet: verifies REQ-TLS-001
#[tokio::test]
async fn the_configured_private_ca_is_still_accepted() {
    let private = ca("warte test CA, configured");
    let base = serve(&private).await;
    let dir = tempfile::tempdir().unwrap();
    let pem = dir.path().join("ca.pem");
    std::fs::write(&pem, &private.pem).unwrap();
    let issued = warte::ghtoken::fetch(&base, Some(&pem), &warte_token(), &[])
        .await
        .unwrap();
    assert_eq!(issued.token, "ghs_planted");

    // the same bundle through the GitHub source's loader (ca_bundle)
    let roots = warte::ghsource::extra_roots(&pem).unwrap();
    assert_eq!(roots.len(), 1);
}

// rivet: verifies REQ-TLS-001
#[test]
fn no_bundled_root_list_is_linked() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).unwrap();
    assert!(
        !lock.contains("name = \"webpki-roots\""),
        "a bundled CA list is back in the dependency tree"
    );
    assert!(lock.contains("name = \"rustls-native-certs\""));
}
