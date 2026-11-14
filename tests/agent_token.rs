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
    // The agents the tests write as. Data and grants point at an agents row
    // (foreign key), and register mints its own ids, so seed rows with known ids.
    for id in ["a1", "b2"] {
        sqlx::query(
            "INSERT OR IGNORE INTO agents (id, name, identity_type, public_key, access_rights, \
             health_status, instance_id, created_at, updated_at) \
             VALUES (?, ?, 'token', x'6b', '[]', 'unknown', 'test', 'now', 'now')",
        )
        .bind(id)
        .bind(id)
        .execute(db.pool())
        .await
        .unwrap();
    }
    let state = warte::server::build_state(config, db).unwrap();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    Server { dir, base }
}

/// Status of the three write routes an agent uses, with an optional bearer.
/// Register names `a1`; heartbeat and data go to `a1`, the id `mint` puts in
/// the token (REQ-API-004: a token writes only as its own agent).
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
    let id = "a1";
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

// ── REQ-API-004: rights and agent binding on the mutating routes ─────────

/// Every right a route can require, plus the agent-binding override.
const ALL_RIGHTS: [&str; 12] = [
    "agents:register",
    "agents:heartbeat",
    "agents:data",
    "access:grant",
    "access:revoke",
    "sync:update",
    "costs:write",
    "skills:write",
    "briefing:write",
    "triage:write",
    "news:write",
    "agents:any",
];

/// (path, right) of every mutating route. `{id}` is filled with the agent.
const ROUTES: [(&str, &str); 11] = [
    ("/api/agents/register", "agents:register"),
    ("/api/agents/{id}/heartbeat", "agents:heartbeat"),
    ("/api/agents/{id}/data", "agents:data"),
    ("/api/access/grant", "access:grant"),
    ("/api/access/revoke", "access:revoke"),
    ("/api/sync/update", "sync:update"),
    ("/api/costs", "costs:write"),
    ("/api/skill-usage", "skills:write"),
    ("/api/briefing", "briefing:write"),
    ("/api/triage/done", "triage:write"),
    ("/api/news/state", "news:write"),
];

fn token_with(s: &Server, agent: &str, rights: &[&str]) -> String {
    let seed = load_or_create_seed(&config_in(s.dir.path()).token_seed_file()).unwrap();
    TokenManager::from_seed(&seed, 24)
        .issue_token(agent, rights.iter().map(|r| r.to_string()).collect())
        .unwrap()
}

