//! `warte admit` (REQ-QUEUE-005, warte#70): the verdict as an exit code.

use axum::{extract::Query, routing::get, Json, Router};
use std::collections::HashMap;
use warte::admission::admit::{exit_code, EXIT_BATCH, EXIT_ERROR, EXIT_HOLD, EXIT_OPEN};

// rivet: verifies REQ-QUEUE-005
#[test]
fn each_verdict_has_its_exit_code() {
    assert_eq!(
        exit_code(r#"{"verdict":"open","reason":"room"}"#),
        EXIT_OPEN
    );
    assert_eq!(
        exit_code(r#"{"verdict":"batch","recheck_s":600}"#),
        EXIT_BATCH
    );
    assert_eq!(
        exit_code(r#"{"verdict":"hold","recheck_s":300}"#),
        EXIT_HOLD
    );
    // planted: an unknown verdict, no verdict, not JSON
    for bad in [r#"{"verdict":"maybe"}"#, "{}", "not json"] {
        assert_eq!(exit_code(bad), EXIT_ERROR, "{bad}");
    }
    assert_eq!(
        (EXIT_OPEN, EXIT_BATCH, EXIT_HOLD, EXIT_ERROR),
        (0, 10, 11, 1)
    );
}

// rivet: verifies REQ-QUEUE-005
#[tokio::test]
async fn the_command_exits_with_the_verdict_so_a_chained_pr_cannot_skip_it() {
    // a stand-in admission endpoint: the verdict is the repo's name
    let app = Router::new().route(
        "/api/admission",
        get(|Query(q): Query<HashMap<String, String>>| async move {
            Json(serde_json::json!({"verdict": q.get("repo").cloned().unwrap_or_default(), "kind": q.get("kind")}))
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let bin = env!("CARGO_BIN_EXE_warte");
    let run = move |repo: &str, url: &str| {
        let out = std::process::Command::new(bin)
            .args(["admit", "--repo", repo, "--kind", "fix", "--url", url])
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).to_string(),
        )
    };
    let base2 = base.clone();
    let (open, batch, hold, unknown, down) = tokio::task::spawn_blocking(move || {
        (
            run("open", &base2),
            run("batch", &base2),
            run("hold", &base2),
            run("weird", &base2),
            run("open", "http://127.0.0.1:9"),
        )
    })
    .await
    .unwrap();
    assert_eq!(open.0, Some(0));
    assert!(open.1.contains(r#""verdict":"open""#), "{}", open.1);
    assert_eq!(batch.0, Some(10));
    assert_eq!(hold.0, Some(11));
    assert_eq!(unknown.0, Some(1));
    assert_eq!(down.0, Some(1), "an unreachable warte is never a go");
}
