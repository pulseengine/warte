//! REQ-API-003: an operator mints agent tokens from a stored seed; the write
//! routes accept them, across restarts, and refuse everything else.

use warte::auth::{load_or_create_seed, mint_agent_token, TokenManager};

struct Server {
    dir: tempfile::TempDir,
    base: String,
}

fn config_in(dir: &std::path::Path) -> warte::Config {
    warte::Config {
        db_path: dir.join("state.db"),
        ca_cert_path: dir.join("ca.pem"),
        ca_key_path: dir.join("ca-key.pem"),
        ..Default::default()
    }
}

async fn start(dir: tempfile::TempDir) -> Server {
    let config = config_in(dir.path());
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    Server { dir, base }
}

/// Status of the three write routes an agent uses, with an optional bearer.
/// Heartbeat and data go to the id register returned (or a made-up one if
/// register was refused).
async fn writes(s: &Server, bearer: Option<&str>) -> [u16; 3] {
    let c = reqwest::Client::new();
    let with = |req: reqwest::RequestBuilder| match bearer {
        Some(t) => req.bearer_auth(t),
        None => req,
    };
    let r = with(
        c.post(format!("{}/api/agents/register", s.base))
            .json(&serde_json::json!({"name":"a1","identity_type":"token","public_key":"k"})),
    )
    .send()
    .await
    .unwrap();
    let reg = r.status().as_u16();
    let id = r
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v["agent_id"].as_str().map(String::from))
        .unwrap_or_else(|| "unregistered".into());
    let hb = with(c.post(format!("{}/api/agents/{id}/heartbeat", s.base)))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    let data = with(
        c.post(format!("{}/api/agents/{id}/data", s.base))
            .json(&serde_json::json!({"data_type":"notice","content":"x"})),
    )
    .send()
    .await
    .unwrap()
    .status()
    .as_u16();
    [reg, hb, data]
}

fn mint(s: &Server, hours: i64) -> String {
    mint_agent_token(&config_in(s.dir.path()), "a1", hours).unwrap()
}

#[tokio::test]
async fn minted_token_is_accepted_by_the_write_routes() {
    let s = start(tempfile::tempdir().unwrap()).await;
    let t = mint(&s, 24);
    let got = writes(&s, Some(&t)).await;
    assert!(
        got.iter().all(|c| *c < 300),
        "write routes answered {got:?}"
    );
}

#[tokio::test]
async fn no_token_is_401() {
    let s = start(tempfile::tempdir().unwrap()).await;
    assert_eq!(writes(&s, None).await, [401, 401, 401]);
}

#[tokio::test]
async fn tampered_token_is_401() {
    let s = start(tempfile::tempdir().unwrap()).await;
    let t = mint(&s, 24);
    let mut p: Vec<String> = t.split('.').map(String::from).collect();
    // Flip the first character of the signature.
    let flipped = if p[2].starts_with('A') { 'B' } else { 'A' };
    p[2].replace_range(0..1, &flipped.to_string());
    assert_eq!(writes(&s, Some(&p.join("."))).await, [401, 401, 401]);
    // And a payload from another token under this token's signature.
    let other = mint_agent_token(&config_in(s.dir.path()), "someone-else", 24).unwrap();
    let o: Vec<&str> = other.split('.').collect();
    let spliced = format!("warte1.{}.{}", o[1], t.split('.').nth(2).unwrap());
    assert_eq!(writes(&s, Some(&spliced)).await, [401, 401, 401]);
}

#[tokio::test]
async fn expired_token_is_401() {
    let s = start(tempfile::tempdir().unwrap()).await;
    // Same seed as the server, but a validity already in the past.
    let seed = load_or_create_seed(&config_in(s.dir.path()).token_seed_file()).unwrap();
    let t = TokenManager::from_seed(&seed, -1)
        .issue_token("a1", vec![])
        .unwrap();
    assert_eq!(writes(&s, Some(&t)).await, [401, 401, 401]);
}

#[tokio::test]
async fn token_from_a_different_seed_is_401() {
    let s = start(tempfile::tempdir().unwrap()).await;
    let t = TokenManager::from_seed(&[7u8; 32], 24)
        .issue_token("a1", vec![])
        .unwrap();
    assert_eq!(writes(&s, Some(&t)).await, [401, 401, 401]);
}

#[tokio::test]
async fn token_survives_a_restart() {
    // The seed lives in a file, so a second server on the same directory
    // accepts a token minted before the first one went away.
    let s = start(tempfile::tempdir().unwrap()).await;
    let t = mint(&s, 24);
    let dir = s.dir;
    let again = start(dir).await;
    let got = writes(&again, Some(&t)).await;
    assert!(got.iter().all(|c| *c < 300), "after restart: {got:?}");
}

#[test]
fn seed_file_is_private_stable_and_checked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("token-seed");
    let a = load_or_create_seed(&path).unwrap();
    assert_eq!(
        a,
        load_or_create_seed(&path).unwrap(),
        "seed must be stable"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // Negative controls: readable by others, and not a seed.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_or_create_seed(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    std::fs::write(&path, "not a seed\n").unwrap();
    let e = load_or_create_seed(&path).unwrap_err().to_string();
    assert!(!e.contains("not a seed"), "error must not quote the file");
}

#[test]
fn mint_refuses_bad_validity_and_empty_agent() {
    let dir = tempfile::tempdir().unwrap();
    let c = config_in(dir.path());
    assert!(mint_agent_token(&c, "a1", 0).is_err());
    assert!(mint_agent_token(&c, "a1", 24 * 365 + 1).is_err());
    assert!(mint_agent_token(&c, " ", 24).is_err());
}

#[test]
fn cli_prints_only_the_token_on_stdout() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("warte.toml"),
        format!(
            "ca_cert_path = {:?}\nca_key_path = {:?}\n",
            dir.path().join("ca.pem"),
            dir.path().join("ca-key.pem")
        ),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .args(["token", "mint", "agent-7", "--valid-hours", "48"])
        .current_dir(dir.path())
        .env_remove("WARTE_TOKEN_SEED_PATH")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let token = stdout.trim_end_matches('\n');
    assert!(token.starts_with("warte1.") && !token.contains(char::is_whitespace));
    // The server, built from the same config, verifies it.
    let seed = load_or_create_seed(&dir.path().join("token-seed")).unwrap();
    let p = TokenManager::from_seed(&seed, 24)
        .verify_token(token)
        .unwrap();
    assert_eq!(p.agent_id, "agent-7");
    assert_eq!(p.rights, warte::auth::AGENT_RIGHTS);
    let hours = (p.expires_at - p.issued_at).num_hours();
    assert_eq!(hours, 48);
    // Bad input exits non-zero and prints no token.
    let bad = std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .args(["token", "mint", "agent-7", "--valid-hours", "0"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!bad.status.success() && bad.stdout.is_empty());
}
