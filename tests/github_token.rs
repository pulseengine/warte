//! Per-agent GitHub tokens (REQ-GHTOKEN-001..004, #189), through warte's real
//! router and auth middleware, against a local mock of GitHub's App API that
//! verifies the App JWT with the public key.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, post},
    Json, Router,
};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD as B64};
use base64::Engine as _;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use warte::auth::mint_agent_token_with;
use warte::ghtoken::{
    git_credential_answer, git_credential_wants, token_agent, Broker, GithubAppConfig, Grant, Store,
};

const APP_ID: u64 = 4242;
const INST: u64 = 77;
const PK8: &[u8] = include_bytes!("fixtures/oidc-test-rsa.pk8");

#[derive(Default)]
struct Gh {
    asked: Vec<Value>,
    revoked: Vec<String>,
    minted: u32,
    overgrant: bool,
}

#[derive(Clone)]
struct Mock {
    gh: Arc<Mutex<Gh>>,
    public: Arc<Vec<u8>>,
}

fn jwt_ok(m: &Mock, headers: &HeaderMap) -> bool {
    let Some(jwt) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    let msg = format!("{}.{}", parts[0], parts[1]);
    let Ok(sig) = B64.decode(parts[2]) else {
        return false;
    };
    let key = ring::signature::UnparsedPublicKey::new(
        &ring::signature::RSA_PKCS1_2048_8192_SHA256,
        m.public.as_slice(),
    );
    if key.verify(msg.as_bytes(), &sig).is_err() {
        return false;
    }
    let claims: Value = serde_json::from_slice(&B64.decode(parts[1]).unwrap()).unwrap();
    claims["iss"] == json!(APP_ID.to_string())
        && claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap() == 600
}

async fn access_tokens(
    State(m): State<Mock>,
    Path(inst): Path<u64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if inst != INST || !jwt_ok(&m, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut g = m.gh.lock().unwrap();
    g.minted += 1;
    g.asked.push(body.clone());
    let mut perms = body["permissions"].clone();
    if g.overgrant {
        perms["administration"] = json!("write");
    }
    let exp = chrono::Utc::now() + chrono::Duration::hours(1);
    (
        StatusCode::CREATED,
        Json(json!({"token": format!("ghs_planted{}", g.minted),
                    "expires_at": exp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    "permissions": perms})),
    )
        .into_response()
}

async fn revoke(State(m): State<Mock>, headers: HeaderMap) -> Response {
    let t = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_string();
    m.gh.lock().unwrap().revoked.push(t);
    StatusCode::NO_CONTENT.into_response()
}

/// The PKCS#1 RSAPrivateKey inside a PKCS#8 PrivateKeyInfo: the last element,
/// an OCTET STRING, of the outer SEQUENCE.
fn pkcs1_of(pk8: &[u8]) -> Vec<u8> {
    fn header(b: &[u8], i: usize) -> (usize, usize) {
        let l = b[i + 1] as usize;
        if l < 0x80 {
            (i + 2, l)
        } else {
            let n = l & 0x7f;
            let len = b[i + 2..i + 2 + n]
                .iter()
                .fold(0usize, |a, x| (a << 8) | *x as usize);
            (i + 2 + n, len)
        }
    }
    let (mut i, _) = header(pk8, 0); // SEQUENCE
    for _ in 0..2 {
        let (s, l) = header(pk8, i); // INTEGER version, AlgorithmIdentifier
        i = s + l;
    }
    assert_eq!(pk8[i], 0x04, "OCTET STRING");
    let (s, l) = header(pk8, i);
    pk8[s..s + l].to_vec()
}

fn pem(kind: &str, der: &[u8]) -> String {
    let b = STANDARD.encode(der);
    let lines: Vec<&str> = b
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    format!(
        "-----BEGIN {kind}-----\n{}\n-----END {kind}-----\n",
        lines.join("\n")
    )
}

fn write_key(dir: &std::path::Path, name: &str, text: &str, mode: u32) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    p
}

