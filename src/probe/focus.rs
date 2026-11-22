//! `warte probe focus` (REQ-FOCUS-001, warte#180): keep the attention
//! schedule's `available` in step with the night Focus's schedule on macOS.
//!
//! Reads `~/Library/DoNotDisturb/DB/ModeConfigurations.json` (field names
//! confirmed on macOS 27, 2026-11-22: `data[0].modeConfigurations.<id>`,
//! `mode.name` (localized), `triggers.triggers[]` with `class`,
//! `enabledSetting` (2 = on), `timePeriodStart/EndTimeHour/Minute` and the
//! `timePeriodWeekdays` bitmask). The day outside the night window becomes
//! one `available` span. Only `available` is written; `off` never is.
//!
//! Fails safe: an unreadable file, an unknown shape, no enabled schedule, a
//! sleep trigger without times, or a per-day bitmask (its bit order is not
//! confirmed yet) is an error, and nothing is written.

use serde_json::value::RawValue;
use serde_json::Value;
use std::collections::BTreeMap;

/// The night Focus by default: the Sleep Focus's stable identifier (its name
/// is localized, e.g. "Schlafen").
pub const DEFAULT_MODE: &str = "com.apple.sleep.sleep-mode";
/// Every day of the week in `timePeriodWeekdays`.
pub const EVERY_DAY: u64 = 127;
const SCHEDULE: &str = "DNDModeConfigurationScheduleTrigger";

/// Where macOS keeps the Focus modes, under the user's home.
pub fn default_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(|h| std::path::Path::new(&h).join("Library/DoNotDisturb/DB/ModeConfigurations.json"))
}

/// A daily night window, start and end as (hour, minute).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Night {
    pub start: (u8, u8),
    pub end: (u8, u8),
}

fn hhmm((h, m): (u8, u8)) -> String {
    format!("{h:02}:{m:02}")
}

impl Night {
    /// The `available` span: the day outside the night, from the night's end
    /// to its start (it crosses midnight when the night does not).
    pub fn available(&self) -> Value {
        serde_json::json!([{"from": hhmm(self.end), "to": hhmm(self.start)}])
    }
}

fn time(t: &Value, h: &str, m: &str) -> Option<(u8, u8)> {
    let hour = u8::try_from(t[h].as_u64()?).ok().filter(|x| *x < 24)?;
    let minute = u8::try_from(t[m].as_u64()?).ok().filter(|x| *x < 60)?;
    Some((hour, minute))
}

/// The night window of `mode` (its identifier or its name, any case).
pub fn night(json: &str, mode: &str) -> Result<Night, String> {
    let v: Value =
        serde_json::from_str(json).map_err(|_| "the Focus file is not JSON".to_string())?;
    let modes = v["data"][0]["modeConfigurations"]
        .as_object()
        .ok_or("the Focus file's format changed: no data[0].modeConfigurations")?;
    let (_, cfg) = modes
        .iter()
        .find(|(id, c)| {
            id.eq_ignore_ascii_case(mode)
                || c["mode"]["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(mode))
        })
        .ok_or_else(|| format!("no Focus mode {mode:?}"))?;
    let triggers = cfg["triggers"]["triggers"]
        .as_array()
        .ok_or("the Focus file's format changed: no triggers")?;
    let enabled: Vec<&Value> = triggers
        .iter()
        .filter(|t| t["enabledSetting"].as_u64() == Some(2))
        .collect();
    if enabled.is_empty() {
        return Err(format!("the Focus {mode:?} has no enabled schedule"));
    }
    let schedules: Vec<&&Value> = enabled.iter().filter(|t| t["class"] == SCHEDULE).collect();
    let [t] = schedules.as_slice() else {
        return Err(if schedules.is_empty() {
            format!("the Focus {mode:?} is not on a time schedule (a sleep schedule from Health has no times here)")
        } else {
            format!("the Focus {mode:?} has several schedules; one is supported")
        });
    };
    if t["timePeriodWeekdays"].as_u64() != Some(EVERY_DAY) {
        return Err(
            "a schedule for some days only is not supported yet: its day order is unconfirmed"
                .into(),
        );
    }
    let start = time(t, "timePeriodStartTimeHour", "timePeriodStartTimeMinute")
        .ok_or("the Focus file's format changed: no start time")?;
    let end = time(t, "timePeriodEndTimeHour", "timePeriodEndTimeMinute")
        .ok_or("the Focus file's format changed: no end time")?;
    if start == end {
        return Err("the night window is empty".into());
    }
    Ok(Night { start, end })
}

/// The attention schedule with only `available` replaced; every other key's
/// value is kept byte for byte. Refuses a schedule that is not a JSON object.
pub fn with_available(attention: &str, night: &Night) -> Result<String, String> {
    let mut doc: BTreeMap<String, Box<RawValue>> = serde_json::from_str(attention)
        .map_err(|_| "the attention schedule is not a JSON object".to_string())?;
    let avail = RawValue::from_string(night.available().to_string()).map_err(|e| e.to_string())?;
    doc.insert("available".into(), avail);
    serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())
}

/// `warte probe focus`: print the derived span, or write it into the
/// attention file. Any doubt writes nothing and returns the reason.
pub fn run(
    focus_file: &std::path::Path,
    mode: &str,
    write: Option<&std::path::Path>,
) -> Result<String, String> {
    let text = std::fs::read_to_string(focus_file).map_err(|e| {
        format!(
            "cannot read {} ({e}); the helper needs Full Disk Access",
            focus_file.display()
        )
    })?;
    let n = night(&text, mode)?;
    let span = n.available().to_string();
    let Some(path) = write else {
        return Ok(format!("available: {span}"));
    };
    let current = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".into());
    let next = with_available(&current, &n)?;
    let tmp = path.with_extension("json.focus-tmp");
    std::fs::write(&tmp, &next).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format!("available: {span} written to {}", path.display()))
}
