//! `warte ci-agent arch`: build warte's architecture.v1 (`warte docs architecture-view`) from the
//! spar instance of a security model, plus the agent sessions the inventory starts on the
//! machines named in `[ci_agent] arch_hosts`.
//!
//! Sessions per machine are the union of the Zellij layout's tabs
//! (roles/agent_sessions/files/hosts/<host>/agents.kdl) and the names that get a warte token
//! (group_vars warte_agent_tokens.<host>); sandboxed and GitHub-broker sessions come from
//! host_vars agent_sessions_sandboxed / agent_sessions_github. Every value is a name or a kind;
//! nothing here reads a secret.
//!
//! Objects keep the order their keys are inserted in (an `IndexMap`), so the printed document
//! lists `id`, `name`, ... in the same order as before the import, without switching serde_json
//! to `preserve_order` for all of warte.

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Map as JMap, Value};
use std::collections::BTreeSet;
use std::path::Path;

const P: &str = "PE_Arch::";

/// A JSON document whose objects keep insertion order.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Doc {
    Object(Map),
    Array(Vec<Doc>),
    Json(Value),
}

type Map = IndexMap<String, Doc>;

fn j(v: Value) -> Doc {
    Doc::Json(v)
}

pub struct Opts<'a> {
    pub instance: &'a Path,
    pub repo: &'a Path,
    pub commit: &'a str,
    pub spar_version: &'a str,
    pub generated_at: &'a str,
    /// Machines whose agent sessions are read (`[ci_agent] arch_hosts`).
    pub hosts: &'a [String],
    /// `source.model` (`[ci_agent] arch_model`).
    pub model: &'a str,
}

/// Lowercase, `_` to `-`, anything outside [a-z0-9-] to `-`, at most 64 characters.
pub fn cid(name: &str) -> String {
    name.to_lowercase()
        .replace('_', "-")
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect()
}

fn props(node: &Value) -> JMap<String, Value> {
    let mut out = JMap::new();
    if let Some(tp) = node.get("typed_properties").and_then(Value::as_object) {
        for (k, v) in tp {
            let Some(key) = k.strip_prefix(P) else {
                continue;
            };
            let val = if v.get("kind").and_then(Value::as_str) == Some("list") {
                Value::Array(
                    v.get("items")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|i| i.get("value").cloned().unwrap_or(Value::Null))
                        .collect(),
                )
            } else {
                v.get("value").cloned().unwrap_or(Value::Null)
            };
            out.insert(key.to_string(), val);
        }
    }
    out
}

/// A property that Python would treat as truthy: present, not null, not false, not "", not [].
fn truthy<'a>(p: &'a JMap<String, Value>, k: &str) -> Option<&'a Value> {
    match p.get(k)? {
        Value::Null | Value::Bool(false) => None,
        Value::String(s) if s.is_empty() => None,
        Value::Array(a) if a.is_empty() => None,
        v => Some(v),
    }
}

fn need<'a>(p: &'a JMap<String, Value>, k: &str, what: &str) -> Result<&'a Value, String> {
    p.get(k)
        .ok_or_else(|| format!("{what}: missing PE_Arch::{k}"))
}

fn credentials(items: &Value) -> Doc {
    let mut creds = Vec::new();
    for s in items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let mut parts = s.splitn(3, '|');
        let name = parts.next().unwrap_or("");
        let kind = parts.next().unwrap_or("");
        let purpose = parts.next().unwrap_or("");
        let mut c = Map::new();
        c.insert("name".into(), j(json!(name)));
        c.insert("kind".into(), j(json!(kind)));
        if !purpose.is_empty() {
            c.insert("purpose".into(), j(json!(purpose)));
        }
        creds.push(Doc::Object(c));
    }
    Doc::Array(creds)
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

type Model = (Vec<Doc>, Vec<Doc>, Vec<Doc>);