fn app_config(dir: &std::path::Path, api: &str) -> GithubAppConfig {
    // GitHub hands out PKCS#1 ("BEGIN RSA PRIVATE KEY"); that is the path tested.
    let key = write_key(
        dir,
        "app.pem",
        &pem("RSA PRIVATE KEY", &pkcs1_of(PK8)),
        0o600,
    );
    toml::from_str(&format!(
        "app_id = {APP_ID}\ninstallation_id = {INST}\naccount = \"pulseengine\"\nkey_path = {:?}\napi_base = {api:?}\n",
        key.display().to_string()
    ))
    .unwrap()
}

struct T {
    dir: tempfile::TempDir,
    base: String,
    config: warte::Config,
    gh: Arc<Mutex<Gh>>,
    broker: Arc<Broker>,
    store: Store,
}

async fn start(per_agent: u32) -> T {
    let mock = Mock {
        gh: Arc::new(Mutex::new(Gh::default())),
        public: Arc::new(
            ring::signature::RsaKeyPair::from_pkcs8(PK8)
                .unwrap()
                .public()
                .as_ref()
                .to_vec(),
        ),
    };
    let gh_app = Router::new()
        .route(
            "/app/installations/{inst}/access_tokens",
            post(access_tokens),
        )
        .route("/installation/token", delete(revoke))
        .with_state(mock.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(l, gh_app).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let mut g = app_config(dir.path(), &api);
    g.per_agent_per_hour = per_agent;
    let config = warte::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        github_app: Some(g),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    for id in ["a1", "b2"] {
        sqlx::query(
            "INSERT OR IGNORE INTO agents (id, name, identity_type, public_key, access_rights, \
             health_status, instance_id, created_at, updated_at) \
             VALUES (?, ?, 'token', x'6b', '[]', 'unknown', 'test', 'now', 'now')",
        )
        .bind(format!("row-{id}"))
        .bind(id)
        .execute(db.pool())
        .await
        .unwrap();
    }
    let store = Store::new(db.pool().clone());
    let state = warte::server::build_state(config.clone(), db).unwrap();
    let broker = state.github_tokens.clone().expect("broker built");
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    T {
        dir,
        base,
        config,
        gh: mock.gh,
        broker,
        store,
    }
}

fn tok(t: &T, agent: &str, rights: &[&str]) -> String {
    let extra: Vec<String> = rights.iter().map(|s| s.to_string()).collect();
    mint_agent_token_with(&t.config, agent, 1, &extra).unwrap()
}

async fn ask(t: &T, path_agent: &str, bearer: Option<&str>, body: Value) -> (u16, Value) {
    let mut r = reqwest::Client::new()
        .post(format!("{}/api/agents/{path_agent}/github-token", t.base))
        .json(&body);
    if let Some(b) = bearer {
        r = r.bearer_auth(b);
    }
    let r = r.send().await.unwrap();
    let s = r.status().as_u16();
    if s == 200 {
        assert_eq!(r.headers()["cache-control"], "no-store");
    }
    (s, r.json().await.unwrap_or(Value::Null))
}

async fn assign(t: &T, agent: &str, repos: &[&str]) {
    let v: Vec<String> = repos.iter().map(|s| s.to_string()).collect();
    t.store
        .set_repos(agent, &v, Some("pulseengine"))
        .await
        .unwrap();
}

// rivet: verifies REQ-GHTOKEN-001
// rivet: verifies REQ-GHTOKEN-002
#[tokio::test]
async fn a_token_reaches_only_the_operator_assigned_repos_and_requests_only_narrow() {
    let t = start(12).await;
    let a1 = tok(&t, "a1", &["github:token"]);
    // planted: nothing assigned yet
    let (s, v) = ask(&t, "a1", Some(&a1), json!({})).await;
    assert_eq!(s, 403, "{v}");
    assert!(v["error"].as_str().unwrap().contains("operator"));

    assign(&t, "a1", &["pulseengine/warte", "pulseengine/rivet"]).await;
    let (s, v) = ask(&t, "a1", Some(&a1), json!({})).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["token"], "ghs_planted1");
    assert_eq!(
        v["repositories"],
        json!(["pulseengine/rivet", "pulseengine/warte"])
    );
    {
        let g = t.gh.lock().unwrap();
        assert_eq!(g.asked[0]["repositories"], json!(["rivet", "warte"]));
        assert_eq!(
            g.asked[0]["permissions"],
            json!({"actions":"read","contents":"write","issues":"write","metadata":"read","pull_requests":"write"})
        );
    }
    // planted: a repository that is not assigned, a permission above the set
    let (s, _) = ask(
        &t,
        "a1",
        Some(&a1),
        json!({"repositories":["pulseengine/sigil"]}),
    )
    .await;
    assert_eq!(s, 403);
    let (s, _) = ask(
        &t,
        "a1",
        Some(&a1),
        json!({"permissions":{"actions":"write"}}),
    )
    .await;
    assert_eq!(s, 403);
    let (s, _) = ask(
        &t,
        "a1",
        Some(&a1),
        json!({"permissions":{"administration":"read"}}),
    )
    .await;
    assert_eq!(s, 403);
    let (s, _) = ask(&t, "a1", Some(&a1), json!({"surprise":1})).await;
    assert_eq!(s, 400, "unknown fields are refused");
    // narrowing works
    let (s, v) = ask(
        &t,
        "a1",
        Some(&a1),
        json!({"repositories":["pulseengine/warte"],"permissions":{"contents":"read"}}),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["permissions"], json!({"contents":"read"}));
    assert_eq!(
        t.gh.lock().unwrap().asked[1]["repositories"],
        json!(["warte"])
    );
    // planted: the operator store refuses a repository outside the account
    let bad = vec!["someone-else/repo".to_string()];
    assert!(t
        .store
        .set_repos("a1", &bad, Some("pulseengine"))
        .await
        .is_err());
}

