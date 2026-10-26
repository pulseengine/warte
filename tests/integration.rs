//! Integration tests for the warte control tower.

use warte::agent::{AgentRegistry, RegisterRequest};
use warte::db::Database;

#[tokio::test]
async fn test_agent_registration() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Database::connect(&db_path).await.unwrap();
    let registry = AgentRegistry::new(db.pool().clone(), "test-instance".to_string());

    let req = RegisterRequest {
        name: "test-agent".to_string(),
        repo: Some("pulseengine/rivet".to_string()),
        identity_type: "signed-challenge".to_string(),
        public_key: "test-key".to_string(),
    };

    let resp = registry.register(req).await.unwrap();
    assert!(!resp.agent_id.is_empty());
    assert!(resp.cert_expires_at.is_some());

    let agent = registry.get(&resp.agent_id).await.unwrap().unwrap();
    assert_eq!(agent.name, "test-agent");
    assert_eq!(agent.repo, Some("pulseengine/rivet".to_string()));
}

#[tokio::test]
async fn test_agent_heartbeat() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Database::connect(&db_path).await.unwrap();
    let registry = AgentRegistry::new(db.pool().clone(), "test-instance".to_string());

    let req = RegisterRequest {
        name: "heartbeat-agent".to_string(),
        repo: None,
        identity_type: "mtls".to_string(),
        public_key: "key".to_string(),
    };
    let resp = registry.register(req).await.unwrap();

    registry.heartbeat(&resp.agent_id).await.unwrap();

    let agent = registry.get(&resp.agent_id).await.unwrap().unwrap();
    assert!(agent.last_heartbeat.is_some());
    assert_eq!(agent.health_status, warte::agent::HealthStatus::Healthy);
}

#[tokio::test]
async fn test_access_control_grant_revoke() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Database::connect(&db_path).await.unwrap();
    let access = warte::access::AccessControl::new(db.pool().clone(), "test-instance".to_string());
    let registry = AgentRegistry::new(db.pool().clone(), "test-instance".to_string());

    // Create agent first (FK constraint)
    let agent = registry
        .register(RegisterRequest {
            name: "access-test-agent".to_string(),
            repo: None,
            identity_type: "mtls".to_string(),
            public_key: "key".to_string(),
        })
        .await
        .unwrap();

    let rights = vec![
        warte::access::AccessRight::new("artifacts", "download"),
        warte::access::AccessRight::new("data", "write"),
    ];

    // Grant
    access
        .grant(&agent.agent_id, &rights, "operator-1")
        .await
        .unwrap();

    // Check
    assert!(access
        .check(&agent.agent_id, "artifacts", "download")
        .await
        .unwrap());
    assert!(access
        .check(&agent.agent_id, "data", "write")
        .await
        .unwrap());
    assert!(!access
        .check(&agent.agent_id, "access", "grant")
        .await
        .unwrap());

    // List
    let listed = access.list_rights(&agent.agent_id).await.unwrap();
    assert_eq!(listed.len(), 2);

    // Revoke
    access
        .revoke(&agent.agent_id, &rights, "operator-1")
        .await
        .unwrap();

    // Check after revoke
    assert!(!access
        .check(&agent.agent_id, "artifacts", "download")
        .await
        .unwrap());
}

#[tokio::test]
async fn test_artifact_store_and_integrity() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Database::connect(&db_path).await.unwrap();
    let store = warte::artifact::ArtifactStore::new(db.pool().clone());

    let content = b"hello warte";
    let artifact = store
        .store(warte::artifact::StoreParams {
            name: "test-artifact",
            version: "1.0.0",
            content,
            integrity_proof: b"fake-signature",
            integrity_type: warte::artifact::IntegrityType::SignedManifest,
            access_scope: vec!["agent-1".to_string()],
            description: Some("A test artifact"),
        })
        .await
        .unwrap();

    assert_eq!(artifact.name, "test-artifact");
    assert_eq!(artifact.version, "1.0.0");

    // Download and verify
    let download = store.download(&artifact.id).await.unwrap().unwrap();
    assert_eq!(download.content, content);
    assert_eq!(download.content_hash, artifact.content_hash);

    // Verify integrity
    assert!(store.verify_integrity(&artifact.id).await.unwrap());

    // List
    let list = store.list().await.unwrap();
    assert_eq!(list.len(), 1);
}

#[tokio::test]
async fn test_sync_version_vectors() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Database::connect(&db_path).await.unwrap();
    let sync = warte::sync::SyncService::new(db.pool().clone(), "instance-1".to_string());

    // Increment
    let vv1 = sync.increment().await.unwrap();
    assert_eq!(vv1.get("instance-1"), Some(&1));

    let vv2 = sync.increment().await.unwrap();
    assert_eq!(vv2.get("instance-1"), Some(&2));

    // Merge remote
    let mut remote = std::collections::HashMap::new();
    remote.insert("instance-2".to_string(), 5u64);
    let merged = sync.merge_version_vector(&remote).await.unwrap();
    assert_eq!(merged.get("instance-1"), Some(&2));
    assert_eq!(merged.get("instance-2"), Some(&5));

    // Divergence detection
    let behind = warte::sync::SyncService::detect_divergence(&merged, &remote);
    assert!(behind.is_empty()); // we're ahead of remote for instance-2

    let behind2 = warte::sync::SyncService::detect_divergence(&remote, &merged);
    assert_eq!(behind2, vec!["instance-1".to_string()]); // remote is behind us for instance-1
}

#[tokio::test]
async fn test_cli_version_format() {
    // Verify the CLI version follows PulseEngine conventions:
    // "warte <semver>" and exits 0
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with("warte "),
        "version should start with 'warte ', got: {}",
        stdout
    );
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "version should contain semver"
    );
}
