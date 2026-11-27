//! The architecture view's contract, `architecture.v1` (REQ-ARCHVIEW-001).
//! The schema and [`check`] live in the warte-check crate, which also builds
//! as a WASI component; this module reads the configured file and renders it.

pub use warte_check::architecture::*;

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
