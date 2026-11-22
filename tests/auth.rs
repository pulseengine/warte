//! Login through an OIDC provider (DD-AUTH-001, REQ-AUTH-001..003), against a
//! mock Zitadel that signs with a test-only RSA key (tests/fixtures).

use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use warte::hub::now;
use warte::login::{class_of, AuthConfig, Class};

const PROJECT: &str = "proj123";
const CLIENT: &str = "client456";

fn rsa() -> ring::signature::RsaKeyPair {
    ring::signature::RsaKeyPair::from_pkcs8(include_bytes!("fixtures/oidc-test-rsa.pk8")).unwrap()
}

fn sign(header: Value, claims: Value) -> String {
    let h = B64.encode(header.to_string());
    let c = B64.encode(claims.to_string());
    let msg = format!("{h}.{c}");
    let kp = rsa();
    let mut sig = vec![0; kp.public().modulus_len()];
    kp.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        msg.as_bytes(),
        &mut sig,
    )
    .unwrap();
    format!("{msg}.{}", B64.encode(sig))
}

fn claims(iss: &str, role: Option<&str>) -> Value {
    let mut c = json!({"iss": iss, "sub": "u1", "aud": [CLIENT, PROJECT], "exp": now() + 600,
                       "iat": now(), "preferred_username": "owner"});
    if let Some(r) = role {
        c[format!("urn:zitadel:iam:org:project:{PROJECT}:roles")] = json!({ r: {"org1": "home"} });
    }
    c
}

fn rs256() -> Value {
    json!({"alg": "RS256", "kid": "k1", "typ": "JWT"})
}

#[derive(Clone, Default)]
struct Idp {
    base: Arc<Mutex<String>>,
    /// challenge and nonce per code, as the authorize step recorded them
    codes: Arc<Mutex<HashMap<String, (String, String)>>>,
    wrong_nonce: Arc<Mutex<bool>>,
    /// The subject the next id token names; empty: "u1".
    sub: Arc<Mutex<String>>,
    /// The role the next id token carries; empty: "owner".
    role: Arc<Mutex<String>>,
}

async fn discovery(State(i): State<Idp>) -> Response {
    let b = i.base.lock().unwrap().clone();
    axum::Json(
        json!({"issuer": b, "authorization_endpoint": format!("{b}/authorize"),
        "token_endpoint": format!("{b}/token"), "jwks_uri": format!("{b}/jwks"),
        "end_session_endpoint": format!("{b}/end")}),
    )
    .into_response()
}

async fn jwks() -> Response {
    let pk = ring::rsa::PublicKeyComponents::<Vec<u8>>::from(rsa().public());
    axum::Json(
        json!({"keys": [{"kty": "RSA", "kid": "k1", "alg": "RS256", "use": "sig",
        "n": B64.encode(&pk.n), "e": B64.encode(&pk.e)}]}),
    )
    .into_response()
}

async fn token(State(i): State<Idp>, Form(f): Form<HashMap<String, String>>) -> Response {
    use sha2::Digest;
    let code = f.get("code").cloned().unwrap_or_default();
    let Some((challenge, nonce)) = i.codes.lock().unwrap().remove(&code) else {
        return (StatusCode::BAD_REQUEST, "bad code").into_response();
    };
    let v = f.get("code_verifier").cloned().unwrap_or_default();
    if B64.encode(sha2::Sha256::digest(v.as_bytes())) != challenge {
        return (StatusCode::BAD_REQUEST, "PKCE verifier does not match").into_response();
    }
    if f.get("client_id").map(String::as_str) != Some(CLIENT)
        || f.get("grant_type").map(String::as_str) != Some("authorization_code")
    {
        return (StatusCode::BAD_REQUEST, "bad client").into_response();
    }
    let b = i.base.lock().unwrap().clone();
    let role = i.role.lock().unwrap().clone();
    let mut c = claims(
        &b,
        Some(if role.is_empty() {
            "owner"
        } else {
            role.as_str()
        }),
    );
    let sub = i.sub.lock().unwrap().clone();
    if !sub.is_empty() {
        c["sub"] = sub.into();
    }
    c["nonce"] = if *i.wrong_nonce.lock().unwrap() {
        "other".into()
    } else {
        nonce.into()
    };
    axum::Json(json!({"id_token": sign(rs256(), c), "access_token": "x", "token_type": "Bearer"}))
        .into_response()
}

