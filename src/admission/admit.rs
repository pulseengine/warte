//! `warte admit` (REQ-QUEUE-005, warte#70): the admission check as a command
//! with exit codes, so `warte admit --repo R --kind K && gh pr create ...`
//! cannot open a pull request against a `batch` or `hold` verdict.

/// `open`: go ahead.
pub const EXIT_OPEN: i32 = 0;
/// `batch`: fold the change into a pull request already open, or wait.
pub const EXIT_BATCH: i32 = 10;
/// `hold`: wait `recheck_s` and ask again.
pub const EXIT_HOLD: i32 = 11;
/// Anything else: warte unreachable, a refused request, an unknown verdict.
pub const EXIT_ERROR: i32 = 1;

/// The exit code for an admission answer's body.
pub fn exit_code(body: &str) -> i32 {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return EXIT_ERROR,
    };
    match v["verdict"].as_str() {
        Some("open") => EXIT_OPEN,
        Some("batch") => EXIT_BATCH,
        Some("hold") => EXIT_HOLD,
        _ => EXIT_ERROR,
    }
}

/// Ask `GET <base>/api/admission`, print the JSON answer and return the exit
/// code. Errors print to stderr and return [`EXIT_ERROR`].
pub async fn run(base: &str, repo: &str, kind: &str, ca_cert: Option<&std::path::Path>) -> i32 {
    let mut http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));
    if let Some(p) = ca_cert {
        let certs = std::fs::read(p)
            .ok()
            .and_then(|pem| reqwest::Certificate::from_pem_bundle(&pem).ok())
            .unwrap_or_default();
        if certs.is_empty() {
            eprintln!("warte admit: --ca-cert {}: no PEM certificate", p.display());
            return EXIT_ERROR;
        }
        for c in certs {
            http = http.add_root_certificate(c);
        }
    }
    let Ok(http) = http.build() else {
        return EXIT_ERROR;
    };
    let r = http
        .get(format!("{}/api/admission", base.trim_end_matches('/')))
        .query(&[("repo", repo), ("kind", kind)])
        .send()
        .await;
    let r = match r {
        Ok(r) => r,
        Err(e) => {
            eprintln!("warte admit: {e}");
            return EXIT_ERROR;
        }
    };
    let status = r.status();
    let body = r.text().await.unwrap_or_default();
    if !status.is_success() {
        eprintln!("warte admit: {status}: {}", body.trim());
        return EXIT_ERROR;
    }
    println!("{}", body.trim());
    exit_code(&body)
}