/// POST `path` ({id} replaced by `target`) with a body the route accepts,
/// as `bearer`; the status code.
async fn post_as(s: &Server, path: &str, target: &str, bearer: Option<&str>) -> u16 {
    let right = serde_json::json!([{"resource": "artifacts", "action": "download"}]);
    let body = if path.ends_with("/register") {
        serde_json::json!({"name": target, "identity_type": "token", "public_key": "k"})
    } else if path.ends_with("/data") {
        serde_json::json!({"data_type": "notice", "content": "x"})
    } else if path.ends_with("/grant") {
        serde_json::json!({"agent_id": target, "rights": right, "granted_by": "t"})
    } else if path.ends_with("/revoke") {
        serde_json::json!({"agent_id": target, "rights": right, "revoked_by": "t"})
    } else {
        serde_json::json!({})
    };
    let mut req = reqwest::Client::new()
        .post(format!("{}{}", s.base, path.replace("{id}", target)))
        .json(&body);
    if let Some(t) = bearer {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap().status().as_u16()
}

#[tokio::test]
async fn every_route_refuses_a_token_lacking_its_right_with_403() {
    let s = start(tempfile::tempdir().unwrap()).await;
    for (path, right) in ROUTES {
        // Every right but this one, and the agent matches: only the right is missing.
        let others: Vec<&str> = ALL_RIGHTS
            .iter()
            .copied()
            .filter(|r| *r != right && *r != "agents:any")
            .collect();
        let t = token_with(&s, "a1", &others);
        assert_eq!(post_as(&s, path, "a1", Some(&t)).await, 403, "{path}");
        // A token with no rights at all.
        let none = token_with(&s, "a1", &[]);
        assert_eq!(post_as(&s, path, "a1", Some(&none)).await, 403, "{path}");
    }
}

#[tokio::test]
async fn every_route_accepts_a_token_with_its_right() {
    let s = start(tempfile::tempdir().unwrap()).await;
    for (path, right) in ROUTES {
        let t = token_with(&s, "a1", &[right]);
        let got = post_as(&s, path, "a1", Some(&t)).await;
        assert!(
            ![401, 403].contains(&got),
            "{path} with only {right}: {got}"
        );
        // costs, the briefing and sync need a ground station, a [briefing]
        // a [news] or a real record: only "not refused".
        if !path.ends_with("/costs")
            && !path.ends_with("/skill-usage")
            && !path.ends_with("/briefing")
            && !path.ends_with("/triage/done")
            && !path.ends_with("/news/state")
            && !path.ends_with("/update")
        {
            assert!(got < 300, "{path} with only {right}: {got}");
        }
    }
}

#[tokio::test]
async fn every_route_without_a_token_is_401() {
    let s = start(tempfile::tempdir().unwrap()).await;
    for (path, _) in ROUTES {
        assert_eq!(post_as(&s, path, "a1", None).await, 401, "{path}");
    }
}

#[tokio::test]
async fn a_fresh_mint_cannot_grant_or_revoke_access() {
    let s = start(tempfile::tempdir().unwrap()).await;
    let t = mint(&s, 24);
    assert_eq!(post_as(&s, "/api/access/grant", "a1", Some(&t)).await, 403);
    assert_eq!(post_as(&s, "/api/access/revoke", "a1", Some(&t)).await, 403);
    assert_eq!(post_as(&s, "/api/costs", "a1", Some(&t)).await, 403);
    assert_eq!(post_as(&s, "/api/skill-usage", "a1", Some(&t)).await, 403);
    assert_eq!(post_as(&s, "/api/sync/update", "a1", Some(&t)).await, 403);
}

#[tokio::test]
async fn a_token_writes_only_as_its_own_agent() {
    let s = start(tempfile::tempdir().unwrap()).await;
    let own = ["agents:register", "agents:heartbeat", "agents:data"];
    let paths = [
        "/api/agents/register",
        "/api/agents/{id}/heartbeat",
        "/api/agents/{id}/data",
    ];
    let t = token_with(&s, "a1", &own);
    for path in paths {
        assert_eq!(post_as(&s, path, "b2", Some(&t)).await, 403, "{path} as b2");
        assert!(
            post_as(&s, path, "a1", Some(&t)).await < 300,
            "{path} as a1"
        );
    }
    // The explicit override: the same rights plus agents:any.
    let any = token_with(&s, "a1", &[own.as_slice(), &["agents:any"]].concat());
    for path in paths {
        assert!(
            post_as(&s, path, "b2", Some(&any)).await < 300,
            "{path} any"
        );
    }
    // agents:any alone is no right to the route.
    let only_any = token_with(&s, "a1", &["agents:any"]);
    assert_eq!(
        post_as(&s, "/api/agents/{id}/data", "b2", Some(&only_any)).await,
        403
    );
}

/// The mutating routes named in src/server/api.rs, read from the source: a
/// route added there without a right in the table is refused (403) even with
/// every right, so this fails until the table has it.
#[tokio::test]
async fn every_mutating_route_in_the_router_has_a_right() {
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/server/api.rs")).unwrap();
    let body = &src[src.find("pub fn build_routes").unwrap()..];
    let body = &body[..body.find("\n}\n").unwrap()];
    let open = warte::auth::PROBE_OPEN_PATHS;
    let mut seen = Vec::new();
    for chunk in body.split(".route(").skip(1) {
        let chunk = &chunk[..chunk.find(");").map_or(chunk.len(), |i| i + 2)];
        let path = chunk.split('"').nth(1).unwrap();
        let mutates = ["post(", "put(", "patch(", "delete("].iter().any(|m| {
            chunk
                .match_indices(m)
                .any(|(i, _)| !chunk[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
        });
        if mutates {
            seen.push(format!("/api{path}"));
        }
    }
    seen.retain(|p| !open.contains(&p.as_str()));
    assert!(seen.len() >= ROUTES.len(), "scraped too little: {seen:?}");
    let s = start(tempfile::tempdir().unwrap()).await;
    let t = token_with(&s, "a1", &ALL_RIGHTS);
    for p in &seen {
        assert_ne!(
            post_as(&s, p, "a1", Some(&t)).await,
            403,
            "{p} has no right in the table (or refuses even a token with every right)"
        );
        assert!(
            ROUTES.iter().any(|(r, _)| r == p),
            "{p} missing from ROUTES"
        );
    }
}

fn mint_cli(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::fs::write(
        dir.join("warte.toml"),
        format!(
            "ca_cert_path = {:?}\nca_key_path = {:?}\n",
            dir.join("ca.pem"),
            dir.join("ca-key.pem")
        ),
    )
    .unwrap();
    std::process::Command::new(env!("CARGO_BIN_EXE_warte"))
        .args(["token", "mint", "agent-7"])
        .args(args)
        .current_dir(dir)
        .env_remove("WARTE_TOKEN_SEED_PATH")
        .output()
        .unwrap()
}

fn rights_of(dir: &std::path::Path, out: &std::process::Output) -> Vec<String> {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let seed = load_or_create_seed(&dir.join("token-seed")).unwrap();
    let token = String::from_utf8(out.stdout.clone()).unwrap();
    TokenManager::from_seed(&seed, 24)
        .verify_token(token.trim())
        .unwrap()
        .rights
}

#[test]
fn mint_flags_add_rights_and_the_default_stays() {
    let dir = tempfile::tempdir().unwrap();
    let has_default = |r: &[String]| {
        warte::auth::AGENT_RIGHTS
            .iter()
            .all(|d| r.contains(&d.to_string()))
    };
    let default = rights_of(dir.path(), &mint_cli(dir.path(), &[]));
    assert_eq!(default, warte::auth::AGENT_RIGHTS);
    let any = rights_of(dir.path(), &mint_cli(dir.path(), &["--any-agent"]));
    assert!(any.contains(&"agents:any".to_string()) && has_default(&any));
    let more = rights_of(
        dir.path(),
        &mint_cli(
            dir.path(),
            &["--right", "costs:write", "--right", "sync:update"],
        ),
    );
    assert!(more.contains(&"costs:write".to_string()));
    assert!(more.contains(&"sync:update".to_string()));
    assert!(!more.contains(&"agents:any".to_string()) && has_default(&more));
}

#[test]
fn mint_rejects_an_unknown_right_and_prints_no_token() {
    let dir = tempfile::tempdir().unwrap();
    // A known right is accepted, so the rejection below is about the name.
    let ok = mint_cli(dir.path(), &["--right", "costs:write"]);
    assert!(ok.status.success());
    let bad = mint_cli(dir.path(), &["--right", "costs:wrte"]);
    assert!(!bad.status.success() && bad.stdout.is_empty());
    let err = String::from_utf8_lossy(&bad.stderr);
    assert!(
        err.contains("unknown right") && err.contains("costs:wrte"),
        "{err}"
    );
}

#[test]
fn docs_and_contract_list_each_routes_right() {
    let docs = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/docs/topics/api-reference.md"
    ))
    .unwrap();
    let contract = serde_json::to_string(&warte::contract::current_contract()).unwrap();
    for (path, right) in ROUTES {
        assert!(docs.contains(right), "docs omit {right} ({path})");
        assert!(contract.contains(right), "contract omits {right} ({path})");
    }
    assert!(docs.contains("agents:any") && docs.contains("--any-agent"));
}

/// Register `name` with `bearer`; the id the server assigned.
async fn register_id(s: &Server, name: &str, bearer: &str) -> String {
    let r = reqwest::Client::new()
        .post(format!("{}/api/agents/register", s.base))
        .bearer_auth(bearer)
        .json(&serde_json::json!({"name": name, "identity_type": "token", "public_key": "k"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    r.json::<serde_json::Value>().await.unwrap()["agent_id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The real client flow (factory-notice): a default token for a name, register
/// that name, then write to the UUID the server assigned. The token names the
/// agent; the path names its row; the row's name must match.
#[tokio::test]
async fn a_default_token_writes_to_the_uuid_its_own_registration_got() {
    let s = start(tempfile::tempdir().unwrap()).await;
    let t = mint_agent_token(&config_in(s.dir.path()), "notice-repo", 24).unwrap();
    let mine = register_id(&s, "notice-repo", &t).await;
    assert_ne!(mine, "notice-repo", "the server assigns its own id");
    let other_t = mint_agent_token(&config_in(s.dir.path()), "other-repo", 24).unwrap();
    let theirs = register_id(&s, "other-repo", &other_t).await;
    for tail in ["heartbeat", "data"] {
        let path = format!("/api/agents/{mine}/{tail}");
        assert!(
            post_as(&s, &path, "", Some(&t)).await < 300,
            "{tail} as self"
        );
        let path = format!("/api/agents/{theirs}/{tail}");
        assert_eq!(
            post_as(&s, &path, "", Some(&t)).await,
            403,
            "{tail} as other"
        );
    }
    // An id no row has is refused, not a server error.
    let none = "/api/agents/00000000-0000-0000-0000-000000000000/data";
    assert_eq!(post_as(&s, none, "", Some(&t)).await, 403);
    // agents:any is the override, and an unknown id is then a 404, not a 500.
    let any = token_with(&s, "notice-repo", &["agents:data", "agents:any"]);
    let path = format!("/api/agents/{theirs}/data");
    assert!(post_as(&s, &path, "", Some(&any)).await < 300);
    assert_eq!(post_as(&s, none, "", Some(&any)).await, 404);
}