// rivet: verifies REQ-GHTOKEN-001
#[tokio::test]
async fn no_token_mints_for_another_agent_not_even_with_agents_any() {
    let t = start(12).await;
    assign(&t, "b2", &["pulseengine/warte"]).await;
    let any = tok(&t, "a1", &["github:token", "agents:any"]);
    let plain = tok(&t, "b2", &[]);
    let (s, _) = ask(&t, "b2", Some(&any), json!({})).await;
    assert_eq!(s, 403, "agents:any never lifts the github-token binding");
    let (s, _) = ask(&t, "b2", Some(&plain), json!({})).await;
    assert_eq!(s, 403, "the right is not a default");
    let (s, _) = ask(&t, "b2", None, json!({})).await;
    assert_eq!(s, 401);
    assert_eq!(t.gh.lock().unwrap().minted, 0, "GitHub was never asked");
    // a token for a name with no agents row (removed) mints nothing
    let ghost = tok(&t, "ghost", &["github:token"]);
    assign(&t, "ghost", &["pulseengine/warte"]).await;
    let (s, _) = ask(&t, "ghost", Some(&ghost), json!({})).await;
    assert_eq!(s, 403);
}

// rivet: verifies REQ-GHTOKEN-003
#[tokio::test]
async fn a_live_token_is_reused_and_the_hourly_limit_holds() {
    let t = start(2).await;
    assign(&t, "a1", &["pulseengine/warte"]).await;
    let a1 = tok(&t, "a1", &["github:token"]);
    let (_, first) = ask(&t, "a1", Some(&a1), json!({})).await;
    let (_, again) = ask(&t, "a1", Some(&a1), json!({})).await;
    assert_eq!(
        first["token"], again["token"],
        "same scope, time left: reused"
    );
    assert_eq!(t.gh.lock().unwrap().minted, 1);
    let (s, _) = ask(
        &t,
        "a1",
        Some(&a1),
        json!({"permissions":{"contents":"read"}}),
    )
    .await;
    assert_eq!(s, 200);
    let (s, v) = ask(
        &t,
        "a1",
        Some(&a1),
        json!({"permissions":{"issues":"read"}}),
    )
    .await;
    assert_eq!(s, 429, "{v}");
    assert_eq!(t.gh.lock().unwrap().minted, 2);
}

