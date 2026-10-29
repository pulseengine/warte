//! The fleet model: what a deployment intends to exist (REQ-FLEET-001).
//!
//! The model is AADL (`architecture/fleet/warte_fleet.aadl` types, the
//! deployment's own instance), instantiated by spar
//! (`spar instance --format json`). warte reads that JSON, draws it, and
//! reports where reality differs: a modelled machine that does not report, a
//! reporting machine that is not modelled. spar models; warte renders.

use serde::Serialize;
use serde_json::Value;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Board {
    /// The instance name in the model (`pixhawk`).
    pub id: String,
    /// The with-device catalogue name (`pixhawk-6xrt`).
    pub catalog: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Machine {
    pub id: String,
    /// The name the machine's probe reports; falls back to `id`.
    pub host: String,
    /// terminal, workshop, cargo_hall, server or github_hosted.
    pub kind: String,
    /// `lan` or `remote`, from the bus the machine is attached to (the
    /// `Location` property where no bus says it).
    pub location: String,
    pub cores: Option<u32>,
    pub runners: Option<u32>,
    pub ground_station: bool,
    pub boards: Vec<Board>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
pub struct Fleet {
    /// `Package::Type.Impl` of the instantiated root.
    pub root: String,
    pub machines: Vec<Machine>,
    /// What the model itself gets wrong (a board on no machine, a location
    /// property that contradicts the bus).
    pub problems: Vec<String>,
}

/// spar writes property values as AADL literal text: `"4"`, `"\"name\""`.
fn literal(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim();
    Some(s.trim_matches('"').to_string())
}

fn prop(node: &Value, name: &str) -> Option<String> {
    literal(node["properties"].get(format!("Warte_Fleet::{name}"))?)
}

/// The component a connection end names (`fourpi.usb` → `fourpi`).
fn owner(end: &str) -> &str {
    end.split('.').next().unwrap_or(end)
}

/// Read a spar instance (`spar instance --format json`) of a Warte_Fleet model.
pub fn from_spar(json: &str) -> Result<Fleet, String> {
    let root: Value =
        serde_json::from_str(json).map_err(|e| format!("not spar instance JSON: {e}"))?;
    let children = root["children"]
        .as_array()
        .ok_or("not spar instance JSON: no children")?;
    let find = |name: &str| children.iter().find(|c| c["name"] == name);
    let mut problems = vec![];
    let mut machines: Vec<Machine> = children
        .iter()
        .filter(|c| c["category"] == "system")
        .map(|c| {
            let id = c["name"].as_str().unwrap_or_default().to_string();
            Machine {
                host: prop(c, "Host_Name").unwrap_or_else(|| id.clone()),
                kind: prop(c, "Kind").unwrap_or_else(|| "server".into()),
                location: prop(c, "Location").unwrap_or_default(),
                cores: prop(c, "Cores").and_then(|v| v.parse().ok()),
                runners: prop(c, "Runners").and_then(|v| v.parse().ok()),
                ground_station: c["type_name"] == "GroundStation",
                boards: vec![],
                id,
            }
        })
        .collect();
    let mut placed = std::collections::BTreeSet::new();
    for conn in root["connections"].as_array().into_iter().flatten() {
        let (src, dst) = (
            conn["source"].as_str().unwrap_or_default(),
            conn["destination"].as_str().unwrap_or_default(),
        );
        let (a, b) = (owner(src), owner(dst));
        let (Some(ca), Some(cb)) = (find(a), find(b)) else {
            continue;
        };
        match (ca["category"].as_str(), cb["category"].as_str()) {
            // a network: Lan or Tunnel decides where the machine lives
            (Some("bus"), Some("system")) => {
                let loc = if ca["type_name"] == "Tunnel" {
                    "remote"
                } else {
                    "lan"
                };
                if let Some(m) = machines.iter_mut().find(|m| m.id == b) {
                    if !m.location.is_empty() && m.location != loc {
                        problems.push(format!(
                            "{}: Location says {} but it is on the {} bus",
                            m.id,
                            m.location,
                            ca["type_name"].as_str().unwrap_or("?")
                        ));
                    }
                    m.location = loc.into();
                }
            }
            // a board on a machine's USB
            (Some("system"), Some("device")) => {
                if let Some(m) = machines.iter_mut().find(|m| m.id == a) {
                    m.boards.push(Board {
                        id: b.into(),
                        catalog: prop(cb, "Catalog_Name").unwrap_or_else(|| b.into()),
                    });
                    placed.insert(b.to_string());
                }
            }
            _ => {}
        }
    }
    for d in children.iter().filter(|c| c["category"] == "device") {
        let id = d["name"].as_str().unwrap_or_default();
        if !placed.contains(id) {
            problems.push(format!("board {id} is attached to no machine"));
        }
    }
    for m in machines.iter_mut().filter(|m| m.location.is_empty()) {
        problems.push(format!(
            "{}: on no network (no Lan or Tunnel bus, no Location)",
            m.id
        ));
        m.location = "lan".into();
    }
    Ok(Fleet {
        root: format!(
            "{}::{}.{}",
            root["package"].as_str().unwrap_or_default(),
            root["type_name"].as_str().unwrap_or_default(),
            root["impl_name"].as_str().unwrap_or_default()
        ),
        machines,
        problems,
    })
}

/// A difference between the model and what reports in.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Difference {
    pub host: String,
    pub text: String,
}

/// Model against reality: `reporting` are the hosts that report now (probes
/// and the CI host's agent). Host names compare case-insensitively.
pub fn differences(f: &Fleet, reporting: &[String]) -> Vec<Difference> {
    let has = |h: &str| reporting.iter().any(|r| r.eq_ignore_ascii_case(h));
    let mut out: Vec<Difference> = f
        .machines
        .iter()
        .filter(|m| !has(&m.host))
        .map(|m| Difference {
            host: m.host.clone(),
            text: "in the fleet model, not reporting (not enrolled, or down)".into(),
        })
        .collect();
    out.extend(
        reporting
            .iter()
            .filter(|r| !f.machines.iter().any(|m| m.host.eq_ignore_ascii_case(r)))
            .map(|r| Difference {
                host: r.clone(),
                text: "reporting, but not in the fleet model".into(),
            }),
    );
    out.extend(f.problems.iter().map(|p| Difference {
        host: "model".into(),
        text: p.clone(),
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../tests/fixtures/example_fleet.instance.json");

    #[test]
    fn reads_machines_locations_and_boards_from_the_spar_instance() {
        let f = from_spar(EXAMPLE).unwrap();
        assert_eq!(f.root, "Example_Fleet::ExampleFleet.impl");
        let m = |id: &str| f.machines.iter().find(|m| m.id == id).unwrap().clone();
        assert!(m("tower").ground_station);
        assert_eq!(
            (m("ci").location.as_str(), m("ci").runners, m("ci").cores),
            ("remote", Some(14), Some(32))
        );
        assert_eq!(m("laptop").location, "lan");
        // AADL string quotes are gone; the board sits on the machine its USB bus says
        assert_eq!(
            m("tower").boards,
            vec![Board {
                id: "flight_ctrl".into(),
                catalog: "pixhawk-6xrt".into()
            }]
        );
        assert_eq!(m("pi_bench").boards[0].catalog, "nucleo-g474re");
        assert!(f.problems.is_empty(), "{:?}", f.problems);
    }

    #[test]
    fn model_against_reality_both_ways() {
        let f = from_spar(EXAMPLE).unwrap();
        let d = differences(&f, &["TOWER".into(), "laptop".into(), "stranger".into()]);
        let says = |h: &str| {
            d.iter()
                .filter(|x| x.host == h)
                .map(|x| x.text.clone())
                .collect::<Vec<_>>()
        };
        assert!(says("pi_bench")[0].starts_with("in the fleet model, not reporting"));
        assert!(says("ci")[0].starts_with("in the fleet model, not reporting"));
        assert_eq!(
            says("stranger"),
            vec!["reporting, but not in the fleet model"]
        );
        assert!(
            says("tower").is_empty(),
            "host names compare case-insensitively"
        );
    }

    #[test]
    fn a_broken_model_says_what_is_wrong() {
        let mut v: Value = serde_json::from_str(EXAMPLE).unwrap();
        // drop the USB connection of the nucleo
        v["connections"]
            .as_array_mut()
            .unwrap()
            .retain(|c| c["name"] != "u_nucleo");
        let f = from_spar(&v.to_string()).unwrap();
        assert_eq!(f.problems, vec!["board nucleo is attached to no machine"]);
        assert!(from_spar("{}").is_err());
    }
}
