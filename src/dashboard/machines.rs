//! Machines view: every machine the ground station hears from (REQ-PROBE-001,
//! REQ-PROBE-003), plus the CI host as the fleet agent reports it.
//! Host names, paths and tool output come from the machines: escaped.

use super::fleet::{dur_pub as dur, esc};
use crate::probe::{findings, Finding, Machines, Seen, MACHINE_STALE_S};

fn meter(label: &str, value: String, pct: f64, hot: bool) -> String {
    format!(
        r#"<div class="hbar"><span class="name">{label}</span><span class="bar"><i class="{}" style="width:{:.0}%"></i></span><span class="n">{value}</span></div>"#,
        if hot { "hot" } else { "" },
        pct.clamp(0.0, 100.0)
    )
}

/// What the history panel says when there is no store or it cannot be read.
pub const HISTORY_UNAVAILABLE: &str = r#"<p class="dim">history unavailable on this warte</p>"#;

/// An id-safe, collision-free form of a host name (hex of its bytes).
fn slug(host: &str) -> String {
    host.bytes().map(|b| format!("{b:02x}")).collect()
}

/// Percent-encode a query value.
fn qenc(v: &str) -> String {
    v.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Findings that have a chart behind them (REQ-MHIST-003).
fn charted(f: &Finding) -> bool {
    ["disk ", "load ", "memory", "swap", "io ", "IO ", "clock "]
        .iter()
        .any(|p| f.text.starts_with(p))
}

/// Three sparklines and the collapsed full history for one machine.
fn trend(
    h: &crate::history::History,
    s: &Seen,
    key: &str,
    span: i64,
    now: i64,
    open: &[String],
) -> String {
    let r = &s.report;
    let fullest = r
        .disks
        .iter()
        .max_by(|a, b| a.used_pct.total_cmp(&b.used_pct))
        .map(|d| {
            (
                format!(
                    "disk.used_pct{{mount={}}}",
                    crate::history::project::label(&d.mount)
                ),
                format!("disk {}", d.mount),
            )
        });
    let mut want = vec![
        ("load.per_core".to_string(), "load/core".to_string()),
        ("mem.used_pct".to_string(), "memory".to_string()),
    ];
    want.extend(fullest);
    let panel = open.contains(&r.host).then(|| {
        history_panel_full(
            h,
            &r.host,
            Some(key),
            r.health.as_ref(),
            r.hardware.as_ref(),
        )
    });
    trend_for(h, &r.host, &want, key, span, now, panel)
}

/// Sparklines of `want` (series, label) and the full history: collapsed and
/// loaded on first open, or open and filled in when `panel` is given (a
/// history the viewer had open, so a refresh keeps it open; REQ-MHIST-003).
fn trend_for(
    h: &crate::history::History,
    host: &str,
    want: &[(String, String)],
    key: &str,
    span: i64,
    now: i64,
    panel: Option<String>,
) -> String {
    let names: Vec<String> = want.iter().map(|(m, _)| m.clone()).collect();
    let series = h.series(host, &names, now - span, now);
    let sparks: String = series
        .iter()
        .zip(want)
        .map(|(x, (_, label))| super::charts::spark(label, x))
        .collect();
    let label = super::charts::RANGES
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, _, l)| *l)
        .unwrap_or("24 h");
    let details = match panel {
        Some(body) => format!(
            r#"<details class="mhist" open id="hist-{}" data-host="{}"><summary>history · {label}</summary><div class="mhbody">{body}</div></details>"#,
            slug(host),
            esc(host),
        ),
        None => format!(
            r#"<details class="mhist" id="hist-{}" data-host="{}" hx-get="api/dashboard/machine-history-partial?host={}&amp;range={key}" hx-trigger="toggle once" hx-target="find .mhbody"><summary>history · {label}</summary><div class="mhbody"><span class="dim">loading…</span></div></details>"#,
            slug(host),
            esc(host),
            qenc(host),
        ),
    };
    format!(r#"<div class="sparks" title="last {label}">{sparks}</div>{details}"#)
}

