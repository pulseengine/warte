//! One person's attention schedule (REQ-ATTN-001, warte#181): the shape of
//! pulseengine.eu's `attention.schema.json`, schema_version 1, stored per
//! warte login and served at `GET /api/attention`.
//!
//! [`validate`] applies the schema and the rules a schema cannot express
//! (the notice client enforces the same): HH:MM times, `from` differs from
//! `to`, `max_asking_flows` 1 to 100, a known timezone, `off.to` after
//! `off.from`. A span written as the string `HH:MM-HH:MM` becomes
//! `{from, to}`. An error names the field, never its value.

use serde_json::{Map, Value};
use sqlx::SqlitePool;

pub const SCHEMA_VERSION: u64 = 1;
pub const MAX_FLOWS: u64 = 100;
const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const KEYS: [&str; 9] = [
    "schema_version",
    "updated_at",
    "max_asking_flows",
    "windows",
    "available",
    "focus",
    "off",
    "timezone",
    "always_open_kinds",
];
/// The system's time zone database; a timezone must name a file in it.
const ZONEINFO: [&str; 2] = ["/usr/share/zoneinfo", "/var/db/timezone/zoneinfo"];

fn hhmm(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && s[..2].parse::<u8>().is_ok_and(|h| h < 24)
        && s[3..].parse::<u8>().is_ok_and(|m| m < 60)
        && b[..2].iter().chain(&b[3..]).all(u8::is_ascii_digit)
}

fn span(path: &str, v: &Value) -> Result<Value, String> {
    let obj = match v {
        Value::String(s) => {
            let (f, t) = s
                .split_once('-')
                .ok_or_else(|| format!("{path}: not HH:MM-HH:MM"))?;
            let mut m = Map::new();
            m.insert("from".into(), Value::String(f.into()));
            m.insert("to".into(), Value::String(t.into()));
            m
        }
        Value::Object(m) => m.clone(),
        _ => return Err(format!("{path}: not a span")),
    };
    for k in obj.keys() {
        if !["from", "to", "days"].contains(&k.as_str()) {
            return Err(format!("{path}.{k}: unknown field"));
        }
    }
    let get = |k: &str| obj.get(k).and_then(Value::as_str).unwrap_or("");
    for k in ["from", "to"] {
        if !hhmm(get(k)) {
            return Err(format!("{path}.{k}: not HH:MM"));
        }
    }
    if get("from") == get("to") {
        return Err(format!("{path}: from and to are the same"));
    }
    if let Some(d) = obj.get("days") {
        let d = d
            .as_array()
            .filter(|a| !a.is_empty())
            .ok_or_else(|| format!("{path}.days: not a list of days"))?;
        if !d
            .iter()
            .all(|x| x.as_str().is_some_and(|s| DAYS.contains(&s)))
        {
            return Err(format!("{path}.days: a day is not mon..sun"));
        }
    }
    Ok(Value::Object(obj))
}

fn spans(doc: &Map<String, Value>, key: &str, min: usize) -> Result<Option<Value>, String> {
    let Some(v) = doc.get(key) else {
        return Ok(None);
    };
    let a = v.as_array().ok_or_else(|| format!("{key}: not a list"))?;
    if a.len() < min {
        return Err(format!("{key}: needs at least {min}"));
    }
    a.iter()
        .enumerate()
        .map(|(i, s)| span(&format!("{key}[{i}]"), s))
        .collect::<Result<Vec<_>, _>>()
        .map(|v| Some(Value::Array(v)))
}

/// An ISO 8601 date-time or date as a UTC instant; without an offset it is
/// read as UTC, which is enough to order `from` and `to`.
fn instant(s: &str) -> Option<i64> {
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(d.timestamp());
    }
    for f in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(d) = chrono::NaiveDateTime::parse_from_str(s, f) {
            return Some(d.and_utc().timestamp());
        }
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|d| d.and_utc().timestamp())
}

/// Is `tz` a zone of the system's database (never a path)?
pub fn known_timezone(tz: &str) -> bool {
    if tz == "UTC" {
        return true;
    }
    let shape = !tz.is_empty()
        && tz.len() <= 64
        && !tz.starts_with('/')
        && !tz.contains("..")
        && tz
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_+-/".contains(c));
    shape
        && ZONEINFO
            .iter()
            .any(|d| std::path::Path::new(d).join(tz).is_file())
}

