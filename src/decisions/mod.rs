//! Decisions in the app (REQ-DECIDE-001, warte#184): the agents' open
//! `needs-decision` notices, answered by the owner in warte. An answer is an
//! `answer.v1` row on the agent that asked (notice.schema.json answer,
//! schema_version 1, `via: "warte"`), which the notice client polls. Seam
//! notice-warte/decision-answers.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const NOTE_MAX: usize = 2000;
pub const OPTION_ID_MAX: usize = 40;
pub const NOTICE_ID_MAX: usize = 80;

/// One open decision and the agents_data owner that asked it.
#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    /// The agents row id the notice was posted under: the answer goes there.
    pub agent_row: String,
    pub notice: Value,
    /// Tagged with an always-open kind: listed first.
    pub urgent: bool,
}

/// The newest revision of each notice, with the agents row it came from.
fn newest(rows: &[(String, Value)]) -> BTreeMap<String, (String, Value)> {
    let mut out: BTreeMap<String, (String, Value)> = BTreeMap::new();
    for (agent, n) in rows {
        let Some(id) = n["id"].as_str() else { continue };
        let rev = n["revision"].as_u64().unwrap_or(0);
        if out
            .get(id)
            .is_none_or(|(_, o)| rev > o["revision"].as_u64().unwrap_or(0))
        {
            out.insert(id.to_string(), (agent.clone(), n.clone()));
        }
    }
    out
}

fn is_open(n: &Value) -> bool {
    n["kind"] == "needs-decision"
        && n["status"]
            .as_str()
            .is_none_or(|s| s == "open" || s == "queued")
}

fn answered(answers: &[Value], id: &str, rev: u64) -> bool {
    answers
        .iter()
        .any(|a| a["notice_id"] == id && a["revision"].as_u64() == Some(rev))
}

