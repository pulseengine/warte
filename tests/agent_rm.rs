//! `warte agents rm` (REQ-API-006): the operator removes a duplicate agent on
//! the host. Every refusal is tested with the bad input planted.

use std::process::{Command, Output};
use warte::agent::{AgentRegistry, RegisterRequest};

fn req(name: &str) -> RegisterRequest {
    RegisterRequest {
        name: name.into(),
        repo: None,
        identity_type: "token".into(),
        public_key: "k".into(),
    }
}

fn rm(cfg: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_warte"))
        .arg("--config")
        .arg(cfg)
        .args(["agents", "rm"])
        .args(args)
        .output()
        .unwrap()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

// rivet: verifies REQ-API-006
#[tokio::test]
async fn rm_removes_an_empty_duplicate_and_refuses_one_with_data_or_rights() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let cfg = dir.path().join("warte.toml");
    std::fs::write(
        &cfg,
        format!(
            "db_path = {:?}\nca_cert_path = {:?}\nca_key_path = {:?}\n",
            db_path,
            dir.path().join("ca.pem"),
            dir.path().join("ca-key.pem")
        ),
    )
    .unwrap();
    let db = warte::db::Database::connect(&db_path).await.unwrap();
    let pool = db.pool().clone();
    let reg = AgentRegistry::new(pool.clone(), "i1".into());
    // three agents, as an older release made duplicates: insert two rows by hand
    let keep = reg.register(req("dup")).await.unwrap().agent_id;
    for id in ["empty-1", "with-data", "with-right"] {
        sqlx::query(
            "INSERT INTO agents (id, name, identity_type, public_key, access_rights, \
             health_status, instance_id, created_at, updated_at) \
             VALUES (?, 'dup', 'token', x'6b', '[]', 'unknown', 'i0', '2026-11-01T00:00:00Z', '2026-11-01T00:00:00Z')",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO agent_data (id, agent_id, data_type, content, instance_id) \
         VALUES ('d1', 'with-data', 'notice', '{}', 'i0')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO access_policies (id, agent_id, resource, action, granted_by) \
         VALUES ('p1', 'with-right', 'artifacts', 'download', 't')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let count = |id: &'static str| {
        let pool = pool.clone();
        async move {
            let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM agents WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
            n
        }
    };

    // the empty duplicate goes, with its reason logged
    let o = rm(
        &cfg,
        &["empty-1", "--reason", "duplicate from a register retry"],
    );
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert_eq!(count("empty-1").await, 0);
    let log = std::fs::read_to_string(dir.path().join("agents-removed.jsonl")).unwrap();
    assert!(
        log.contains("\"id\":\"empty-1\"") && log.contains("register retry"),
        "{log}"
    );

    // planted: data rows, without and then with --with-data
    let o = rm(&cfg, &["with-data", "--reason", "x"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("1 data row(s)"), "{}", err(&o));
    assert_eq!(count("with-data").await, 1, "a refused rm removes nothing");
    let o = rm(&cfg, &["with-data", "--reason", "x", "--with-data"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let (left,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM agent_data WHERE agent_id = 'with-data'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(left, 0);

    // planted: an access policy, an unknown id, a bad reason
    let o = rm(&cfg, &["with-right", "--reason", "x", "--with-data"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("revoke them first"), "{}", err(&o));
    assert_eq!(count("with-right").await, 1);
    let o = rm(&cfg, &["no-such-agent", "--reason", "x"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("no agent no-such-agent"), "{}", err(&o));
    for bad in ["", "   ", "two\nlines"] {
        let o = rm(&cfg, &[keep.as_str(), "--reason", bad]);
        assert_eq!(o.status.code(), Some(1), "{bad:?}");
    }
    let long = "x".repeat(201);
    assert_eq!(
        rm(&cfg, &[keep.as_str(), "--reason", &long]).status.code(),
        Some(1)
    );

    // the kept agent is untouched, and registering the name finds the newest left
    let again = reg.register(req("dup")).await.unwrap().agent_id;
    assert_eq!(again, keep);
}