/// The schedule as it will be stored and served, or the first broken field.
/// `updated_at` is set by the caller.
pub fn validate(v: &Value) -> Result<Value, String> {
    let doc = v.as_object().ok_or("the schedule is not an object")?;
    for k in doc.keys() {
        if !KEYS.contains(&k.as_str()) {
            return Err(format!("{k}: unknown field"));
        }
    }
    let mut out = Map::new();
    if let Some(s) = doc.get("schema_version") {
        if s.as_u64() != Some(SCHEMA_VERSION) {
            return Err("schema_version: must be 1".into());
        }
    }
    out.insert("schema_version".into(), Value::from(SCHEMA_VERSION));
    if let Some(m) = doc.get("max_asking_flows") {
        let n = m
            .as_u64()
            .filter(|n| (1..=MAX_FLOWS).contains(n))
            .ok_or_else(|| format!("max_asking_flows: must be 1 to {MAX_FLOWS}"))?;
        out.insert("max_asking_flows".into(), Value::from(n));
    }
    for (key, min) in [("windows", 1), ("available", 0), ("focus", 0)] {
        if let Some(s) = spans(doc, key, min)? {
            out.insert(key.into(), s);
        }
    }
    if let Some(o) = doc.get("off") {
        let a = o.as_array().ok_or("off: not a list")?;
        let mut v = Vec::new();
        for (i, p) in a.iter().enumerate() {
            let path = format!("off[{i}]");
            let m = p
                .as_object()
                .ok_or_else(|| format!("{path}: not a period"))?;
            for k in m.keys() {
                if !["from", "to", "note"].contains(&k.as_str()) {
                    return Err(format!("{path}.{k}: unknown field"));
                }
            }
            let at = |k: &str| {
                m.get(k)
                    .and_then(Value::as_str)
                    .filter(|s| (10..=40).contains(&s.len()))
                    .and_then(instant)
                    .ok_or_else(|| format!("{path}.{k}: not an ISO 8601 date-time"))
            };
            if at("to")? <= at("from")? {
                return Err(format!("{path}: to is not after from"));
            }
            if m.get("note")
                .is_some_and(|n| n.as_str().is_none_or(|s| s.len() > 500))
            {
                return Err(format!("{path}.note: at most 500 characters"));
            }
            v.push(p.clone());
        }
        out.insert("off".into(), Value::Array(v));
    }
    if let Some(t) = doc.get("timezone") {
        let t = t
            .as_str()
            .filter(|t| known_timezone(t))
            .ok_or("timezone: not a known time zone")?;
        out.insert("timezone".into(), Value::from(t));
    }
    if let Some(k) = doc.get("always_open_kinds") {
        let a = k.as_array().ok_or("always_open_kinds: not a list")?;
        if !a
            .iter()
            .all(|x| x.as_str().is_some_and(|s| (1..=40).contains(&s.len())))
        {
            return Err("always_open_kinds: each is 1 to 40 characters".into());
        }
        out.insert("always_open_kinds".into(), k.clone());
    }
    Ok(Value::Object(out))
}

/// The schedules, one per login subject, in warte's database.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, subject: &str) -> Result<Option<Value>, String> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT doc FROM attention_schedules WHERE subject = ?")
                .bind(subject)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| format!("database error: {e}"))?;
        Ok(row.and_then(|(d,)| serde_json::from_str(&d).ok()))
    }

    /// Validate and store `v` for `subject`; returns what is served.
    pub async fn put(&self, subject: &str, v: &Value) -> Result<Value, String> {
        let mut doc = validate(v)?;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        doc["updated_at"] = Value::from(now.clone());
        sqlx::query(
            "INSERT INTO attention_schedules (subject, doc, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(subject) DO UPDATE SET doc = excluded.doc, updated_at = excluded.updated_at",
        )
        .bind(subject)
        .bind(doc.to_string())
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("database error: {e}"))?;
        Ok(doc)
    }
}
