//! The architecture view's contract, `architecture.v1` (REQ-ARCHVIEW-001):
//! hosts, services, agent sessions, credential stores, trust zones and how
//! they connect, as an export from the operator's AADL model. warte owns the
//! schema, checks it ([`check`], `warte arch check`) and renders it; it
//! never holds a credential's value, only its name and kind.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "architecture.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Architecture {
    pub schema: String,
    pub generated_at: String,
    pub source: Source,
    pub zones: Vec<Zone>,
    pub components: Vec<Component>,
    pub connections: Vec<Connection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub tool: String,
    pub version: String,
    pub model: String,
    #[serde(default)]
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Trust {
    Trusted,
    Internal,
    Dmz,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub id: String,
    pub name: String,
    pub trust: Trust,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Host,
    Service,
    Session,
    CredentialStore,
    Device,
    External,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialKind {
    PrivateKey,
    Token,
    Password,
    ApiKey,
    Certificate,
    Seed,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credential {
    pub name: String,
    pub kind: CredentialKind,
    #[serde(default)]
    pub purpose: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub zone: String,
    /// The host component it runs on (not for a host).
    #[serde(default)]
    pub host: Option<String>,
    /// STPA-Sec controller ids; one host may run several controllers.
    #[serde(default)]
    pub controller: Vec<String>,
    /// Names and kinds only, never values.
    #[serde(default)]
    pub credentials: Vec<Credential>,
    /// The spar source element.
    #[serde(default)]
    pub aadl: Option<String>,
    /// For a session: the registered agent name it runs as.
    #[serde(default)]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Quic,
    Https,
    Ssh,
    Udp,
    Wireguard,
    Unix,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub id: String,
    pub from: String,
    pub to: String,
    pub protocol: Protocol,
    #[serde(default)]
    pub port: Option<u16>,
    pub authenticated: bool,
    pub encrypted: bool,
    /// `<component id>/<credential name>` it presents.
    #[serde(default)]
    pub credential: Option<String>,
    /// When absent it is derived from the endpoints' zones; when given it must
    /// agree with them.
    #[serde(default)]
    pub zone_crossing: Option<bool>,
    #[serde(default)]
    pub controller: Vec<String>,
}

/// A checked architecture with what warte derived and found.
#[derive(Debug, Clone, Serialize)]
pub struct Checked {
    pub architecture: Architecture,
    /// Connection id → crosses a zone boundary.
    pub crossings: BTreeMap<String, bool>,
    /// Worth a look, not an error: a zone crossing without authentication or
    /// encryption.
    pub findings: Vec<String>,
}

fn id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Prefixes and markers of secret values: a credential's value never belongs
/// in the export.
const SECRET_MARKERS: [&str; 10] = [
    "-----BEGIN",
    "ghp_",
    "gho_",
    "ghs_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "AKIA",
    "sk-",
    "warte1.",
];

fn looks_secret(s: &str) -> bool {
    SECRET_MARKERS.iter().any(|m| s.contains(m))
        || (s.starts_with("eyJ") && s.len() > 40 && s.matches('.').count() == 2)
}

/// Every string in the document whose value looks like a secret, by path.
fn secrets(v: &Value, path: &str, out: &mut Vec<String>) {
    match v {
        Value::String(s) if looks_secret(s) => out.push(path.to_string()),
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                secrets(x, &format!("{path}[{i}]"), out);
            }
        }
        Value::Object(m) => {
            for (k, x) in m {
                secrets(x, &format!("{path}.{k}"), out);
            }
        }
        _ => {}
    }
}