fn model(inst: &Value) -> Result<Model, String> {
    let (mut zones, mut comps, mut conns) = (Vec::new(), Vec::new(), Vec::new());
    for ch in inst
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let p = props(ch);
        let name = s(ch, "name");
        let i = cid(name);
        let label = p.get("Label").cloned().unwrap_or_else(|| json!(i));
        if p.get("Kind").and_then(Value::as_str) == Some("zone") {
            let mut z = Map::new();
            z.insert("id".into(), j(json!(i)));
            z.insert("name".into(), j(label));
            z.insert("trust".into(), j(need(&p, "Trust", name)?.clone()));
            if let Some(d) = truthy(&p, "Description") {
                z.insert("description".into(), j(d.clone()));
            }
            zones.push(Doc::Object(z));
            continue;
        }
        let mut c = Map::new();
        c.insert("id".into(), j(json!(i)));
        c.insert("name".into(), j(label));
        c.insert("kind".into(), j(need(&p, "Kind", name)?.clone()));
        c.insert("zone".into(), j(need(&p, "Zone", name)?.clone()));
        if let Some(h) = truthy(&p, "Host") {
            c.insert("host".into(), j(json!(cid(h.as_str().unwrap_or("")))));
        }
        if let Some(v) = truthy(&p, "Controller") {
            c.insert("controller".into(), j(v.clone()));
        }
        if let Some(v) = truthy(&p, "Credentials") {
            c.insert("credentials".into(), credentials(v));
        }
        c.insert(
            "aadl".into(),
            j(json!(format!(
                "{}::{} ({})",
                s(ch, "package"),
                s(ch, "type_name"),
                name
            ))),
        );
        comps.push(Doc::Object(c));
    }
    for e in inst
        .get("connections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let p = props(e);
        let name = s(e, "name");
        let end = |k: &str| cid(s(e, k).split('.').next().unwrap_or(""));
        let mut c = Map::new();
        c.insert("id".into(), j(json!(cid(name))));
        c.insert("from".into(), j(json!(end("source"))));
        c.insert("to".into(), j(json!(end("destination"))));
        c.insert("protocol".into(), j(need(&p, "Protocol", name)?.clone()));
        c.insert(
            "authenticated".into(),
            j(need(&p, "Authenticated", name)?.clone()),
        );
        c.insert("encrypted".into(), j(need(&p, "Encrypted", name)?.clone()));
        if let Some(v) = p.get("Port_Number") {
            c.insert("port".into(), j(v.clone()));
        }
        if let Some(v) = truthy(&p, "Credential") {
            c.insert("credential".into(), j(v.clone()));
        }
        if let Some(v) = truthy(&p, "Controller") {
            c.insert("controller".into(), j(v.clone()));
        }
        conns.push(Doc::Object(c));
    }
    Ok((zones, comps, conns))
}

fn yaml(path: &Path) -> Result<serde_yaml::Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_yaml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn str_list(v: Option<&serde_yaml::Value>) -> Vec<String> {
    v.and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(String::from))
        .collect()
}

/// The `name` of every `tab name="..."` in a Zellij layout.
fn tabs(kdl: &str) -> Vec<String> {
    let pat = "tab name=\"";
    let mut out = Vec::new();
    let mut rest = kdl;
    while let Some(i) = rest.find(pat) {
        rest = &rest[i + pat.len()..];
        if let Some(j) = rest.find('"') {
            if j > 0 {
                out.push(rest[..j].to_string());
            }
            rest = &rest[j..];
        }
    }
    out
}

fn https(id: String, from: &str, to: &str, credential: String) -> Map {
    let mut c = Map::new();
    c.insert("id".into(), j(json!(id)));
    c.insert("from".into(), j(json!(from)));
    c.insert("to".into(), j(json!(to)));
    c.insert("protocol".into(), j(json!("https")));
    c.insert("port".into(), j(json!(443)));
    c.insert("authenticated".into(), j(json!(true)));
    c.insert("encrypted".into(), j(json!(true)));
    c.insert("credential".into(), j(json!(credential)));
    c
}

fn cred(name: &str, kind: &str, purpose: &str) -> Doc {
    let mut c = Map::new();
    c.insert("name".into(), j(json!(name)));
    c.insert("kind".into(), j(json!(kind)));
    c.insert("purpose".into(), j(json!(purpose)));
    Doc::Object(c)
}

