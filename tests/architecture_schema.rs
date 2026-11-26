//! The published JSON Schema for architecture.v1 agrees with `check()`
//! (REQ-ARCHVIEW-002): every document the schema refuses, `check()` refuses
//! too, and the documents only `check()` refuses are exactly the rules the
//! schema's description lists as beyond JSON Schema.

use serde_json::{json, Value};
use warte::architecture::check;

const EXAMPLE: &str = include_str!("fixtures/architecture.example.json");
const SCHEMA: &str = include_str!("../schemas/architecture.v1.json");

type Mutation = (&'static str, Box<dyn Fn(&mut Value)>);

fn example() -> Value {
    serde_json::from_str(EXAMPLE).unwrap()
}

fn schema_ok(v: &Value) -> bool {
    let s: Value = serde_json::from_str(SCHEMA).unwrap();
    jsonschema::validator_for(&s).unwrap().is_valid(v)
}

fn check_ok(v: &Value) -> bool {
    check(&v.to_string()).is_ok()
}

fn mutated(f: &dyn Fn(&mut Value)) -> Value {
    let mut v = example();
    f(&mut v);
    v
}

// rivet: verifies REQ-ARCHVIEW-002
#[test]
fn documents_both_accept() {
    let mut cases: Vec<Mutation> = vec![
        ("the example", Box::new(|_| {})),
        (
            "optional fields as null",
            Box::new(|v| {
                v["source"]["commit"] = Value::Null;
                v["connections"][0]["port"] = Value::Null;
                v["connections"][0]["zone_crossing"] = Value::Null;
            }),
        ),
        (
            "port at the u16 bounds",
            Box::new(|v| v["connections"][0]["port"] = json!(65535)),
        ),
        (
            "empty lists",
            Box::new(|v| {
                v["components"][0]["controller"] = json!([]);
                v["components"][6]["credentials"] = json!([]);
            }),
        ),
        (
            "several controllers on one connection",
            Box::new(|v| v["connections"][0]["controller"] = json!(["CTRL-001", "CTRL-002"])),
        ),
    ];
    // every enum value the schema publishes is one check() accepts
    let s: Value = serde_json::from_str(SCHEMA).unwrap();
    let values = |p: &str| -> Vec<String> {
        s.pointer(p)
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    };
    for t in values("/$defs/zone/properties/trust/enum") {
        cases.push((
            "zone trust",
            Box::new(move |v| v["zones"][0]["trust"] = json!(t)),
        ));
    }
    for k in values("/$defs/credential/properties/kind/enum") {
        cases.push((
            "credential kind",
            Box::new(move |v| v["components"][1]["credentials"][0]["kind"] = json!(k)),
        ));
    }
    // c-sensor (index 6) has no host, agent or credentials, so any kind but
    // session-with-agent rules apply cleanly
    for k in values("/$defs/component/properties/kind/enum") {
        cases.push((
            "component kind",
            Box::new(move |v| v["components"][6]["kind"] = json!(k)),
        ));
    }
    for p in values("/$defs/connection/properties/protocol/enum") {
        cases.push((
            "protocol",
            Box::new(move |v| v["connections"][0]["protocol"] = json!(p)),
        ));
    }
    assert!(cases.len() > 25, "{}", cases.len());
    for (name, f) in &cases {
        let v = mutated(f.as_ref());
        assert!(schema_ok(&v), "schema refused: {name}");
        assert!(check_ok(&v), "check refused: {name}");
    }
}

// rivet: verifies REQ-ARCHVIEW-002
#[test]
fn shape_errors_both_refuse() {
    let cases: Vec<Mutation> = vec![
        (
            "schema version",
            Box::new(|v| v["schema"] = json!("architecture.v2")),
        ),
        (
            "unknown top-level field",
            Box::new(|v| v["extra"] = json!(1)),
        ),
        (
            "missing generated_at",
            Box::new(|v| {
                v.as_object_mut().unwrap().remove("generated_at");
            }),
        ),
        (
            "missing source.model",
            Box::new(|v| {
                v["source"].as_object_mut().unwrap().remove("model");
            }),
        ),
        (
            "unknown source field",
            Box::new(|v| v["source"]["branch"] = json!("main")),
        ),
        (
            "unknown zone field",
            Box::new(|v| v["zones"][0]["colour"] = json!("red")),
        ),
        (
            "unknown trust",
            Box::new(|v| v["zones"][0]["trust"] = json!("public")),
        ),
        (
            "unknown component kind",
            Box::new(|v| v["components"][0]["kind"] = json!("router")),
        ),
        (
            "kind in the wrong case",
            Box::new(|v| v["components"][0]["kind"] = json!("credential_store")),
        ),
        (
            "unknown component field",
            Box::new(|v| v["components"][0]["owner"] = json!("x")),
        ),
        (
            "controller as a string",
            Box::new(|v| v["components"][0]["controller"] = json!("CTRL-001")),
        ),
        (
            "controller entry not a string",
            Box::new(|v| v["components"][0]["controller"] = json!([1])),
        ),
        (
            "credentials null",
            Box::new(|v| v["components"][1]["credentials"] = Value::Null),
        ),
        (
            "unknown credential kind",
            Box::new(|v| v["components"][1]["credentials"][0]["kind"] = json!("ssh-key")),
        ),
        (
            "credential without name",
            Box::new(|v| {
                v["components"][1]["credentials"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("name");
            }),
        ),
        (
            "id with capitals",
            Box::new(|v| v["components"][1]["id"] = json!("Bad_Id")),
        ),
        ("empty id", Box::new(|v| v["zones"][0]["id"] = json!(""))),
        (
            "id of 65 chars",
            Box::new(|v| v["connections"][0]["id"] = json!("a".repeat(65))),
        ),
        (
            "unknown protocol",
            Box::new(|v| v["connections"][0]["protocol"] = json!("ftp")),
        ),
        (
            "port above u16",
            Box::new(|v| v["connections"][0]["port"] = json!(65536)),
        ),
        (
            "negative port",
            Box::new(|v| v["connections"][0]["port"] = json!(-1)),
        ),
        (
            "fractional port",
            Box::new(|v| v["connections"][0]["port"] = json!(80.5)),
        ),
        (
            "authenticated missing",
            Box::new(|v| {
                v["connections"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("authenticated");
            }),
        ),
        (
            "encrypted as a string",
            Box::new(|v| v["connections"][0]["encrypted"] = json!("yes")),
        ),
        ("zones not a list", Box::new(|v| v["zones"] = json!({}))),
    ];
    for (name, f) in &cases {
        let v = mutated(f.as_ref());
        assert!(!schema_ok(&v), "schema accepted: {name}");
        assert!(!check_ok(&v), "check accepted: {name}");
    }
}

/// The rules the schema's description names as beyond JSON Schema: the
/// schema accepts each of these, `check()` refuses each.
// rivet: verifies REQ-ARCHVIEW-002
#[test]
fn only_check_refuses_the_listed_cross_reference_rules() {
    let cases: Vec<Mutation> = vec![
        (
            "(1) duplicate id",
            Box::new(|v| v["components"][1]["id"] = json!("c-server")),
        ),
        (
            "(2) zone names no zone",
            Box::new(|v| v["components"][1]["zone"] = json!("z-nowhere")),
        ),
        (
            "(3) host on a host",
            Box::new(|v| v["components"][0]["host"] = json!("c-ci-host")),
        ),
        (
            "(3) host is not a host",
            Box::new(|v| v["components"][1]["host"] = json!("c-forge")),
        ),
        (
            "(4) agent off a session",
            Box::new(|v| v["components"][1]["agent"] = json!("someone")),
        ),
        (
            "(5) credential declared twice",
            Box::new(|v| v["components"][1]["credentials"][1]["name"] = json!("app-key")),
        ),
        (
            "(6) to names no component",
            Box::new(|v| v["connections"][0]["to"] = json!("c-missing")),
        ),
        (
            "(6) no such credential",
            Box::new(|v| v["connections"][0]["credential"] = json!("c-agent/no-such")),
        ),
        (
            "(7) zone_crossing disagrees",
            Box::new(|v| v["connections"][0]["zone_crossing"] = json!(false)),
        ),
        (
            "(8) a secret-looking value",
            Box::new(|v| v["components"][1]["credentials"][0]["purpose"] = json!("ghp_planted")),
        ),
    ];
    let desc: Value = serde_json::from_str(SCHEMA).unwrap();
    let desc = desc["description"].as_str().unwrap();
    for n in 1..=8 {
        assert!(desc.contains(&format!("({n})")), "rule ({n}) not listed");
    }
    for (name, f) in &cases {
        let v = mutated(f.as_ref());
        assert!(schema_ok(&v), "schema refused: {name}");
        assert!(!check_ok(&v), "check accepted: {name}");
    }
}

/// Each enum the schema publishes is exactly the set serde accepts. The
/// matches are exhaustive, so a new variant does not compile until it is
/// listed here, and then this fails until the schema has it too.
// rivet: verifies REQ-ARCHVIEW-002
#[test]
fn schema_enums_equal_the_rust_enums() {
    use std::collections::BTreeSet;
    use warte::architecture::{CredentialKind as C, Kind as K, Protocol as P, Trust as T};
    fn names<X: serde::Serialize>(xs: &[X]) -> BTreeSet<String> {
        xs.iter()
            .map(|x| {
                serde_json::to_value(x)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }
    let trust = [T::Trusted, T::Internal, T::Dmz, T::External];
    for t in &trust {
        match t {
            T::Trusted | T::Internal | T::Dmz | T::External => {}
        }
    }
    let kind = [
        K::Host,
        K::Service,
        K::Session,
        K::CredentialStore,
        K::Device,
        K::External,
    ];
    for k in &kind {
        match k {
            K::Host | K::Service | K::Session | K::CredentialStore | K::Device | K::External => {}
        }
    }
    let cred = [
        C::PrivateKey,
        C::Token,
        C::Password,
        C::ApiKey,
        C::Certificate,
        C::Seed,
        C::Other,
    ];
    for c in &cred {
        match c {
            C::PrivateKey
            | C::Token
            | C::Password
            | C::ApiKey
            | C::Certificate
            | C::Seed
            | C::Other => {}
        }
    }
    let proto = [
        P::Quic,
        P::Https,
        P::Ssh,
        P::Udp,
        P::Wireguard,
        P::Unix,
        P::Other,
    ];
    for p in &proto {
        match p {
            P::Quic | P::Https | P::Ssh | P::Udp | P::Wireguard | P::Unix | P::Other => {}
        }
    }
    let s: Value = serde_json::from_str(SCHEMA).unwrap();
    let published = |p: &str| -> BTreeSet<String> {
        s.pointer(p)
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        published("/$defs/zone/properties/trust/enum"),
        names(&trust)
    );
    assert_eq!(
        published("/$defs/component/properties/kind/enum"),
        names(&kind)
    );
    assert_eq!(
        published("/$defs/credential/properties/kind/enum"),
        names(&cred)
    );
    assert_eq!(
        published("/$defs/connection/properties/protocol/enum"),
        names(&proto)
    );
}
