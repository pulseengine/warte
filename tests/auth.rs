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
    let mut c = claims(&b, Some("owner"));
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
}

async fn serve(auth: Option<AuthConfig>) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let config = warte::config::Config {
        db_path: dir.path().join("state.db"),
        ca_cert_path: dir.path().join("ca.pem"),
        ca_key_path: dir.path().join("ca-key.pem"),
        auth,
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
    let app = warte::server::build_router(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    Server {
        _dir: dir,
        base,
        agent,
        briefing_job,
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