fn sessions(repo: &Path, hosts: &[String]) -> Result<(Vec<Doc>, Vec<Doc>), String> {
    if hosts.is_empty() {
        // no `arch_hosts`: the model alone, and no inventory is read
        return Ok((Vec::new(), Vec::new()));
    }
    let a = repo.join("ansible");
    let all = yaml(&a.join("group_vars/all.yml"))?;
    let tokens = all.get("warte_agent_tokens");
    let (mut comps, mut conns) = (Vec::new(), Vec::new());
    for mac in hosts.iter().map(String::as_str) {
        let hv = yaml(&a.join(format!("host_vars/{mac}/main.yml")))?;
        let sandboxed: BTreeSet<String> = str_list(hv.get("agent_sessions_sandboxed"))
            .into_iter()
            .collect();
        let github: BTreeSet<String> = str_list(hv.get("agent_sessions_github"))
            .into_iter()
            .collect();
        let kdl = a.join(format!("roles/agent_sessions/files/hosts/{mac}/agents.kdl"));
        let tab_names = if kdl.exists() {
            tabs(&std::fs::read_to_string(&kdl).map_err(|e| format!("{}: {e}", kdl.display()))?)
        } else {
            Vec::new()
        };
        let tok: BTreeSet<String> = str_list(tokens.and_then(|t| t.get(mac)))
            .into_iter()
            .collect();
        let names: BTreeSet<String> = tab_names.into_iter().chain(tok.iter().cloned()).collect();
        for name in names {
            let sid = cid(&format!("s-{mac}-{name}"));
            let mut creds = Vec::new();
            let mut ctrl = vec![j(json!("CTRL-008"))];
            if tok.contains(&name) {
                creds.push(cred(
                    "warte-token",
                    "token",
                    "factory notices and heartbeats; GitHub tokens when brokered",
                ));
            }
            if sandboxed.contains(&name) {
                ctrl.push(j(json!("CTRL-015")));
            }
            if github.contains(&name) {
                ctrl.push(j(json!("CTRL-016")));
                creds.push(cred(
                    "github-token",
                    "token",
                    "one hour, from warte's broker, limited to the agent's repositories",
                ));
            }
            let mut c = Map::new();
            c.insert("id".into(), j(json!(sid)));
            c.insert("name".into(), j(json!(format!("{name} ({mac})"))));
            c.insert("kind".into(), j(json!("session")));
            c.insert("zone".into(), j(json!("z-agents")));
            c.insert("host".into(), j(json!(mac)));
            c.insert("controller".into(), Doc::Array(ctrl));
            c.insert("agent".into(), j(json!(name)));
            if !creds.is_empty() {
                c.insert("credentials".into(), Doc::Array(creds));
            }
            comps.push(Doc::Object(c));
            if tok.contains(&name) {
                conns.push(Doc::Object(https(
                    cid(&format!("{sid}-warte")),
                    &sid,
                    "caddy",
                    format!("{sid}/warte-token"),
                )));
            }
            if github.contains(&name) {
                let mut c = https(
                    cid(&format!("{sid}-github")),
                    &sid,
                    "github",
                    format!("{sid}/github-token"),
                );
                c.insert("controller".into(), j(json!(["CTRL-016"])));
                conns.push(Doc::Object(c));
            } else if !sandboxed.contains(&name) {
                // unsandboxed: gh and git with the owner's login on that machine
                conns.push(Doc::Object(https(
                    cid(&format!("{sid}-github")),
                    &sid,
                    "github",
                    format!("{mac}/gh-login"),
                )));
            }
        }
    }
    Ok((comps, conns))
}