async fn idp() -> (Idp, String) {
    let i = Idp::default();
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/token", post(token))
        .with_state(i.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    *i.base.lock().unwrap() = base.clone();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (i, base)
}

struct Server {
    _dir: tempfile::TempDir,
    base: String,
    agent: String,
    briefing_job: String,
    /// "a1" with agents:read and the agent rights; "a2" the same; "ops"
    /// with agents:read and agents:any.
    reader: String,
    reader2: String,
    reader_any: String,
    /// "a1" with agents:read, working for the login subject "u1" (`--owner u1`).
    owned_by_u1: String,
    /// "ops" with architecture:read only.
    arch_reader: String,
    /// "seam-job" with reports:write:seam-check only.
    reporter: String,
}

async fn serve(auth: Option<AuthConfig>) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        auth,
        architecture: Some(warte::config::ArchitectureConfig {
            file: concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/architecture.example.json"
            )
            .into(),
        }),
        ..Default::default()
    };
    let db = warte::db::Database::connect(&config.db_path).await.unwrap();
    let state = warte::server::build_state(config, db).unwrap();
    let agent = state
        .tokens
        .issue_token("a1", vec!["agents:data".into()])
        .unwrap();
    let briefing_job = state
        .tokens
        .issue_token("b1", vec!["briefing:write".into()])
        .unwrap();
    let read = |who: &str, extra: &[&str]| {
        let mut r: Vec<String> = [
            "agents:register",
            "agents:data",
            "agents:read",
            "agents:heartbeat",
        ]
        .iter()
        .chain(extra)
        .map(|s| s.to_string())
        .collect();
        r.dedup();
        state.tokens.issue_token(who, r).unwrap()
    };
    let (reader, reader2, reader_any) = (
        read("a1", &[]),
        read("a2", &[]),
        read("ops", &["agents:any"]),
    );
    let reporter = state
        .tokens
        .issue_token("seam-job", vec!["reports:write:seam-check".into()])
        .unwrap();
    let arch_reader = state
        .tokens
        .issue_token("ops", vec!["architecture:read".into()])
        .unwrap();
    let owned_by_u1 = state
        .tokens
        .issue_token_owned("a1", vec!["agents:read".into()], Some("u1"))
        .unwrap();
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    Server {
        _dir: dir,
        base,
        agent,
        briefing_job,
        reader,
        reader2,
        reader_any,
        owned_by_u1,
        arch_reader,
        reporter,
    }
}

