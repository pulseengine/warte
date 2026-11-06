//! The probe's own version and its certificate (REQ-PROBE-013): which
//! machines run an older probe, and which certificates will run out.
//!
//! Probe certificates last [`super::enroll::CERT_DAYS`] days and a probe
//! renews in the last third of that. A probe older than [`RENEWS_SINCE`]
//! cannot renew at all; it needs a newer build before its certificate ends.

/// The first probe version that renews its own certificate.
pub const RENEWS_SINCE: (u64, u64, u64) = (1, 5, 0);

/// A probe that can renew starts at 10 days left; by 7 it should have.
pub const NOT_RENEWED_AMBER_S: i64 = 7 * 86400;
pub const NOT_RENEWED_RED_S: i64 = 2 * 86400;
/// A probe that cannot renew needs a new build: warn early.
pub const CANNOT_RENEW_AMBER_S: i64 = 14 * 86400;
pub const CANNOT_RENEW_RED_S: i64 = 3 * 86400;

/// The version as reported, if it is a plain version string. The read API is
/// unauthenticated, so anything else is not shown or published.
pub fn clean(v: &str) -> Option<&str> {
    let ok = !v.is_empty()
        && v.len() <= 40
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    ok.then_some(v)
}

/// `major.minor.patch`, ignoring a `-pre` or `+build` suffix.
pub fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let core = clean(v)?.split(['-', '+']).next()?;
    let mut it = core.split('.');
    let n = |s: Option<&str>| s.filter(|s| !s.is_empty()).and_then(|s| s.parse().ok());
    let v = (n(it.next())?, n(it.next())?, n(it.next())?);
    it.next().is_none().then_some(v)
}

/// True when `probe` is an older release than `ground`.
pub fn behind(probe: &str, ground: &str) -> bool {
    matches!((parse(probe), parse(ground)), (Some(p), Some(g)) if p < g)
}

/// False only when the version is known and older than [`RENEWS_SINCE`].
pub fn can_renew(probe: &str) -> bool {
    parse(probe).is_none_or(|p| p >= RENEWS_SINCE)
}

/// "12 d" or "20 h": how long until `s` seconds have passed.
fn left(s: i64) -> String {
    if s >= 86400 {
        format!("{} d", s / 86400)
    } else {
        format!("{} h", (s / 3600).max(0))
    }
}

/// A finding for a certificate that will run out: (severity, text).
pub fn cert_finding(probe: &str, cert_until: i64, now: i64) -> Option<(&'static str, String)> {
    let rest = cert_until - now;
    if !can_renew(probe) {
        let sev = match rest {
            r if r <= CANNOT_RENEW_RED_S => "red",
            r if r <= CANNOT_RENEW_AMBER_S => "amber",
            _ => return None,
        };
        let (a, b, c) = RENEWS_SINCE;
        return Some((
            sev,
            format!(
                "probe {} cannot renew its certificate, which ends in {}; it needs {a}.{b}.{c} or newer",
                clean(probe).unwrap_or("?"),
                left(rest)
            ),
        ));
    }
    let sev = match rest {
        r if r <= NOT_RENEWED_RED_S => "red",
        r if r <= NOT_RENEWED_AMBER_S => "amber",
        _ => return None,
    };
    Some((
        sev,
        format!(
            "probe certificate ends in {} and has not been renewed",
            left(rest)
        ),
    ))
}