// rivet: verifies REQ-GHTOKEN-003
#[tokio::test]
async fn nothing_is_at_rest_and_the_audit_never_holds_the_token() {
    let t = start(12).await;
    assign(&t, "a1", &["pulseengine/warte"]).await;
    let a1 = tok(&t, "a1", &["github:token"]);
    let (s, _) = ask(&t, "a1", Some(&a1), json!({})).await;
    assert_eq!(s, 200);
    let (s, _) = ask(
        &t,
        "a1",
        Some(&a1),
        json!({"repositories":["pulseengine/nope"]}),
    )
    .await;
    assert_eq!(s, 403);
    let audit = std::fs::read_to_string(t.dir.path().join("github-tokens.jsonl")).unwrap();
    let outcomes: Vec<String> = audit
        .lines()
        .map(|l| {
            serde_json::from_str::<Value>(l).unwrap()["outcome"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(outcomes, ["issued", "refused"]);
    // every file warte wrote, the database included, is searched for the token
    for e in std::fs::read_dir(t.dir.path()).unwrap() {
        let p = e.unwrap().path();
        let bytes = std::fs::read(&p).unwrap();
        assert!(
            !bytes.windows(11).any(|w| w == b"ghs_planted"),
            "{} holds the token",
            p.display()
        );
    }
}

// rivet: verifies REQ-GHTOKEN-003
#[tokio::test]
async fn a_removed_or_narrowed_agent_has_its_live_tokens_revoked() {
    let t = start(12).await;
    assign(&t, "a1", &["pulseengine/warte", "pulseengine/rivet"]).await;
    let a1 = tok(&t, "a1", &["github:token"]);
    let (_, v) = ask(&t, "a1", Some(&a1), json!({})).await;
    let now = chrono::Utc::now().timestamp();
    t.broker.sweep(now).await;
    assert!(
        t.gh.lock().unwrap().revoked.is_empty(),
        "nothing changed, nothing revoked"
    );
    // planted: the operator narrows the scope
    assign(&t, "a1", &["pulseengine/warte"]).await;
    t.broker.sweep(now).await;
    assert_eq!(t.gh.lock().unwrap().revoked, [v["token"].as_str().unwrap()]);
    assert!(t.broker.live_counts(now).is_empty());
    // planted: the agents row disappears
    let (_, w) = ask(&t, "a1", Some(&a1), json!({})).await;
    sqlx::query("DELETE FROM agents WHERE name = 'a1'")
        .execute(
            &sqlx::SqlitePool::connect(&format!("sqlite:{}", t.config.db_path.display()))
                .await
                .unwrap(),
        )
        .await
        .unwrap();
    t.broker.sweep(now).await;
    assert_eq!(
        t.gh.lock().unwrap().revoked.last().unwrap(),
        w["token"].as_str().unwrap()
    );
    let audit = std::fs::read_to_string(t.dir.path().join("github-tokens.jsonl")).unwrap();
    assert_eq!(audit.matches("\"revoked\"").count(), 2);
}

// rivet: verifies REQ-GHTOKEN-002
#[tokio::test]
async fn actions_write_is_opt_in_per_agent() {
    let t = start(12).await;
    assign(&t, "a1", &["pulseengine/warte"]).await;
    let a1 = tok(&t, "a1", &["github:token"]);
    let want = json!({"permissions":{"actions":"write"}});
    let (s, _) = ask(&t, "a1", Some(&a1), want.clone()).await;
    assert_eq!(s, 403);
    t.store.set_actions_write("a1", true).await.unwrap();
    let (s, v) = ask(&t, "a1", Some(&a1), want).await;
    assert_eq!(s, 200, "{v}");
    t.store.set_actions_write("a1", false).await.unwrap();
    t.broker.sweep(chrono::Utc::now().timestamp()).await;
    assert_eq!(t.gh.lock().unwrap().revoked, [v["token"].as_str().unwrap()]);
    // the configured set can never carry it
    let mut g = app_config(t.dir.path(), "https://api.github.com");
    g.permissions.insert("actions".into(), "write".into());
    assert!(g.check().unwrap_err().contains("per agent"));
}

// rivet: verifies REQ-GHTOKEN-005
#[tokio::test]
async fn workflows_write_is_opt_in_per_agent_and_never_configured() {
    let t = start(12).await;
    assign(&t, "a1", &["pulseengine/warte"]).await;
    assign(&t, "a2", &["pulseengine/warte"]).await;
    let a1 = tok(&t, "a1", &["github:token"]);
    let a2 = tok(&t, "a2", &["github:token"]);
    let want = json!({"permissions":{"workflows":"write","contents":"write"}});
    // not granted: asking is refused, and the default set leaves it out
    let (s, _) = ask(&t, "a1", Some(&a1), want.clone()).await;
    assert_eq!(s, 403);
    let (s, v) = ask(&t, "a1", Some(&a1), json!({})).await;
    assert_eq!(s, 200, "{v}");
    assert!(v["permissions"].get("workflows").is_none(), "{v}");
    // granted to a1 only
    t.store.set_workflows_write("a1", true).await.unwrap();
    let (s, v) = ask(&t, "a1", Some(&a1), want.clone()).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["permissions"]["workflows"], "write", "{v}");
    let granted = v["token"].as_str().unwrap().to_string();
    let (s, _) = ask(&t, "a2", Some(&a2), want).await;
    assert_eq!(s, 403, "a grant is per agent");
    // the grants are separate: workflows does not bring actions write
    let (s, _) = ask(
        &t,
        "a1",
        Some(&a1),
        json!({"permissions":{"actions":"write"}}),
    )
    .await;
    assert_eq!(s, 403);
    let shown = t.store.get("a1").await.unwrap().unwrap();
    assert!(shown.workflows_write && !shown.actions_write);
    // taken back: the live token carrying it is revoked by the sweep
    t.store.set_workflows_write("a1", false).await.unwrap();
    t.broker.sweep(chrono::Utc::now().timestamp()).await;
    assert!(
        t.gh.lock().unwrap().revoked.contains(&granted),
        "the workflows token must be revoked"
    );
    // the configured set can never carry it, at any level
    for level in ["read", "write"] {
        let mut g = app_config(t.dir.path(), "https://api.github.com");
        g.permissions.insert("workflows".into(), level.into());
        assert!(g.check().unwrap_err().contains("per agent"), "{level}");
    }
    // only the two named grants exist
    assert_eq!(Grant::parse("workflows:write"), Some(Grant::WorkflowsWrite));
    assert_eq!(Grant::parse("actions:write"), Some(Grant::ActionsWrite));
    for bad in [
        "administration:write",
        "workflows:read",
        "workflows",
        "secrets:write",
    ] {
        assert_eq!(Grant::parse(bad), None, "{bad}");
    }
}

// rivet: verifies REQ-GHTOKEN-003
#[tokio::test]
async fn github_granting_more_than_asked_is_revoked_and_refused() {
    let t = start(12).await;
    assign(&t, "a1", &["pulseengine/warte"]).await;
    t.gh.lock().unwrap().overgrant = true;
    let a1 = tok(&t, "a1", &["github:token"]);
    let (s, v) = ask(&t, "a1", Some(&a1), json!({})).await;
    assert_eq!(s, 502, "{v}");
    assert_eq!(t.gh.lock().unwrap().revoked, ["ghs_planted1"]);
}

// rivet: verifies REQ-GHTOKEN-001
#[tokio::test]
async fn the_config_and_key_are_refused_when_unsafe() {
    let dir = tempfile::tempdir().unwrap();
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = || Store::new(pool.clone());
    let ok = app_config(dir.path(), "https://api.github.com");
    assert!(ok.check().is_ok());
    // planted: plain http off loopback, an unknown permission, silly limits
    let mut bad = ok.clone();
    bad.api_base = "http://example.org".into();
    assert!(bad.check().is_err());
    let mut bad = ok.clone();
    bad.permissions
        .insert("administration".into(), "read".into());
    assert!(bad.check().is_err());
    let mut bad = ok.clone();
    bad.per_agent_per_hour = 0;
    assert!(bad.check().is_err());
    // planted: a key other users can read; a key file that is not a key
    let mut bad = ok.clone();
    bad.key_path = write_key(
        dir.path(),
        "open.pem",
        &pem("RSA PRIVATE KEY", &pkcs1_of(PK8)),
        0o644,
    );
    let e = Broker::new(bad, store(), dir.path()).err().unwrap();
    assert!(e.contains("readable by other users"), "{e}");
    let mut bad = ok.clone();
    bad.key_path = write_key(dir.path(), "junk.pem", "SECRET-LOOKING-JUNK", 0o600);
    let e = Broker::new(bad, store(), dir.path()).err().unwrap();
    assert!(
        !e.contains("SECRET-LOOKING-JUNK"),
        "never quotes the file: {e}"
    );
    // PKCS#8 is accepted too
    let mut p8 = ok.clone();
    p8.key_path = write_key(dir.path(), "p8.pem", &pem("PRIVATE KEY", PK8), 0o600);
    assert!(Broker::new(p8, store(), dir.path()).is_ok());
    // without [github_app] the route answers 404
    let cfg = warte::Config {
        db_path: dir.path().join("plain.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&cfg.db_path).await.unwrap();
    let token = mint_agent_token_with(&cfg, "a1", 1, &["github:token".into()]).unwrap();
    let app = warte::server::build_router(warte::server::build_state(cfg, db).unwrap());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let s = reqwest::Client::new()
        .post(format!("{base}/api/agents/a1/github-token"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(s, 404);
}

// rivet: verifies REQ-GHTOKEN-004
#[tokio::test]
async fn the_client_and_git_helper_get_a_token_without_a_file() {
    let t = start(12).await;
    assign(&t, "a1", &["pulseengine/warte"]).await;
    let a1 = tok(&t, "a1", &["github:token"]);
    assert_eq!(token_agent(&a1).as_deref(), Some("a1"));
    assert_eq!(token_agent("ghp_notwarte"), None);
    let got = warte::ghtoken::fetch(&t.base, None, &a1, &[])
        .await
        .unwrap();
    assert_eq!(got.token, "ghs_planted1");
    let err = warte::ghtoken::fetch(&t.base, None, &a1, &["pulseengine/sigil".into()])
        .await
        .unwrap_err();
    assert!(err.contains("403") && err.contains("not assigned"), "{err}");
    // git's credential protocol: https on github.com only
    assert!(git_credential_wants(
        "protocol=https\nhost=github.com\npath=pulseengine/warte.git\n\n"
    ));
    assert!(!git_credential_wants("protocol=https\nhost=gitlab.com\n\n"));
    assert!(!git_credential_wants("protocol=http\nhost=github.com\n\n"));
    assert_eq!(
        git_credential_answer("ghs_x"),
        "username=x-access-token\npassword=ghs_x\n"
    );
    let before: Vec<_> = std::fs::read_dir(t.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let _ = warte::ghtoken::fetch(&t.base, None, &a1, &[])
        .await
        .unwrap();
    let after: Vec<_> = std::fs::read_dir(t.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(before, after, "the client wrote no file");
}