fn cfg(issuer: &str) -> AuthConfig {
    AuthConfig {
        issuer: issuer.into(),
        client_id: CLIENT.into(),
        project_id: Some(PROJECT.into()),
        base_url: "https://tower.example/warte".into(),
        roles: Default::default(),
        session_hours: 12,
        ca_bundle: None,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

async fn status(base: &str, path: &str, bearer: Option<&str>, cookie: Option<&str>) -> u16 {
    let mut r = client().get(format!("{base}{path}"));
    if let Some(b) = bearer {
        r = r.bearer_auth(b);
    }
    if let Some(c) = cookie {
        r = r.header("Cookie", c);
    }
    r.send().await.unwrap().status().as_u16()
}

// rivet: verifies REQ-AUTH-001
#[tokio::test]
async fn without_a_login_only_fleet_status_is_readable() {
    let (_i, iss) = idp().await;
    let s = serve(Some(cfg(&iss))).await;
    assert_eq!(status(&s.base, "/api/agents", None, None).await, 401);
    assert_eq!(status(&s.base, "/api/costs", None, None).await, 401);
    for open in [
        "/api/health",
        "/api/openapi.json",
        "/",
        "/idle.js",
        "/api/me",
    ] {
        assert_eq!(status(&s.base, open, None, None).await, 200, "{open}");
    }
    // fleet status answers as before (503 here: no ground station), never 401
    assert_ne!(status(&s.base, "/api/queue", None, None).await, 401);
    // a personal view inside the page says how to sign in
    let html = client()
        .get(format!("{}/api/dashboard/costs-partial", s.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        html.contains("Sign in") && html.contains("auth/login"),
        "{html}"
    );
    // an agent token is no login; with the job's right it reads what it writes
    assert_eq!(
        status(&s.base, "/api/agents", Some(&s.agent), None).await,
        403
    );
    assert_ne!(
        status(&s.base, "/api/briefing", Some(&s.briefing_job), None).await,
        401
    );
    assert_ne!(
        status(&s.base, "/api/briefing", Some(&s.briefing_job), None).await,
        403
    );

    // without [auth] nothing changes
    let open = serve(None).await;
    assert_eq!(status(&open.base, "/api/agents", None, None).await, 200);
    let me: Value = client()
        .get(format!("{}/api/me", open.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["auth"], false);
}

// rivet: verifies REQ-AUTH-002
// rivet: verifies DD-AUTH-001
#[tokio::test]
async fn a_provider_token_counts_only_when_every_check_holds() {
    let (_i, iss) = idp().await;
    let s = serve(Some(cfg(&iss))).await;
    let good = sign(rs256(), claims(&iss, Some("viewer")));
    assert_eq!(status(&s.base, "/api/agents", Some(&good), None).await, 200);

    let mut cases: Vec<(&str, String, u16)> = vec![];
    let mut c = claims(&iss, Some("viewer"));
    c["aud"] = json!(["someone-else"]);
    cases.push(("another audience", sign(rs256(), c), 401));
    let mut c = claims(&iss, Some("viewer"));
    c["exp"] = json!(now() - 3600);
    cases.push(("expired", sign(rs256(), c), 401));
    cases.push((
        "another issuer",
        sign(rs256(), claims("https://evil.example", Some("viewer"))),
        401,
    ));
    cases.push((
        "no warte role",
        sign(rs256(), claims(&iss, Some("cfo"))),
        403,
    ));
    cases.push(("no roles at all", sign(rs256(), claims(&iss, None)), 403));
    let none = format!(
        "{}.{}.",
        B64.encode(json!({"alg": "none"}).to_string()),
        B64.encode(claims(&iss, Some("owner")).to_string())
    );
    cases.push(("alg none", none, 401));
    let hs = format!(
        "{}.{}.{}",
        B64.encode(json!({"alg": "HS256", "kid": "k1"}).to_string()),
        B64.encode(claims(&iss, Some("owner")).to_string()),
        B64.encode(b"x")
    );
    cases.push(("HS256 (a shared secret warte does not have)", hs, 401));
    // a signature from the right key over other claims
    let other = sign(rs256(), claims(&iss, Some("viewer")));
    let forged = format!(
        "{}.{}.{}",
        other.split('.').next().unwrap(),
        B64.encode(claims(&iss, Some("owner")).to_string().replace("u1", "u2")),
        other.split('.').nth(2).unwrap()
    );
    cases.push(("tampered claims", forged, 401));
    let mut h = rs256();
    h["kid"] = "k9".into();
    cases.push(("unknown key", sign(h, claims(&iss, Some("viewer"))), 401));
    for (what, t, want) in cases {
        assert_eq!(
            status(&s.base, "/api/agents", Some(&t), None).await,
            want,
            "{what}"
        );
    }
    // a header alone is no identity
    let r = client()
        .get(format!("{}/api/agents", s.base))
        .header("X-Forwarded-User", "owner")
        .header("X-Auth-Request-Email", "owner@x")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
}

fn query(url: &str) -> HashMap<String, String> {
    url.split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("")
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), urldecode(v)))
        .collect()
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// /auth/login, then what the provider would do: record the challenge and
/// nonce under a code. Returns the state and the browser's login cookie.
async fn start_login(s: &Server, i: &Idp, back: &str, code: &str) -> (String, String) {
    let r = client()
        .get(format!("{}/auth/login?back={back}", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 303);
    let loc = r.headers()["location"].to_str().unwrap().to_string();
    let lc = r.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(
        lc.starts_with("warte_login=") && lc.contains("HttpOnly"),
        "{lc}"
    );
    let lc = lc.split(';').next().unwrap().to_string();
    let q = query(&loc);
    assert!(
        loc.starts_with(&format!("{}/authorize?", i.base.lock().unwrap())),
        "{loc}"
    );
    assert_eq!(q["code_challenge_method"], "S256");
    assert!(q["scope"].contains(&format!("urn:zitadel:iam:org:project:id:{PROJECT}:aud")));
    i.codes.lock().unwrap().insert(
        code.into(),
        (q["code_challenge"].clone(), q["nonce"].clone()),
    );
    (q["state"].clone(), lc)
}

async fn callback(s: &Server, code: &str, state: &str, login: Option<&str>) -> reqwest::Response {
    let mut r = client().get(format!(
        "{}/auth/callback?code={code}&state={state}",
        s.base
    ));
    if let Some(c) = login {
        r = r.header("Cookie", c);
    }
    r.send().await.unwrap()
}

// rivet: verifies REQ-AUTH-003
#[tokio::test]
async fn the_hand_over_ends_in_a_server_side_session() {
    let (i, iss) = idp().await;
    let s = serve(Some(cfg(&iss))).await;
    // the state of a login another browser started (login CSRF)
    let (st, _theirs) = start_login(&s, &i, "/", "c0").await;
    assert_eq!(callback(&s, "c0", &st, None).await.status().as_u16(), 401);
    let (_, mine) = start_login(&s, &i, "/", "c00").await;
    let (st, _) = start_login(&s, &i, "/", "c000").await;
    assert_eq!(
        callback(&s, "c000", &st, Some(&mine))
            .await
            .status()
            .as_u16(),
        401
    );

    // another site is no place to return to
    let (st, lc) = start_login(&s, &i, "https://evil.example/x", "c1").await;
    let r = callback(&s, "c1", &st, Some(&lc)).await;
    assert_eq!(r.status().as_u16(), 303);
    assert_eq!(r.headers()["location"], "/warte/");
    let set = r.headers()["set-cookie"].to_str().unwrap().to_string();
    for want in [
        "warte_session=",
        "HttpOnly",
        "Secure",
        "SameSite=Lax",
        "Path=/warte",
    ] {
        assert!(set.contains(want), "{want}: {set}");
    }
    let cookie = set.split(';').next().unwrap().to_string();
    assert_eq!(
        status(&s.base, "/api/agents", None, Some(&cookie)).await,
        200
    );
    let me: Value = client()
        .get(format!("{}/api/me", s.base))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        (me["user"]["name"].as_str(), me["user"]["role"].as_str()),
        (Some("owner"), Some("owner"))
    );
    // a state is good once; an unknown one never
    assert_eq!(
        callback(&s, "c1", &st, Some(&lc)).await.status().as_u16(),
        401
    );
    assert_eq!(
        callback(&s, "c1", "forged", Some(&lc))
            .await
            .status()
            .as_u16(),
        401
    );
    // an ID token for another login's nonce
    *i.wrong_nonce.lock().unwrap() = true;
    let (st, lc2) = start_login(&s, &i, "/", "c2").await;
    assert_eq!(
        callback(&s, "c2", &st, Some(&lc2)).await.status().as_u16(),
        401
    );
    *i.wrong_nonce.lock().unwrap() = false;
    // logout drops the session and goes to the provider's end-session page
    let r = client()
        .get(format!("{}/auth/logout", s.base))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert!(r.headers()["location"]
        .to_str()
        .unwrap()
        .starts_with(&format!("{iss}/end?")));
    assert!(r.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    assert_eq!(
        status(&s.base, "/api/agents", None, Some(&cookie)).await,
        401
    );
}

// rivet: verifies REQ-AUTH-003
#[tokio::test]
async fn back_is_a_local_path_even_at_the_root() {
    let (i, iss) = idp().await;
    let mut c = cfg(&iss);
    c.base_url = "https://tower.example".into();
    let s = serve(Some(c)).await;
    for (n, back) in [
        "https://evil.example/x",
        "//evil.example/x",
        "/%09/evil.example",
        "/%5Cevil.example",
        "javascript:alert(1)",
    ]
    .iter()
    .enumerate()
    {
        let code = format!("r{n}");
        let (st, lc) = start_login(&s, &i, back, &code).await;
        let r = callback(&s, &code, &st, Some(&lc)).await;
        assert_eq!(r.status().as_u16(), 303, "{back}");
        assert_eq!(r.headers()["location"], "/", "{back}");
    }
    let (st, lc) = start_login(&s, &i, "/x/y", "ok").await;
    let r = callback(&s, "ok", &st, Some(&lc)).await;
    assert_eq!(r.headers()["location"], "/x/y", "a local path is kept");
}

// rivet: verifies REQ-AUTH-002
#[tokio::test]
async fn only_this_project_s_roles_and_no_critical_extensions() {
    let (_i, iss) = idp().await;
    let s = serve(Some(cfg(&iss))).await;
    // a role of whichever project asked: the generic claim does not count
    let mut c = claims(&iss, None);
    c["urn:zitadel:iam:org:project:roles"] = json!({"owner": {"org1": "home"}});
    assert_eq!(
        status(&s.base, "/api/agents", Some(&sign(rs256(), c)), None).await,
        403
    );
    let mut h = rs256();
    h["crit"] = json!(["exp"]);
    let t = sign(h, claims(&iss, Some("owner")));
    assert_eq!(status(&s.base, "/api/agents", Some(&t), None).await, 401);
    // a look-alike of the local test issuer is no issuer, and plain http neither
    assert!(warte::login::Login::new(cfg("http://127.0.0.1.evil.net")).is_err());
    assert!(warte::login::Login::new(cfg("http://plain.example")).is_err());
    assert!(warte::login::Login::new(cfg("http://127.0.0.1:8080")).is_ok());
}

#[test]
fn no_route_is_in_two_classes() {
    use warte::login::{PUBLIC, SIGNED_IN, SIGNED_IN_OR_RIGHT};
    for p in PUBLIC {
        assert!(
            !SIGNED_IN.contains(p) && !SIGNED_IN_OR_RIGHT.iter().any(|(q, _)| q == p),
            "{p}"
        );
    }
    for p in SIGNED_IN {
        assert!(!SIGNED_IN_OR_RIGHT.iter().any(|(q, _)| q == p), "{p}");
    }
}

// rivet: verifies REQ-AUTH-001
#[test]
fn every_read_route_has_a_class() {
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/server/api.rs")).unwrap();
    let body = &src[src.find("pub fn build_routes").unwrap()..];
    let body = &body[..body.find("\n}\n").unwrap()];
    let mut paths = vec![];
    for chunk in body.split(".route(").skip(1) {
        let path = chunk.split('"').nth(1).unwrap();
        let chunk = &chunk[..chunk.find(");").unwrap_or(chunk.len())];
        if chunk.contains("get(") {
            // the API router is nested under /api; the dashboard shell is not
            let nested = !path.starts_with("/api/")
                && ![
                    "/",
                    "/dashboard.css",
                    "/htmx.min.js",
                    "/idle.js",
                    "/airport.js",
                ]
                .contains(&path);
            paths.push(if nested {
                format!("/api{path}")
            } else {
                path.to_string()
            });
        }
    }
    assert!(paths.len() > 40, "scraped too little: {paths:?}");
    let listed = |p: &str| {
        warte::login::PUBLIC.contains(&p)
            || warte::login::SIGNED_IN.contains(&p)
            || warte::login::SIGNED_IN_OR_RIGHT
                .iter()
                .any(|(q, _)| *q == p)
    };
    let missing: Vec<&String> = paths.iter().filter(|p| !listed(p)).collect();
    assert!(missing.is_empty(), "routes with no data class: {missing:?}");
    // an unknown path fails closed
    assert_eq!(class_of("/api/something-new"), Class::SignedIn);
    assert_eq!(class_of("/api/history/machines/gale/series"), Class::Public);
}

async fn register(s: &Server, name: &str, bearer: &str) -> String {
    let r: Value = client()
        .post(format!("{}/api/agents/register", s.base))
        .bearer_auth(bearer)
        .json(&json!({"name": name, "identity_type": "notice", "public_key": ""}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    r["agent_id"].as_str().unwrap().to_string()
}

// rivet: verifies REQ-API-005
#[tokio::test]
async fn agents_read_opens_the_list_and_only_the_tokens_own_agent() {
    let (_i, iss) = idp().await;
    let s = serve(Some(cfg(&iss))).await;
    let id1 = register(&s, "a1", &s.reader).await;
    let id2 = register(&s, "a2", &s.reader2).await;
    let own = [
        "/api/agents".to_string(),
        format!("/api/agents/{id1}"),
        format!("/api/agents/{id1}/data?data_type=notice"),
    ];
    for p in &own {
        assert_eq!(status(&s.base, p, Some(&s.reader), None).await, 200, "{p}");
    }
    // planted: another agent's row and data, an unknown id, a token without
    // the right, and a route agents:read does not name
    for p in [
        format!("/api/agents/{id2}"),
        format!("/api/agents/{id2}/data"),
        "/api/agents/no-such-id/data".to_string(),
        "/api/costs".to_string(),
        "/api/access/list/a1".to_string(),
    ] {
        assert_eq!(status(&s.base, &p, Some(&s.reader), None).await, 403, "{p}");
    }
    for p in &own {
        assert_eq!(status(&s.base, p, Some(&s.agent), None).await, 403, "{p}");
    }
    // agents:any widens it to every agent
    for p in [
        format!("/api/agents/{id2}"),
        format!("/api/agents/{id2}/data"),
    ] {
        assert_eq!(
            status(&s.base, &p, Some(&s.reader_any), None).await,
            200,
            "{p}"
        );
    }
    // a percent-encoded id is decoded before the comparison, as the router does
    let enc: String = id1.chars().map(|c| format!("%{:02X}", c as u32)).collect();
    assert_eq!(
        status(
            &s.base,
            &format!("/api/agents/{enc}"),
            Some(&s.reader),
            None
        )
        .await,
        200
    );
    assert!(warte::auth::is_known_right("agents:read"));
    assert!(
        !warte::auth::AGENT_RIGHTS.contains(&"agents:read"),
        "never a default"
    );
}

// rivet: verifies REQ-API-007
#[tokio::test]
async fn without_a_login_sensitive_reads_still_need_an_agent_token() {
    let s = serve(None).await;
    let id1 = register(&s, "a1", &s.reader).await;
    let id2 = register(&s, "a2", &s.reader2).await;
    let sensitive = [
        format!("/api/agents/{id1}/data"),
        "/api/access/list/a1".to_string(),
    ];
    // planted: no token, a token that is not a warte token, and a warte token
    // without agents:read
    for p in &sensitive {
        assert_eq!(status(&s.base, p, None, None).await, 401, "{p}");
        assert_eq!(
            status(&s.base, p, Some("not-a-token"), None).await,
            401,
            "{p}"
        );
        assert_eq!(status(&s.base, p, Some(&s.agent), None).await, 403, "{p}");
    }
    // agents:read opens the routes for the token's own agent
    for p in &sensitive {
        let code = status(&s.base, p, Some(&s.reader), None).await;
        assert!(code != 401 && code != 403, "{p}: {code}");
    }
    // but never another agent's data or access list
    for p in [
        format!("/api/agents/{id2}/data"),
        "/api/access/list/a2".to_string(),
        "/api/agents/no-such-id/data".to_string(),
    ] {
        assert_eq!(status(&s.base, &p, Some(&s.reader), None).await, 403, "{p}");
    }
    // agents:any widens it to every agent
    for p in [
        format!("/api/agents/{id2}/data"),
        "/api/access/list/a2".to_string(),
    ] {
        assert_eq!(
            status(&s.base, &p, Some(&s.reader_any), None).await,
            200,
            "{p}"
        );
    }
    // the rest of the read API stays as it was without [auth]
    for open in ["/api/health", "/api/agents", "/api/openapi.json"] {
        assert_eq!(status(&s.base, open, None, None).await, 200, "{open}");
    }
}

/// Sign in as `sub` and return the session cookie.
async fn sign_in(s: &Server, i: &Idp, sub: &str, code: &str) -> String {
    *i.sub.lock().unwrap() = sub.into();
    let (st, lc) = start_login(s, i, "/", code).await;
    let r = callback(s, code, &st, Some(&lc)).await;
    assert_eq!(r.status().as_u16(), 303);
    r.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

/// The CSRF token the My hours panel carries for this session.
async fn csrf(s: &Server, cookie: &str) -> String {
    let html = client()
        .get(format!("{}/api/dashboard/attention-partial", s.base))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    html.split(r#"data-csrf=""#)
        .nth(1)
        .and_then(|r| r.split('"').next())
        .unwrap_or_else(|| panic!("no csrf in {html}"))
        .to_string()
}

async fn put_attention(
    s: &Server,
    cookie: &str,
    token: Option<&str>,
    origin: Option<&str>,
    body: &Value,
) -> (u16, String) {
    let host = s.base.trim_start_matches("http://").to_string();
    let mut r = client()
        .put(format!("{}/api/attention", s.base))
        .header("Cookie", cookie)
        .json(body);
    if let Some(t) = token {
        r = r.header("X-Warte-CSRF", t);
    }
    if let Some(o) = origin {
        r = r.header("Origin", o.replace("SELF", &host));
    }
    let r = r.send().await.unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

async fn get_attention(s: &Server, cookie: Option<&str>, bearer: Option<&str>) -> (u16, Value) {
    let mut r = client().get(format!("{}/api/attention", s.base));
    if let Some(c) = cookie {
        r = r.header("Cookie", c);
    }
    if let Some(b) = bearer {
        r = r.bearer_auth(b);
    }
    let r = r.send().await.unwrap();
    let code = r.status().as_u16();
    (code, r.json().await.unwrap_or(Value::Null))
}

// rivet: verifies REQ-ATTN-001
#[tokio::test]
async fn each_person_reads_and_writes_only_their_own_attention_schedule() {
    let (i, iss) = idp().await;
    let s = serve(Some(cfg(&iss))).await;
    let u1 = sign_in(&s, &i, "u1", "a1").await;
    let u2 = sign_in(&s, &i, "u2", "a2").await;
    // nothing set yet: 404, so the notice client keeps its local file
    assert_eq!(get_attention(&s, Some(&u1), None).await.0, 404);
    assert_eq!(get_attention(&s, None, None).await.0, 401);

    let mine = json!({"schema_version": 1, "timezone": "UTC", "windows": ["09:00-11:00"],
                      "off": [{"from": "2026-12-24", "to": "2026-12-27", "note": "holiday"}]});
    let t1 = csrf(&s, &u1).await;
    let (code, body) = put_attention(&s, &u1, Some(&t1), Some("http://SELF"), &mine).await;
    assert_eq!(code, 200, "{body}");
    let theirs = json!({"windows": [{"from": "20:00", "to": "21:00", "days": ["sat"]}]});
    let t2 = csrf(&s, &u2).await;
    assert_eq!(
        put_attention(&s, &u2, Some(&t2), Some("http://SELF"), &theirs)
            .await
            .0,
        200
    );

    // each reads their own; the string span came back as {from, to}
    let (_, a) = get_attention(&s, Some(&u1), None).await;
    assert_eq!(
        a["windows"],
        json!([{"from": "09:00", "to": "11:00"}]),
        "{a}"
    );
    assert_eq!(a["schema_version"], 1);
    assert!(a["updated_at"].is_string());
    let (_, b) = get_attention(&s, Some(&u2), None).await;
    assert_eq!(b["windows"][0]["days"], json!(["sat"]), "{b}");

    // an agent token reads its owner's schedule, never anyone else's
    let (code, a1) = get_attention(&s, None, Some(&s.owned_by_u1)).await;
    assert_eq!((code, &a1["windows"]), (200, &a["windows"]));
    assert_eq!(
        get_attention(&s, None, Some(&s.reader)).await.0,
        404,
        "no --owner"
    );
    assert_eq!(
        get_attention(&s, None, Some(&s.agent)).await.0,
        403,
        "no agents:read"
    );

    // planted: a write without the CSRF token, with another session's token,
    // from a foreign origin or without one, and with an agent token
    let evil = json!({"windows": ["00:00-23:00"]});
    for (tok, origin) in [
        (None, Some("http://SELF")),
        (Some(t2.as_str()), Some("http://SELF")),
        (Some(t1.as_str()), Some("https://evil.example")),
        (Some(t1.as_str()), None),
    ] {
        assert_eq!(
            put_attention(&s, &u1, tok, origin, &evil).await.0,
            403,
            "{tok:?} {origin:?}"
        );
    }
    let r = client()
        .put(format!("{}/api/attention", s.base))
        .bearer_auth(&s.owned_by_u1)
        .json(&evil)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401, "a token never writes a schedule");
    assert_eq!(
        get_attention(&s, Some(&u1), None).await.1["windows"],
        a["windows"]
    );
}

// rivet: verifies REQ-ATTN-001
#[tokio::test]
async fn a_broken_schedule_is_refused_by_field_and_nothing_is_stored() {
    let (i, iss) = idp().await;
    let s = serve(Some(cfg(&iss))).await;
    let u1 = sign_in(&s, &i, "u1", "a1").await;
    let t = csrf(&s, &u1).await;
    for (bad, field) in [
        (json!({"windows": ["9:00-11:00"]}), "windows[0].from"),
        (json!({"windows": ["09:00-09:00"]}), "windows[0]"),
        (json!({"windows": []}), "windows"),
        (
            json!({"available": [{"from": "08:00", "to": "25:00"}]}),
            "available[0].to",
        ),
        (
            json!({"focus": [{"from": "08:00", "to": "09:00", "days": ["mo"]}]}),
            "focus[0].days",
        ),
        (json!({"max_asking_flows": 101}), "max_asking_flows"),
        (json!({"max_asking_flows": 0}), "max_asking_flows"),
        (json!({"timezone": "Nowhere/Secret_City"}), "timezone"),
        (json!({"timezone": "../../etc/passwd"}), "timezone"),
        (
            json!({"off": [{"from": "2026-12-27", "to": "2026-12-24"}]}),
            "off[0]",
        ),
        (json!({"schema_version": 2}), "schema_version"),
        (json!({"windows": ["09:00-10:00"], "secret": "x"}), "secret"),
    ] {
        let (code, msg) = put_attention(&s, &u1, Some(&t), Some("http://SELF"), &bad).await;
        assert_eq!(code, 422, "{bad}");
        assert!(msg.starts_with(field), "{bad}: {msg}");
        assert!(
            !msg.contains("Secret_City") && !msg.contains("passwd"),
            "a value was echoed: {msg}"
        );
    }
    assert_eq!(
        get_attention(&s, Some(&u1), None).await.0,
        404,
        "nothing was stored"
    );
    // a real zone passes
    let (code, body) = put_attention(
        &s,
        &u1,
        Some(&t),
        Some("http://SELF"),
        &json!({"timezone": "Europe/Berlin"}),
    )
    .await;
    assert_eq!(code, 200, "{body}");
}

// rivet: verifies REQ-ARCHVIEW-001
#[tokio::test]
async fn the_architecture_is_never_open_and_needs_a_login_or_architecture_read() {
    // without [auth]: the data needs architecture:read, the page shows nothing
    let open = serve(None).await;
    assert_eq!(
        status(&open.base, "/api/architecture", None, None).await,
        401
    );
    assert_eq!(
        status(&open.base, "/api/architecture", Some(&open.reader), None).await,
        403,
        "agents:read is not enough"
    );
    let body = client()
        .get(format!("{}/api/architecture", open.base))
        .bearer_auth(&open.arch_reader)
        .send()
        .await
        .unwrap();
    assert_eq!(body.status().as_u16(), 200);
    let v: Value = body.json().await.unwrap();
    assert_eq!(v["architecture"]["schema"], "architecture.v1");
    assert_eq!(v["findings"].as_array().unwrap().len(), 1, "{v}");
    let page = client()
        .get(format!("{}/api/dashboard/architecture-partial", open.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("needs a login"), "{page}");
    assert!(
        !page.contains("Ground station") && !page.contains("app-key"),
        "{page}"
    );

    // with [auth]: a login sees it; no login is 401; an agent token needs the right
    let (i, iss) = idp().await;
    let s = serve(Some(cfg(&iss))).await;
    let u1 = sign_in(&s, &i, "u1", "arch1").await;
    assert_eq!(status(&s.base, "/api/architecture", None, None).await, 401);
    assert_eq!(
        status(&s.base, "/api/architecture", None, Some(&u1)).await,
        200
    );
    assert_eq!(
        status(&s.base, "/api/architecture", Some(&s.reader), None).await,
        403
    );
    assert_eq!(
        status(&s.base, "/api/architecture", Some(&s.arch_reader), None).await,
        200
    );
    let page = client()
        .get(format!("{}/api/dashboard/architecture-partial", s.base))
        .header("Cookie", &u1)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        page.contains("Ground station") && page.contains("crosses a zone"),
        "{page}"
    );
    assert!(
        page.contains("warnrow"),
        "the insecure crossing is highlighted"
    );
    assert!(warte::auth::is_known_right("architecture:read"));
    assert!(
        !warte::auth::AGENT_RIGHTS.contains(&"architecture:read"),
        "never a default"
    );
}

// rivet: verifies REQ-GLANCE-001
#[tokio::test]
async fn the_glance_reads_notices_for_a_login_and_heartbeats_carry_a_status() {
    // without [auth]: no glance at all
    let open = serve(None).await;
    assert_eq!(status(&open.base, "/api/glance", None, None).await, 401);
    assert_eq!(status(&open.base, "/glance", None, None).await, 401);

    let (i, iss) = idp().await;
    let s = serve(Some(cfg(&iss))).await;
    let id = register(&s, "a1", &s.reader).await;
    // heartbeat status: none, a known one, then refusals
    let beat = |body: Option<&str>| {
        let mut r = client()
            .post(format!("{}/api/agents/{id}/heartbeat", s.base))
            .bearer_auth(&s.reader);
        if let Some(b) = body {
            r = r
                .header("Content-Type", "application/json")
                .body(b.to_string());
        }
        r.send()
    };
    assert_eq!(beat(None).await.unwrap().status().as_u16(), 204);
    assert_eq!(
        beat(Some(r#"{"status":"ended"}"#))
            .await
            .unwrap()
            .status()
            .as_u16(),
        204
    );
    // older clients' bodies still beat; only an unknown status is refused
    for old in [
        "{}",
        r#"{"name":"a1"}"#,
        r#"{"status":"idle","x":1}"#,
        "not json",
    ] {
        assert_eq!(
            beat(Some(old)).await.unwrap().status().as_u16(),
            204,
            "{old}"
        );
    }
    for bad in [r#"{"status":"asleep"}"#, r#"{"status":7}"#] {
        assert_eq!(
            beat(Some(bad)).await.unwrap().status().as_u16(),
            422,
            "{bad}"
        );
    }

    let u1 = sign_in(&s, &i, "u1", "g1").await;
    assert_eq!(status(&s.base, "/api/glance", None, None).await, 401);
    let g = glance_json(&s.base, &u1).await;
    assert_eq!(g["verdict"], "quiet", "{g}");
    // an urgent decision posted by an agent turns it
    let notice = json!({"schema_version": 2, "id": "n1", "kind": "needs-decision", "revision": 1,
        "created_at": "2026-11-22T01:00:00Z", "agent": "a1", "title": "Main is red",
        "repo": "pulseengine/rivet", "tags": ["red-main"], "status": "open"});
    let r = client()
        .post(format!("{}/api/agents/{id}/data", s.base))
        .bearer_auth(&s.reader)
        .json(&json!({"data_type": "notice.v1", "content": notice.to_string()}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    let g = glance_json(&s.base, &u1).await;
    assert_eq!(g["verdict"], "needs-you", "{g}");
    assert_eq!(g["items"][0]["text"], "red-main: Main is red");
    let page = client()
        .get(format!("{}/glance", s.base))
        .header("Cookie", &u1)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        page.contains("Needs you now") && page.contains("Main is red"),
        "{page}"
    );
}

async fn glance_json(base: &str, cookie: &str) -> Value {
    client()
        .get(format!("{base}/api/glance"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn post_answer(
    s: &Server,
    cookie: &str,
    csrf: Option<&str>,
    origin: Option<&str>,
    body: &Value,
) -> (u16, String) {
    let host = s.base.trim_start_matches("http://").to_string();
    let mut r = client()
        .post(format!("{}/api/decisions/answer", s.base))
        .header("Cookie", cookie)
        .json(body);
    if let Some(t) = csrf {
        r = r.header("X-Warte-CSRF", t);
    }
    if let Some(o) = origin {
        r = r.header("Origin", o.replace("SELF", &host));
    }
    let r = r.send().await.unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

async fn decisions_page(s: &Server, cookie: &str) -> String {
    client()
        .get(format!("{}/api/dashboard/decisions-partial", s.base))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

// rivet: verifies REQ-DECIDE-001
#[tokio::test]
async fn only_the_owner_answers_a_decision_from_this_page_and_it_reaches_the_agent_once() {
    let open_s = serve(None).await;
    let (code, _) = post_answer(&open_s, "x=y", None, None, &json!({})).await;
    assert_eq!(code, 401, "no answers without [auth]");

    let (i, iss) = idp().await;
    let s = serve(Some(cfg(&iss))).await;
    let id = register(&s, "a1", &s.reader).await;
    let notice = json!({"schema_version": 2, "id": "n1", "kind": "needs-decision", "revision": 2,
        "created_at": "2026-11-22T01:00:00Z", "agent": "a1", "title": "Pick the transport",
        "repo": "pulseengine/relay", "status": "open", "recommended": "quic",
        "options": [{"id": "quic", "label": "QUIC", "description": "", "pros": [], "cons": []},
                    {"id": "udp", "label": "UDP", "description": "", "pros": [], "cons": []}]});
    let r = client()
        .post(format!("{}/api/agents/{id}/data", s.base))
        .bearer_auth(&s.reader)
        .json(&json!({"data_type": "notice.v1", "content": notice.to_string()}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    assert_eq!(
        post_answer(&s, "x=y", None, Some("http://SELF"), &json!({}))
            .await
            .0,
        401
    );

    let owner = sign_in(&s, &i, "u1", "d1").await;
    let page = decisions_page(&s, &owner).await;
    assert!(
        page.contains("Pick the transport") && page.contains("QUIC (recommended)"),
        "{page}"
    );
    let csrf = page
        .split(r#"data-csrf=""#)
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    let good = json!({"notice_id": "n1", "revision": 2, "option_id": "quic", "note": "go"});
    // planted: no CSRF token, a foreign origin, a member rather than the owner
    assert_eq!(
        post_answer(&s, &owner, None, Some("http://SELF"), &good)
            .await
            .0,
        403
    );
    assert_eq!(
        post_answer(&s, &owner, Some(&csrf), Some("https://evil.example"), &good)
            .await
            .0,
        403
    );
    *i.role.lock().unwrap() = "member".into();
    let member = sign_in(&s, &i, "u2", "d2").await;
    *i.role.lock().unwrap() = String::new();
    let mcsrf = decisions_page(&s, &member).await;
    let mcsrf = mcsrf
        .split(r#"data-csrf=""#)
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    assert_eq!(
        post_answer(&s, &member, Some(&mcsrf), Some("http://SELF"), &good)
            .await
            .0,
        403
    );
    // planted: a stale revision and an option not offered
    let stale = json!({"notice_id": "n1", "revision": 1, "option_id": "quic"});
    assert_eq!(
        post_answer(&s, &owner, Some(&csrf), Some("http://SELF"), &stale)
            .await
            .0,
        409
    );
    let other = json!({"notice_id": "n1", "revision": 2, "option_id": "carrier-pigeon"});
    assert_eq!(
        post_answer(&s, &owner, Some(&csrf), Some("http://SELF"), &other)
            .await
            .0,
        409
    );
    // the owner's answer is stored once, on the agent that asked
    let (code, body) = post_answer(&s, &owner, Some(&csrf), Some("http://SELF"), &good).await;
    assert_eq!(code, 201, "{body}");
    assert_eq!(
        post_answer(&s, &owner, Some(&csrf), Some("http://SELF"), &good)
            .await
            .0,
        409,
        "answered once"
    );
    let stored: Value = client()
        .get(format!(
            "{}/api/agents/{id}/data?data_type=answer.v1",
            s.base
        ))
        .bearer_auth(&s.reader)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = stored.to_string();
    assert!(
        text.contains(r#"\"via\":\"warte\""#) && text.contains(r#"\"option_id\":\"quic\""#),
        "{text}"
    );
    assert!(
        !decisions_page(&s, &owner)
            .await
            .contains("Pick the transport"),
        "answered decisions disappear"
    );
}

// rivet: verifies REQ-REPORTS-001
#[tokio::test]
async fn a_producer_posts_its_report_with_its_right_and_only_a_login_sees_it() {
    let (i, iss) = idp().await;
    let s = serve(Some(cfg(&iss))).await;
    let report = serde_json::json!({"schema": "seam-check.v1", "now": "2026-11-22T06:41:00+00:00",
        "seams": [{"id": "a/b", "source": {"repo": "pulseengine/warte", "issue": 87, "url": "u"},
                   "status": "decided", "violations": [], "warnings": []}],
        "issues_without_block": [], "errors": [], "exit": 0});
    let post = |bearer: Option<String>, body: String| {
        let mut r = client()
            .post(format!("{}/api/reports/seam-check", s.base))
            .header("Content-Type", "application/json")
            .body(body);
        if let Some(b) = bearer {
            r = r.bearer_auth(b);
        }
        r.send()
    };
    // planted: no token, a token without the right, a report of the wrong schema
    assert_eq!(
        post(None, report.to_string())
            .await
            .unwrap()
            .status()
            .as_u16(),
        401
    );
    assert_eq!(
        post(Some(s.reader.clone()), report.to_string())
            .await
            .unwrap()
            .status()
            .as_u16(),
        403
    );
    let mut wrong = report.clone();
    wrong["schema"] = "seam-check.v9".into();
    assert_eq!(
        post(Some(s.reporter.clone()), wrong.to_string())
            .await
            .unwrap()
            .status()
            .as_u16(),
        422
    );
    assert_eq!(
        post(Some(s.reporter.clone()), report.to_string())
            .await
            .unwrap()
            .status()
            .as_u16(),
        204
    );
    // only a login sees it; the producer's token reads nothing (a refused
    // partial answers with the sign-in panel, never the data)
    let refused = client()
        .get(format!("{}/api/dashboard/seams-partial", s.base))
        .bearer_auth(&s.reporter)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        refused.contains("Sign in") && !refused.contains("a/b"),
        "{refused}"
    );
    let u1 = sign_in(&s, &i, "u1", "r1").await;
    let page = client()
        .get(format!("{}/api/dashboard/seams-partial", s.base))
        .header("Cookie", &u1)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("a/b") && page.contains("exit 0"), "{page}");
    // without [auth] the view shows nothing
    let open = serve(None).await;
    let page = client()
        .get(format!("{}/api/dashboard/seams-partial", open.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("needs a login"), "{page}");
}