/// Check an `architecture.v1` document. `Err` lists every error, naming the
/// field or id, never a value.
pub fn check(text: &str) -> Result<Checked, Vec<String>> {
    let raw: Value =
        serde_json::from_str(text).map_err(|e| vec![format!("not JSON: line {}", e.line())])?;
    let mut leaked = Vec::new();
    secrets(&raw, "$", &mut leaked);
    if !leaked.is_empty() {
        return Err(leaked
            .into_iter()
            .map(|p| {
                format!("{p}: looks like a secret value; the export carries names and kinds only")
            })
            .collect());
    }
    let a: Architecture = serde_json::from_value(raw).map_err(|e| {
        // serde names the field; drop anything after a value quote
        let m = e.to_string();
        vec![format!(
            "schema: {}",
            m.split('`').next().unwrap_or(&m).trim_end()
        )]
    })?;
    let mut errs = Vec::new();
    if a.schema != SCHEMA {
        errs.push(format!("schema: must be {SCHEMA}"));
    }
    let mut ids = BTreeSet::new();
    let all_ids = a
        .zones
        .iter()
        .map(|z| &z.id)
        .chain(a.components.iter().map(|c| &c.id))
        .chain(a.connections.iter().map(|c| &c.id));
    for id in all_ids {
        if !id_ok(id) {
            errs.push(format!("{id:?}: an id is [a-z0-9-], at most 64"));
        } else if !ids.insert(id.clone()) {
            errs.push(format!("{id}: declared more than once"));
        }
    }
    let zones: BTreeMap<&str, &Zone> = a.zones.iter().map(|z| (z.id.as_str(), z)).collect();
    let comps: BTreeMap<&str, &Component> =
        a.components.iter().map(|c| (c.id.as_str(), c)).collect();
    for c in &a.components {
        if !zones.contains_key(c.zone.as_str()) {
            errs.push(format!("{}.zone: no zone {}", c.id, c.zone));
        }
        match (&c.host, c.kind) {
            (Some(_), Kind::Host) => errs.push(format!("{}.host: a host runs on no host", c.id)),
            (Some(h), _) if comps.get(h.as_str()).map(|x| x.kind) != Some(Kind::Host) => {
                errs.push(format!("{}.host: {h} is not a host component", c.id))
            }
            _ => {}
        }
        if c.agent.is_some() && c.kind != Kind::Session {
            errs.push(format!("{}.agent: only a session runs as an agent", c.id));
        }
        let mut names = BTreeSet::new();
        for cr in &c.credentials {
            if !names.insert(&cr.name) {
                errs.push(format!("{}.credentials: {} declared twice", c.id, cr.name));
            }
        }
    }
    let mut crossings = BTreeMap::new();
    let mut findings = Vec::new();
    for e in &a.connections {
        let (Some(f), Some(t)) = (comps.get(e.from.as_str()), comps.get(e.to.as_str())) else {
            errs.push(format!("{}: from and to must name components", e.id));
            continue;
        };
        if let Some(cred) = &e.credential {
            let ok = cred.split_once('/').is_some_and(|(cid, name)| {
                comps
                    .get(cid)
                    .is_some_and(|c| c.credentials.iter().any(|x| x.name == name))
            });
            if !ok {
                errs.push(format!(
                    "{}.credential: no such credential on that component",
                    e.id
                ));
            }
        }
        let crosses = f.zone != t.zone;
        if e.zone_crossing.is_some_and(|z| z != crosses) {
            errs.push(format!(
                "{}.zone_crossing: disagrees with the endpoints' zones",
                e.id
            ));
        }
        crossings.insert(e.id.clone(), crosses);
        if crosses && !(e.authenticated && e.encrypted) {
            let missing = match (e.authenticated, e.encrypted) {
                (false, false) => "authentication and encryption",
                (false, true) => "authentication",
                _ => "encryption",
            };
            findings.push(format!(
                "{} crosses from {} to {} without {missing}",
                e.id, f.zone, t.zone
            ));
        }
    }
    if errs.is_empty() {
        Ok(Checked {
            architecture: a,
            crossings,
            findings,
        })
    } else {
        Err(errs)
    }
}

/// The configured file, read and checked now: `None` without `[architecture]`.
pub fn load(
    cfg: Option<&crate::config::ArchitectureConfig>,
) -> Option<Result<Checked, Vec<String>>> {
    let cfg = cfg?;
    Some(match std::fs::read_to_string(&cfg.file) {
        Ok(t) => check(&t),
        Err(_) => Err(vec!["the architecture file cannot be read".into()]),
    })
}