pub fn build(o: &Opts) -> Result<Doc, String> {
    let text = std::fs::read_to_string(o.instance)
        .map_err(|e| format!("{}: {e}", o.instance.display()))?;
    let inst: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", o.instance.display()))?;
    let (zones, mut comps, mut conns) = model(&inst)?;
    let (sc, sx) = sessions(o.repo, o.hosts)?;
    comps.extend(sc);
    conns.extend(sx);
    let generated_at = if o.generated_at.is_empty() {
        utc(crate::ciagent::collect::now())
    } else {
        o.generated_at.to_string()
    };
    let mut source = Map::new();
    source.insert("tool".into(), j(json!("spar")));
    source.insert("version".into(), j(json!(o.spar_version)));
    source.insert("model".into(), j(json!(o.model)));
    source.insert("commit".into(), j(json!(o.commit)));
    let mut doc = Map::new();
    doc.insert("schema".into(), j(json!("architecture.v1")));
    doc.insert("generated_at".into(), j(json!(generated_at)));
    doc.insert("source".into(), Doc::Object(source));
    doc.insert("zones".into(), Doc::Array(zones));
    doc.insert("components".into(), Doc::Array(comps));
    doc.insert("connections".into(), Doc::Array(conns));
    Ok(Doc::Object(doc))
}

/// Unix seconds as `YYYY-MM-DDTHH:MM:SSZ` (Howard Hinnant's civil_from_days).
fn utc(t: i64) -> String {
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn utc_formats_known_epochs() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_795_000_000), "2026-11-18T11:06:40Z");
        assert_eq!(
            crate::ciagent::parse::rfc3339_utc(&utc(1_795_000_000)),
            Some(1_795_000_000)
        );
    }

    use super::*;

    #[test]
    fn cid_matches_the_python_rule() {
        assert_eq!(cid("Hub_Caddy"), "hub-caddy");
        assert_eq!(cid("s-mac2-plant.x"), "s-mac2-plant-x");
        assert_eq!(cid(&"a".repeat(80)).len(), 64);
    }

    #[test]
    fn tabs_reads_every_tab_name() {
        let kdl = "layout {\n tab name=\"rivet\" {}\n tab name=\"spar\" focus=true {}\n}";
        assert_eq!(tabs(kdl), vec!["rivet", "spar"]);
    }

    #[test]
    fn credentials_split_name_kind_purpose() {
        let v =
            serde_json::to_value(credentials(&json!(["key|token|signs", "pw|password"]))).unwrap();
        assert_eq!(
            v,
            json!([{"name": "key", "kind": "token", "purpose": "signs"}, {"name": "pw", "kind": "password"}])
        );
    }

    #[test]
    fn objects_print_in_insertion_order() {
        let s = serde_json::to_string(&credentials(&json!(["key|token|signs"]))).unwrap();
        assert_eq!(s, r#"[{"name":"key","kind":"token","purpose":"signs"}]"#);
    }

    #[test]
    fn a_component_without_zone_is_an_error() {
        let inst = json!({"children": [{"name": "x", "package": "P", "type_name": "T",
            "typed_properties": {"PE_Arch::Kind": {"kind": "enum", "value": "service"}}}], "connections": []});
        assert!(model(&inst).unwrap_err().contains("Zone"));
    }

    #[test]
    fn sessions_of_the_configured_hosts() {
        let dir =
            std::env::temp_dir().join(format!("warte-ciagent-arch-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = dir.join("ansible");
        std::fs::create_dir_all(a.join("group_vars")).unwrap();
        std::fs::create_dir_all(a.join("host_vars/mac1")).unwrap();
        std::fs::write(
            a.join("group_vars/all.yml"),
            "warte_agent_tokens:\n  mac1: [rivet]\n",
        )
        .unwrap();
        std::fs::write(
            a.join("host_vars/mac1/main.yml"),
            "agent_sessions_github: [rivet]\n",
        )
        .unwrap();
        let (comps, conns) = sessions(&dir, &["mac1".to_string()]).unwrap();
        let c = serde_json::to_string(&comps).unwrap();
        assert!(
            c.starts_with(r#"[{"id":"s-mac1-rivet","name":"rivet (mac1)","kind":"session""#),
            "{c}"
        );
        assert_eq!(conns.len(), 2, "warte and brokered github");
        assert!(
            sessions(&dir, &[]).unwrap().0.is_empty(),
            "no hosts, no sessions"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