/// The full history of one machine, grouped (REQ-MHIST-003).
pub fn history_panel(h: &crate::history::History, host: &str, range: Option<&str>) -> String {
    history_panel_with(h, host, range, None)
}

/// The top processes of the latest report: names only, a snapshot, never history.
fn processes(x: &crate::probe::health::Health) -> String {
    let rows = |v: &[crate::probe::health::Proc], cpu: bool| -> String {
        v.iter()
            .map(|p| {
                let n = if p.count > 1 {
                    format!(" ×{}", p.count)
                } else {
                    String::new()
                };
                let val = if cpu {
                    format!("{:.0}% CPU", p.cpu_pct)
                } else {
                    format!("{:.0} MB", p.mem_mb)
                };
                format!(
                    "<li><b>{}</b>{n} <span class=\"dim\">{val}</span></li>",
                    esc(&p.name)
                )
            })
            .collect()
    };
    if x.top_cpu.is_empty() && x.top_mem.is_empty() {
        return String::new();
    }
    format!(
        r#"<div class="hgroup"><h4>Processes now</h4><div class="hprocs"><div><span class="dim">by CPU</span><ul>{}</ul></div><div><span class="dim">by memory</span><ul>{}</ul></div></div></div>"#,
        rows(&x.top_cpu, true),
        rows(&x.top_mem, false)
    )
}

/// The full history, plus the latest report's process list when given.
pub fn history_panel_with(
    h: &crate::history::History,
    host: &str,
    range: Option<&str>,
    latest: Option<&crate::probe::health::Health>,
) -> String {
    history_panel_full(h, host, range, latest, None)
}