fn esc(s: &str) -> String {
    crate::dashboard::fleet::esc(s)
}

/// The view (REQ-ARCHVIEW-001): components grouped by zone, then the
/// connections, zone crossings and insecure edges first, then where each
/// credential lives and who presents it.
pub fn view(c: &Checked) -> String {
    let a = &c.architecture;
    let name = |id: &str| {
        a.components
            .iter()
            .find(|x| x.id == id)
            .map_or(id.to_string(), |x| x.name.clone())
    };
    let mut zones = String::new();
    for z in &a.zones {
        let rows: String = a
            .components
            .iter()
            .filter(|x| x.zone == z.id)
            .map(|x| {
                let creds: Vec<String> = x
                    .credentials
                    .iter()
                    .map(|cr| format!("{} ({})", esc(&cr.name), esc(&format!("{:?}", cr.kind).to_lowercase())))
                    .collect();
                let agent = x
                    .agent
                    .as_deref()
                    .map(|n| format!(r##" · agent <a href="#agents">{}</a>"##, esc(n)))
                    .unwrap_or_default();
                format!(
                    "<tr><td><b>{}</b>{agent}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    esc(&x.name),
                    esc(&format!("{:?}", x.kind).to_lowercase()),
                    x.host.as_deref().map(|h| esc(&name(h))).unwrap_or_default(),
                    esc(&x.controller.join(", ")),
                    creds.join(", "),
                )
            })
            .collect();
        zones.push_str(&format!(
            r#"<h3>{} <span class="pill">{}</span></h3><div class="scroll"><table class="data-table"><thead><tr><th>Component</th><th>Kind</th><th>Runs on</th><th>Controllers</th><th>Credentials held</th></tr></thead><tbody>{rows}</tbody></table></div>"#,
            esc(&z.name),
            esc(&format!("{:?}", z.trust).to_lowercase()),
        ));
    }
    let mut edges: Vec<&Connection> = a.connections.iter().collect();
    // insecure crossings first, then other crossings, then the rest
    edges.sort_by_key(|e| {
        let crosses = c.crossings.get(&e.id).copied().unwrap_or(false);
        (!(crosses && !(e.authenticated && e.encrypted)), !crosses)
    });
    let rows: String = edges
        .iter()
        .map(|e| {
            let crosses = c.crossings.get(&e.id).copied().unwrap_or(false);
            let insecure = crosses && !(e.authenticated && e.encrypted);
            format!(
                r#"<tr class="{}"><td>{} → {}</td><td>{}{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                if insecure { "warnrow" } else { "" },
                esc(&name(&e.from)),
                esc(&name(&e.to)),
                esc(&format!("{:?}", e.protocol).to_lowercase()),
                e.port.map(|p| format!(":{p}")).unwrap_or_default(),
                if crosses { "crosses a zone" } else { "" },
                if e.authenticated { "yes" } else { "no" },
                if e.encrypted { "yes" } else { "no" },
                e.credential.as_deref().map(esc).unwrap_or_default(),
            )
        })
        .collect();
    let findings = if c.findings.is_empty() {
        r#"<p class="empty">Every connection that crosses a zone is authenticated and encrypted.</p>"#.to_string()
    } else {
        format!(
            "<ul>{}</ul>",
            c.findings
                .iter()
                .map(|f| format!("<li>{}</li>", esc(f)))
                .collect::<String>()
        )
    };
    format!(
        r#"<div class="overview architecture"><h2>Architecture</h2><p>From {} {}, generated {}. Credentials are named, never shown.</p><h3>Worth a look</h3>{findings}{zones}<h3>Connections</h3><div class="scroll"><table class="data-table"><thead><tr><th>From → to</th><th>Protocol</th><th>Zone</th><th>Authenticated</th><th>Encrypted</th><th>Presents</th></tr></thead><tbody>{rows}</tbody></table></div></div>"#,
        esc(&a.source.tool),
        esc(&a.source.version),
        esc(&a.generated_at),
    )
}
