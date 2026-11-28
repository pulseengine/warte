//! `warte agents` (REQ-CLI-002): the host lists its agents from the database,
//! without a token, as `warte agents rm` removes them.

use std::process::{Command, Output};
use warte::agent::{AgentRegistry, RegisterRequest};

fn warte(cfg: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_warte"))
        .arg("--config")
        .arg(cfg)
        .args(args)
        .output()
        .unwrap()
}

fn config(dir: &std::path::Path, db_path: &std::path::Path) -> std::path::PathBuf {
    let cfg = dir.join("warte.toml");
    std::fs::write(
        &cfg,
        format!(
            "db_path = {:?}\nca_cert_path = {:?}\nca_key_path = {:?}\n",
            db_path,
            dir.join("ca.pem"),
            dir.join("ca-key.pem")
        ),
    )
    .unwrap();
    cfg
}

// rivet: verifies REQ-CLI-002
#[tokio::test]
async fn agents_lists_from_the_database_and_filters_health() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let cfg = config(dir.path(), &db_path);
    let db = warte::db::Database::connect(&db_path).await.unwrap();
    let reg = AgentRegistry::new(db.pool().clone(), "i1".into());
    let mk = |name: &str| RegisterRequest {
        name: name.into(),
        repo: None,
        identity_type: "token".into(),
        public_key: "k".into(),
    };
    let ok = reg.register(mk("alive")).await.unwrap().agent_id;
    let stale = reg.register(mk("gone")).await.unwrap().agent_id;
    sqlx::query("UPDATE agents SET health_status = 'unhealthy' WHERE id = ?")
        .bind(&stale)
        .execute(db.pool())
        .await
        .unwrap();

    // no server runs: the list comes from the database
    let o = warte(&cfg, &["agents"]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(out.contains(&ok) && out.contains(&stale), "{out}");

    // --health matches the lowercase name the docs use
    let o = warte(&cfg, &["agents", "--health", "unhealthy"]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(0));
    assert!(out.contains(&stale) && !out.contains(&ok), "{out}");

    // case does not matter; a value outside the four is a usage error
    let o = warte(&cfg, &["agents", "--health", "UNHEALTHY"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stdout).contains(&stale));
    let o = warte(&cfg, &["agents", "--health", "sick"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("degraded"));

    // --format json is the same shape as GET /api/agents
    let o = warte(&cfg, &["--format", "json", "agents"]);
    assert_eq!(o.status.code(), Some(0));
    let v: Vec<serde_json::Value> = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v.len(), 2);
    assert!(v
        .iter()
        .all(|a| a["id"].is_string() && a["health_status"].is_string()));
}

// rivet: verifies REQ-CLI-002
#[test]
fn agents_refuses_a_missing_database_without_creating_one() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("absent.db");
    let cfg = config(dir.path(), &db_path);
    let o = warte(&cfg, &["agents"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("no database"));
    assert!(!db_path.exists());
}