/// What is attached now (REQ-PROBE-012): debug probes first, then the other
/// USB devices, then GPUs and disks; names only, a snapshot.
fn attached(x: &crate::probe::hardware::Hardware) -> String {
    let mut usb: Vec<&crate::probe::hardware::Usb> = x.usb.iter().collect();
    usb.sort_by_key(|u| u.probe.is_none());
    let rows: String = usb
        .iter()
        .map(|u| {
            let what = u
                .product
                .as_deref()
                .or(u.vendor.as_deref())
                .unwrap_or("USB device");
            let kind = u
                .probe
                .as_deref()
                .map(|k| format!(r#"<span class="badge green">{}</span> "#, esc(k)))
                .unwrap_or_default();
            let drv = u
                .driver
                .as_deref()
                .map(|d| format!(r#" <span class="dim">{}</span>"#, esc(d)))
                .unwrap_or_default();
            format!(
                "<li>{kind}<b>{}</b> <span class=\"dim\">{}</span>{drv}</li>",
                esc(what),
                esc(&u.id)
            )
        })
        .collect();
    let gpus: String = x
        .gpus
        .iter()
        .map(|g| {
            let cores = g.cores.map(|c| format!(" · {c} cores")).unwrap_or_default();
            format!("<li>GPU <b>{}</b>{cores}</li>", esc(&g.name))
        })
        .collect();
    let disks: String = x
        .disks
        .iter()
        .map(|d| {
            let smart = match d.smart_ok {
                Some(true) => " · SMART ok",
                Some(false) => " · SMART failing",
                None => "",
            };
            let wear = d
                .wear_pct
                .map(|w| format!(" · {w:.0}% worn"))
                .unwrap_or_default();
            format!(
                "<li>disk <b>{}</b> <span class=\"dim\">{}{smart}{wear}</span></li>",
                esc(&d.dev),
                esc(d.model.as_deref().unwrap_or(""))
            )
        })
        .collect();
    let gone: String = x
        .probes_gone
        .iter()
        .map(|g| format!(r#"<li class="dim">gone: {}</li>"#, esc(&g.probe)))
        .collect();
    if rows.is_empty() && gpus.is_empty() && disks.is_empty() && gone.is_empty() {
        return String::new();
    }
    format!(
        r#"<div class="hgroup"><h4>Attached now</h4><ul class="hattached">{rows}{gone}{gpus}{disks}</ul></div>"#
    )
}

/// The full history with the latest report's process list and hardware.
pub fn history_panel_full(
    h: &crate::history::History,
    host: &str,
    range: Option<&str>,
    latest: Option<&crate::probe::health::Health>,
    hw: Option<&crate::probe::hardware::Hardware>,
) -> String {
    let (_, span) = super::charts::range(range);
    let now = chrono::Utc::now().timestamp();
    let metrics = h.metrics(host);
    let procs = latest.map(processes).unwrap_or_default() + &hw.map(attached).unwrap_or_default();
    if metrics.is_empty() {
        return format!(r#"<p class="dim">no history for this machine yet</p>{procs}"#);
    }
    const GROUPS: [(&str, &[&str]); 14] = [
        ("CPU", &["load.", "psi.cpu"]),
        ("Memory", &["mem.", "swap.", "psi.mem", "oom."]),
        ("Disks", &["disk.used_pct"]),
        (
            "Disk health",
            &["disk.wear", "disk.media", "disk.power", "disk.temp"],
        ),
        ("IO", &["io.", "psi.io"]),
        ("Network", &["net."]),
        ("Temperature", &["temp."]),
        ("Health", &["health."]),
        ("Power", &["battery."]),
        ("Hardware", &["gpu.", "usb."]),
        ("Runners", &["runners.", "runner."]),
        ("Queue", &["queue."]),
        ("Agents", &["sessions"]),
        ("Uptime", &["uptime"]),
    ];
    let series = h.series(host, &metrics, now - span, now);
    let mut out = String::new();
    for (title, prefixes) in GROUPS {
        let charts: String = series
            .iter()
            .filter(|s| prefixes.iter().any(|p| s.metric.starts_with(p)))
            .filter(|s| s.values.iter().any(Option::is_some))
            .map(|s| super::charts::chart(&super::charts::title(&s.metric), s))
            .collect();
        if !charts.is_empty() {
            out += &format!(
                r#"<div class="hgroup"><h4>{title}</h4><div class="hcharts">{charts}</div></div>"#
            );
        }
    }
    if out.is_empty() {
        return format!(r#"<p class="dim">no data in this range; try a longer one</p>{procs}"#);
    }
    out + &procs
}

/// Hardware chips for a card: attached debug probes, a missing collector.
fn hardware_chips(x: &crate::probe::hardware::Hardware) -> String {
    let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
    for u in &x.usb {
        if let Some(k) = &u.probe {
            *kinds.entry(k.as_str()).or_default() += 1;
        }
    }
    let mut c: Vec<String> = kinds
        .iter()
        .map(|(k, n)| {
            let n = if *n > 1 {
                format!(" ×{n}")
            } else {
                String::new()
            };
            format!(r#"<span class="chip on">{}{n}</span>"#, esc(k))
        })
        .collect();
    // the status comes from the machine's report: only known values are shown
    let status = match x.privileged.as_deref() {
        Some("missing") => Some("missing"),
        Some("stale") => Some("stale"),
        Some("invalid") => Some("invalid"),
        Some("failing") => Some("failing"),
        _ => None,
    };
    if let Some(s) = status {
        c.push(format!(
            r#"<span class="chip off" title="warte probe privileged: {}">SMART: collector not running</span>"#,
            esc(s)
        ));
    }
    if c.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="mtools mhw">{}</div>"#, c.join(""))
    }
}

/// Health chips for a card: what needs a look, at a glance (REQ-PROBE-011).
fn health_chips(x: &crate::probe::health::Health) -> String {
    let mut c: Vec<String> = Vec::new();
    if let Some(u) = x.failed_units.as_ref().filter(|u| !u.is_empty()) {
        c.push(format!(
            r#"<span class="chip warn">{} failed unit(s)</span>"#,
            u.len()
        ));
    }
    if x.reboot_required_since.is_some() {
        c.push(r#"<span class="chip warn">reboot required</span>"#.into());
    }
    if let Some(u) = x.updates.as_ref().filter(|u| u.count > 0) {
        let sec = u
            .security
            .filter(|s| *s > 0)
            .map(|s| format!(", {s} security"))
            .unwrap_or_default();
        c.push(format!(
            r#"<span class="chip">{} update(s){sec}</span>"#,
            u.count
        ));
    }
    if let Some(b) = &x.battery {
        c.push(format!(
            r#"<span class="chip">battery {:.0}%{}</span>"#,
            b.pct,
            if b.charging == Some(true) {
                " charging"
            } else {
                ""
            }
        ));
    }
    if let Some(t) = x.temps.first() {
        c.push(format!(
            r#"<span class="chip">{} {:.0} °C</span>"#,
            esc(&t.sensor),
            t.c
        ));
    }
    if c.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="mtools mhealth">{}</div>"#, c.join(""))
    }
}

fn card(s: &Seen, mine: &[&Finding], ci: Option<&str>, now: i64, trend: &str) -> String {
    let r = &s.report;
    let age = now - s.received_at;
    let (pill, word) = if age <= MACHINE_STALE_S {
        ("pill", "live")
    } else {
        ("pill stale", "stale")
    };
    let mut body = String::new();
    if r.cpu.ncpu > 0 {
        body += &meter(
            "load",
            format!("{:.1} on {}", r.cpu.load[0], r.cpu.ncpu),
            r.cpu.load[0] * 100.0 / r.cpu.ncpu as f64,
            r.cpu.load[1] > r.cpu.ncpu as f64,
        );
    }
    if r.memory.total_mb > 0 {
        let used = r.memory.total_mb.saturating_sub(r.memory.avail_mb);
        let pct = used as f64 * 100.0 / r.memory.total_mb as f64;
        body += &meter(
            "memory",
            format!(
                "{:.1} of {:.1} GB",
                used as f64 / 1024.0,
                r.memory.total_mb as f64 / 1024.0
            ),
            pct,
            pct >= 90.0,
        );
    }
    for d in &r.disks {
        body += &meter(
            &esc(&d.mount),
            format!("{:.0}% of {:.0} GB", d.used_pct, d.size_gb),
            d.used_pct,
            d.used_pct >= 85.0,
        );
    }
    // IO (REQ-PROBE-008): per backing device, and pressure
    if let Some(io) = &r.io {
        for d in io.devices.iter().filter(|d| d.busy_pct.is_some()) {
            let busy = d.busy_pct.unwrap_or(0.0);
            let spin = d.rotational == Some(true);
            body += &meter(
                &format!("io {}{}", esc(&d.name), if spin { " (hdd)" } else { "" }),
                format!(
                    "{busy:.0}% busy{}",
                    d.await_ms
                        .map(|w| format!(" · {w:.0} ms"))
                        .unwrap_or_default()
                ),
                busy,
                spin && (busy >= 60.0 || d.await_ms.unwrap_or(0.0) >= 100.0),
            );
        }
        if let Some(p) = io.psi_some_avg60 {
            body += &meter("io pressure", format!("{p:.1}% stalled"), p, p >= 20.0);
        }
    }
    let tools: String = r
        .tools
        .iter()
        .map(|t| match (&t.path, &t.version) {
            (None, _) => format!(r#"<span class="chip off">{}</span>"#, esc(&t.name)),
            (_, Some(v)) => format!(
                r#"<span class="chip on">{} {}</span>"#,
                esc(&t.name),
                esc(v)
            ),
            (_, None) => format!(
                r#"<span class="chip warn" title="installed, version unknown">{} ?</span>"#,
                esc(&t.name)
            ),
        })
        .collect();
    let inv = r.inventory.as_ref().map(inventory).unwrap_or_default();
    let chips = r.health.as_ref().map(health_chips).unwrap_or_default()
        + &r.hardware.as_ref().map(hardware_chips).unwrap_or_default();
    let mut agents: std::collections::BTreeMap<&str, usize> = Default::default();
    for x in &r.sessions {
        *agents.entry(x.agent.as_str()).or_default() += 1;
    }
    let sessions = if agents.is_empty() {
        r#"<span class="dim">no agent sessions</span>"#.to_string()
    } else {
        agents
            .iter()
            .map(|(a, n)| format!(r#"<span class="badge green">{n} {}</span>"#, esc(a)))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let notes: String = mine
        .iter()
        .map(|f| {
            let link = if charted(f) && !trend.is_empty() {
                format!(
                    r#" <a class="flink" href="javascript:void(0)" onclick="var d=document.getElementById('hist-{}');if(d){{d.open=true;d.scrollIntoView({{block:'nearest'}})}}">chart</a>"#,
                    slug(&r.host)
                )
            } else {
                String::new()
            };
            format!(
                r#"<div class="finding {}">{}{link}</div>"#,
                f.severity,
                esc(&f.text)
            )
        })
        .collect();
    let probe = probe_line(s);
    let uptime = if r.uptime_s >= 86400 {
        format!("up {}d", r.uptime_s / 86400)
    } else {
        format!("up {}", dur(r.uptime_s as i64))
    };
    format!(
        r#"<div class="machine{}"><div class="mhead"><span class="mname">{}</span><span class="{pill}">{word} · {} ago</span></div><div class="mos">{} · {} · {} · {uptime}</div>{notes}<div class="hbars">{body}</div>{chips}{trend}<div class="mtools">{tools}</div>{inv}<div class="msess">{sessions}</div><div class="mvia">via {}{probe}{}</div></div>"#,
        if mine.iter().any(|f| f.severity == "red") {
            " red"
        } else if mine.is_empty() {
            ""
        } else {
            " amber"
        },
        esc(&r.host),
        dur(age),
        esc(&r.os.name),
        esc(&r.os.kernel),
        esc(&r.os.arch),
        esc(&s.via),
        ci.unwrap_or_default(),
    )
}

/// The probe's version and its certificate's end (REQ-PROBE-013), after
/// "via probe". The version is marked when it is behind the ground station's.
fn probe_line(s: &Seen) -> String {
    if s.via != "probe" {
        return String::new();
    }
    let ground = env!("CARGO_PKG_VERSION");
    let v = &s.report.probe_version;
    let ver = match crate::probe::version::clean(v) {
        Some(c) if crate::probe::version::behind(c, ground) => format!(
            r#" <span class="chip warn" title="behind the ground station's {ground}">{}</span>"#,
            esc(c)
        ),
        Some(c) => format!(r#" <span class="chip on">{}</span>"#, esc(c)),
        None => r#" <span class="chip off" title="version not reported">?</span>"#.to_string(),
    };
    let cert = s
        .cert_until
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|t| format!(" · cert until {}", t.format("%-d %b")))
        .unwrap_or_default();
    format!("{ver}{cert}")
}

/// The fleet agent's runner line for a CI host that also runs a probe: the
/// host keeps one card, and the agent's view adds what only it knows.
fn ci_line(hub: &crate::hub::Hub) -> Option<(String, String)> {
    let (s, _) = hub.snapshot()?;
    let busy = s.runners.iter().filter(|r| r.local_busy).count();
    Some((
        s.host.clone(),
        format!(
            " · CI host, {busy}/{} runners busy (fleet agent {}, CCSDS; details under Health)",
            s.runners.len(),
            esc(&s.agent_version)
        ),
    ))
}

/// Plugins, scheduled jobs and varve shadowing, folded (REQ-PROBE-010).
fn inventory(i: &crate::probe::inventory::Inventory) -> String {
    let failed = i.scheduled.iter().filter(|j| j.failed).count();
    let varve = match &i.shadowing {
        None => "varve: no agent session seen".to_string(),
        Some(s) if !s.shims_on_path => "varve: shims not on agents' PATH".into(),
        Some(s) if s.tools.is_empty() => "varve: pinned".into(),
        Some(s) => format!("varve: {} shadowed", s.tools.len()),
    };
    let mut summary = format!(
        "{} plugin(s) · {} scheduled",
        i.plugins.len(),
        i.scheduled.len()
    );
    if failed > 0 {
        summary += &format!(" ({failed} failed)");
    }
    summary += &format!(" · {varve}");
    let plugins: String = i
        .plugins
        .iter()
        .map(|p| {
            format!(
                r#"<span class="chip on" title="{}{}">{} {}</span>"#,
                esc(&p.marketplace),
                if p.scope.is_empty() {
                    String::new()
                } else {
                    format!(", {} scope", esc(&p.scope))
                },
                esc(&p.name),
                esc(&p.version)
            )
        })
        .collect();
    let jobs: String = i
        .scheduled
        .iter()
        .map(|j| {
            let state = if j.failed {
                match j.last_exit {
                    Some(x) if x < 0 => format!(" · killed ({x})"),
                    Some(x) => format!(" · exited {x}"),
                    None => " · failed".into(),
                }
            } else if j.running {
                " · running".into()
            } else {
                String::new()
            };
            format!(
                r#"<li class="{}">{} <span class="dim">{}{}</span></li>"#,
                if j.failed { "bad" } else { "" },
                esc(&j.name),
                esc(if j.schedule.is_empty() {
                    &j.via
                } else {
                    &j.schedule
                }),
                esc(&state)
            )
        })
        .collect();
    let shadowed: String = i
        .shadowing
        .iter()
        .flat_map(|s| &s.tools)
        .map(|t| {
            format!(
                r#"<span class="chip warn">{} {} · {}</span>"#,
                esc(&t.tool),
                esc(t.version.as_deref().unwrap_or("?")),
                crate::probe::inventory::origin(&t.path)
            )
        })
        .collect();
    // the count only: error output can carry paths (warte#46)
    let errors = if i.errors.is_empty() {
        String::new()
    } else {
        format!(
            r#"<div class="dim">{} source(s) could not be read; see <code>warte probe report</code> on the machine</div>"#,
            i.errors.len()
        )
    };
    format!(
        r#"<details class="minv"><summary>{}</summary>{}{}{}{errors}</details>"#,
        esc(&summary),
        if shadowed.is_empty() {
            String::new()
        } else {
            format!(
                r#"<div class="mtools"><span class="dim">unpinned on agents' PATH:</span> {shadowed}</div>"#
            )
        },
        if plugins.is_empty() {
            String::new()
        } else {
            format!(r#"<div class="mtools">{plugins}</div>"#)
        },
        if jobs.is_empty() {
            String::new()
        } else {
            format!(r#"<ul class="mjobs">{jobs}</ul>"#)
        },
    )
}

/// The CI host as the fleet agent reports it: a smaller card, no tools.
fn ci_card(
    hub: &crate::hub::Hub,
    now: i64,
    hist: Option<(&crate::history::History, &str, i64, &[String])>,
) -> Option<String> {
    let (s, at) = hub.snapshot()?;
    let m = &s.machine;
    let mut body = String::new();
    if m.ncpu > 0 {
        body += &meter(
            "load",
            format!("{:.1} on {}", m.load[0], m.ncpu),
            m.load[0] * 100.0 / m.ncpu as f64,
            m.load[1] > m.ncpu as f64,
        );
        let used = m.mem_total_mb.saturating_sub(m.mem_avail_mb);
        body += &meter(
            "memory",
            format!(
                "{:.0} of {:.0} GB",
                used as f64 / 1024.0,
                m.mem_total_mb as f64 / 1024.0
            ),
            used as f64 * 100.0 / m.mem_total_mb.max(1) as f64,
            false,
        );
    }
    for d in &m.disks {
        body += &meter(
            &esc(&d.mount),
            format!("{:.0}% of {:.0} GB", d.used_pct, d.size_gb),
            d.used_pct,
            d.used_pct >= 85.0,
        );
    }
    let busy = s.runners.iter().filter(|r| r.local_busy).count();
    let trend = hist
        .map(|(h, key, span, open)| {
            let want = [
                ("load.1", "load"),
                ("runners.busy", "busy runners"),
                ("queue.self_hosted", "queued"),
            ]
            .map(|(m, l)| (m.to_string(), l.to_string()));
            let panel = open
                .contains(&s.host)
                .then(|| history_panel_with(h, &s.host, Some(key), None));
            trend_for(h, &s.host, &want, key, span, now, panel)
        })
        .unwrap_or_default();
    Some(format!(
        r#"<div class="machine"><div class="mhead"><span class="mname">{}</span><span class="pill">live · {} ago</span></div><div class="mos">CI host · {busy}/{} runners busy</div><div class="hbars">{body}</div>{trend}<div class="mvia">via fleet agent {} (CCSDS); details under Health</div></div>"#,
        esc(&s.host),
        dur(now - at),
        s.runners.len(),
        esc(&s.agent_version),
    ))
}

pub fn view(machines: Option<&Machines>, hub: Option<&crate::hub::Hub>) -> String {
    view_range(machines, hub, None)
}

/// The Machines view with trend lines over `range` (`1h`, `24h`, `7d`, `90d`;
/// default 24 h) when machine history is on (REQ-MHIST-003).
pub fn view_range(
    machines: Option<&Machines>,
    hub: Option<&crate::hub::Hub>,
    range: Option<&str>,
) -> String {
    view_open(machines, hub, range, &[])
}

/// As `view_range`, with the histories of `open` hosts already open and
/// filled in: the viewer had them open, so a refresh must not fold them.
pub fn view_open(
    machines: Option<&Machines>,
    hub: Option<&crate::hub::Hub>,
    range: Option<&str>,
    open: &[String],
) -> String {
    let now = chrono::Utc::now().timestamp();
    let (key, span) = super::charts::range(range);
    let hist = machines.and_then(Machines::history);
    let seen = machines.map(Machines::list).unwrap_or_default();
    let mut all = findings(&seen, now);
    all.extend(
        machines
            .and_then(Machines::history)
            .and_then(|h| h.finding(now)),
    );
    let ci = hub.and_then(ci_line);
    let probed = |host: &str| {
        seen.iter()
            .any(|s| s.report.host.eq_ignore_ascii_case(host))
    };
    let mut cards: String = seen
        .iter()
        .map(|s| {
            let mine: Vec<&Finding> = all.iter().filter(|f| f.host == s.report.host).collect();
            let line = ci
                .as_ref()
                .filter(|(h, _)| h.eq_ignore_ascii_case(&s.report.host))
                .map(|(_, l)| l.as_str());
            let t = hist
                .map(|h| trend(h, s, key, span, now, open))
                .unwrap_or_default();
            card(s, &mine, line, now, &t)
        })
        .collect();
    // A CI host without a probe still shows, from the agent alone.
    if !ci.as_ref().is_some_and(|(h, _)| probed(h)) {
        if let Some(c) = hub.and_then(|h| ci_card(h, now, hist.map(|x| (x, key, span, open)))) {
            cards += &c;
        }
    }
    let summary = if all.is_empty() {
        r#"<div class="allclear"><span class="pill">no drift</span> Every machine reporting is current and consistent.</div>"#.to_string()
    } else {
        all.iter()
            .map(|f| {
                format!(
                    r#"<div class="incident sev{}"><span class="badge {}">{}</span><b>{}</b>&nbsp;{}</div>"#,
                    if f.severity == "red" { 2 } else { 1 },
                    if f.severity == "red" { "red" } else { "yellow" },
                    if f.severity == "red" { "critical" } else { "drift" },
                    esc(&f.host),
                    esc(&f.text)
                )
            })
            .collect()
    };
    let empty = if cards.is_empty() {
        r#"<p class="empty">No machine has reported yet. The ground station reports itself within a minute of starting; other machines appear once enrolled (<code>warte probe</code>).</p>"#
    } else {
        ""
    };
    let switch = if hist.is_some() && !cards.is_empty() {
        let b: String = super::charts::RANGES
            .iter()
            .map(|(k, _, l)| {
                format!(
                    r##"<button class="rbtn{}" hx-get="api/dashboard/machines-partial?range={k}" hx-vals="js:{{open: warteOpen()}}" hx-target="#content" hx-swap="innerHTML" onclick="warteSetRange('{k}')">{l}</button>"##,
                    if *k == key { " on" } else { "" }
                )
            })
            .collect();
        format!(r#"<div class="hrange" role="group" aria-label="history range">history {b}</div>"#)
    } else {
        String::new()
    };
    format!(
        r#"<div class="overview"><h2>Machines</h2>{switch}<section class="panel"><h3>Drift and problems</h3>{summary}</section>{empty}<div class="machines">{cards}</div><div class="note">Each machine reports every minute with <code>warte probe</code>. Agent sessions show the agent and working directory only, never command lines. More machines appear as they are enrolled.</div></div>"#
    )
}

/// Model vs reality, and the modelled machines that do not report, as dashed
/// cards (REQ-FLEET-001). Empty when no model is configured.
pub fn model_panel(
    fleet: Option<&Result<crate::fleet::Fleet, String>>,
    reporting: &[String],
) -> String {
    let f = match fleet {
        None => return String::new(),
        Some(Err(e)) => {
            return format!(
                r#"<section class="panel"><h3>Fleet model</h3><div class="incident sev1"><span class="badge yellow">model</span>{}</div></section>"#,
                esc(e)
            )
        }
        Some(Ok(f)) => f,
    };
    let diffs = crate::fleet::differences(f, reporting);
    let list = if diffs.is_empty() {
        r#"<div class="allclear"><span class="pill">matches</span> Every modelled machine reports, and nothing reports that is not modelled.</div>"#.to_string()
    } else {
        diffs
            .iter()
            .map(|d| format!(r#"<div class="incident sev1"><span class="badge yellow">model</span><b>{}</b>&nbsp;{}</div>"#, esc(&d.host), esc(&d.text)))
            .collect()
    };
    let missing: String = f
        .machines
        .iter()
        .filter(|m| !reporting.iter().any(|r| r.eq_ignore_ascii_case(&m.host)))
        .map(|m| {
            let boards: Vec<String> = m.boards.iter().map(|b| format!(r#"<span class="chip">{}</span>"#, esc(&b.catalog))).collect();
            format!(
                r#"<div class="machine modelled"><div class="mhead"><span class="mname">{}</span><span class="pill stale">not reporting</span></div><div class="mos">modelled: {} · {}{}</div><div class="mtools">{}</div><div class="mvia">from the fleet model</div></div>"#,
                esc(&m.host),
                esc(&m.kind),
                esc(&m.location),
                m.cores.map(|c| format!(" · {c} cores")).unwrap_or_default(),
                boards.join("")
            )
        })
        .collect();
    format!(
        r#"<section class="panel"><h3>Fleet model vs reality · {}</h3>{list}</section><div class="machines">{missing}</div>"#,
        esc(&f.root)
    )
}