/// The open decisions: newest revision, still open, not yet answered at
/// that revision. Always-open first, then by flow, then oldest first.
pub fn open(rows: &[(String, Value)], answers: &[Value], always_open: &[String]) -> Vec<Decision> {
    let mut out: Vec<Decision> = newest(rows)
        .into_iter()
        .filter(|(_, (_, n))| is_open(n))
        .filter(|(id, (_, n))| !answered(answers, id, n["revision"].as_u64().unwrap_or(0)))
        .map(|(_, (agent_row, notice))| {
            let urgent = notice["tags"].as_array().is_some_and(|t| {
                t.iter().any(|x| {
                    x.as_str()
                        .is_some_and(|s| always_open.iter().any(|k| k == s))
                })
            });
            Decision {
                agent_row,
                notice,
                urgent,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        (
            !a.urgent,
            a.notice["flow"].as_str().unwrap_or(""),
            a.notice["created_at"].as_str().unwrap_or(""),
        )
            .cmp(&(
                !b.urgent,
                b.notice["flow"].as_str().unwrap_or(""),
                b.notice["created_at"].as_str().unwrap_or(""),
            ))
    });
    out
}

/// The answer for `notice_id` at `revision` with `option_id`, or why not:
/// the notice must exist, be the newest revision, still be open and not
/// answered, and have offered the option. Returns the agents row to store it
/// on and the `answer.v1` content.
pub fn answer(
    rows: &[(String, Value)],
    answers: &[Value],
    notice_id: &str,
    revision: u64,
    option_id: &str,
    note: Option<&str>,
    now_rfc3339: &str,
) -> Result<(String, Value), String> {
    if notice_id.is_empty() || notice_id.len() > NOTICE_ID_MAX {
        return Err("notice_id: not a notice id".into());
    }
    if option_id.is_empty() || option_id.len() > OPTION_ID_MAX {
        return Err("option_id: not an option id".into());
    }
    if note.is_some_and(|n| n.chars().count() > NOTE_MAX) {
        return Err(format!("note: at most {NOTE_MAX} characters"));
    }
    let latest = newest(rows);
    let Some((agent_row, n)) = latest.get(notice_id) else {
        return Err("notice_id: no such decision".into());
    };
    if !is_open(n) {
        return Err("notice_id: the decision is no longer open".into());
    }
    if n["revision"].as_u64() != Some(revision) {
        return Err("revision: the decision changed; reload it".into());
    }
    if answered(answers, notice_id, revision) {
        return Err("notice_id: already answered".into());
    }
    let offered = n["options"]
        .as_array()
        .is_some_and(|o| o.iter().any(|x| x["id"] == option_id));
    if !offered {
        return Err("option_id: the decision did not offer it".into());
    }
    let mut a = json!({
        "schema_version": 1,
        "notice_id": notice_id,
        "revision": revision,
        "option_id": option_id,
        "answered_at": now_rfc3339,
        "via": "warte",
    });
    if let Some(note) = note.filter(|n| !n.is_empty()) {
        a["note"] = Value::from(note);
    }
    Ok((agent_row.clone(), a))
}

/// The Decisions view: one card per decision, its options as buttons.
pub fn view(ds: &[Decision], csrf: &str) -> String {
    let esc = crate::dashboard::fleet::esc;
    if ds.is_empty() {
        return r#"<div class="overview"><h2>Decisions</h2><p class="empty">No decision waits for you.</p></div>"#.into();
    }
    let cards: String = ds
        .iter()
        .map(|d| {
            let n = &d.notice;
            let rec = n["recommended"].as_str().unwrap_or("");
            let opts: String = n["options"]
                .as_array()
                .map(|o| {
                    o.iter()
                        .map(|x| {
                            let id = x["id"].as_str().unwrap_or("");
                            format!(
                                r#"<li><button type="button" class="btn-small" data-option="{}">{}{}</button> {}</li>"#,
                                esc(id),
                                esc(x["label"].as_str().unwrap_or(id)),
                                if id == rec { " (recommended)" } else { "" },
                                esc(x["description"].as_str().unwrap_or("")),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            format!(
                r#"<div class="decision{}" data-notice="{}" data-revision="{}"><h3>{}</h3><p class="dim">{} · {} · {}revision {}</p><p>{}</p><ul>{opts}</ul><label>Note for the agent (optional) <input type="text" name="note" maxlength="2000"></label><p class="decision-status" role="status"></p></div>"#,
                if d.urgent { " urgent" } else { "" },
                esc(n["id"].as_str().unwrap_or("")),
                n["revision"].as_u64().unwrap_or(0),
                esc(n["title"].as_str().unwrap_or("")),
                esc(n["agent"].as_str().unwrap_or("")),
                esc(n["repo"].as_str().unwrap_or("")),
                n["flow"].as_str().map(|f| format!("{} · ", esc(f))).unwrap_or_default(),
                n["revision"].as_u64().unwrap_or(0),
                esc(n["question"].as_str().unwrap_or("")),
            )
        })
        .collect();
    format!(
        r#"<div class="overview decisions" id="decisions" data-csrf="{csrf}"><h2>Decisions</h2><p>An answer goes to the agent that asked; a note is data for it, never instructions.</p>{cards}<script>
(function () {{
  var root = document.getElementById("decisions");
  if (!root) return;
  root.querySelectorAll("button[data-option]").forEach(function (b) {{
    b.addEventListener("click", function () {{
      var card = b.closest(".decision");
      var s = card.querySelector(".decision-status");
      var note = card.querySelector("input[name=note]").value;
      var body = {{notice_id: card.dataset.notice, revision: Number(card.dataset.revision), option_id: b.dataset.option}};
      if (note) body.note = note;
      fetch("api/decisions/answer", {{method: "POST", headers: {{"Content-Type": "application/json", "X-Warte-CSRF": root.dataset.csrf}}, body: JSON.stringify(body)}})
        .then(function (r) {{ return r.text().then(function (t) {{ s.textContent = r.ok ? "Answered." : t; if (r.ok) card.querySelectorAll("button").forEach(function (x) {{ x.disabled = true; }}); }}); }})
        .catch(function () {{ s.textContent = "Could not reach warte."; }});
    }});
  }});
}})();
</script></div>"#,
        csrf = esc(csrf),
    )
}
