//! Error signatures for failed jobs (warte REQ-CI-005, shared incidents).
//!
//! A failed job record carries `sig = "<kind>:<host>"`, taken from the end of
//! its log by fixed patterns only: a resolver failure, a connection timeout or
//! refusal, a server error, each for one DNS name. No log text leaves the CI
//! host. This is a port of `of_line` / `of_log` in pulseengine/warte
//! `src/incidents/mod.rs` at v1.6.21 (commit 89b2571), which parses what we send
//! with the same vocabulary and drops anything else.

/// How much of a job log's end is read for its signature (warte's `LOG_TAIL`).
pub const LOG_TAIL: usize = 256 * 1024;
/// At most this many job logs per ledger pass (warte's `MAX_LOGS_PER_PASS`).
pub const MAX_LOGS_PER_PASS: usize = 30;
/// Only failures this recent get a signature: warte groups the last day plus
/// its one-hour window, so an older log would cost a call and show nothing.
pub const RECENT_S: i64 = 86_400 + 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Resolver,
    Timeout,
    Refused,
    ServerError,
}

impl Kind {
    fn code(self) -> &'static str {
        match self {
            Kind::Resolver => "resolver",
            Kind::Timeout => "timeout",
            Kind::Refused => "refused",
            Kind::ServerError => "server_error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub kind: Kind,
    pub host: String,
}

impl Signature {
    /// The form a job record carries: `kind:host`.
    pub fn encode(&self) -> String {
        format!("{}:{}", self.kind.code(), self.host)
    }
}

/// Whether a conclusion gets a signature.
pub fn failed(conclusion: &str) -> bool {
    conclusion == "failure" || conclusion == "timed_out"
}

/// A DNS name: lowercase, with a dot, at most 253 characters, never an address
/// (IPv4 ends in an all-digit label, IPv6 holds colons).
fn host_name(s: &str) -> Option<String> {
    let s = s.trim_end_matches('.').to_ascii_lowercase();
    let ok = s.len() <= 253
        && s.contains('.')
        && s.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && !l.starts_with('-')
                && !l.ends_with('-')
        })
        && !s
            .rsplit('.')
            .next()
            .unwrap_or("")
            .chars()
            .all(|c| c.is_ascii_digit());
    ok.then_some(s)
}

/// The host right after `marker` in `line`, up to the first character a host cannot hold.
fn host_after(line: &str, marker: &str) -> Option<String> {
    let i = line.find(marker)? + marker.len();
    let rest = line[i..].trim_start_matches(['\'', '"', '`', ' ']);
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '.'))
        .unwrap_or(rest.len());
    host_name(&rest[..end])
}

/// The host of the first `http://` or `https://` URL in `line`.
fn url_host(line: &str) -> Option<String> {
    let i = line.find("://")?;
    if !line[..i].ends_with("http") && !line[..i].ends_with("https") {
        return None;
    }
    host_after(&line[i..], "://")
}

/// The signature one log line carries, if it matches a fixed pattern.
pub fn of_line(line: &str) -> Option<Signature> {
    let l = line.to_ascii_lowercase();
    let sig = |kind, host: Option<String>| host.map(|host| Signature { kind, host });
    for m in [
        "temporary failure resolving ",
        "could not resolve host: ",
        "could not resolve host ",
        "getaddrinfo enotfound ",
        "getaddrinfo eai_again ",
    ] {
        if let Some(s) = sig(Kind::Resolver, host_after(&l, m)) {
            return Some(s);
        }
    }
    if l.contains("no such host") || l.contains("name or service not known") {
        if let Some(s) = sig(
            Kind::Resolver,
            host_after(&l, "lookup ").or_else(|| url_host(&l)),
        ) {
            return Some(s);
        }
    }
    let target = || {
        host_after(&l, "failed to connect to ")
            .or_else(|| host_after(&l, "could not connect to "))
            .or_else(|| host_after(&l, "connect to "))
            .or_else(|| url_host(&l))
    };
    if l.contains("timed out") || l.contains("etimedout") {
        if let Some(s) = sig(Kind::Timeout, target()) {
            return Some(s);
        }
    }
    if l.contains("connection refused") || l.contains("econnrefused") {
        if let Some(s) = sig(Kind::Refused, target()) {
            return Some(s);
        }
    }
    let server_error = [
        "500 internal server error",
        "502 bad gateway",
        "503 service unavailable",
        "504 gateway timeout",
        "returned error: 500",
        "returned error: 502",
        "returned error: 503",
        "returned error: 504",
    ];
    if server_error.iter().any(|p| l.contains(p)) {
        return sig(Kind::ServerError, url_host(&l));
    }
    None
}

/// A failed job's signature from the end of its log: the last matching line,
/// the one closest to the failure. Nothing of the log is kept.
pub fn of_log(log: &str) -> Option<Signature> {
    log.lines().rev().find_map(of_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(line: &str) -> Option<String> {
        of_line(line).map(|s| s.encode())
    }

    #[test]
    fn resolver_lines_from_apt_curl_npm_go() {
        assert_eq!(
            enc("Err:1 http://x Temporary failure resolving 'Azure.Archive.Ubuntu.com'").as_deref(),
            Some("resolver:azure.archive.ubuntu.com")
        );
        assert_eq!(
            enc("curl: (6) Could not resolve host: static.rust-lang.org").as_deref(),
            Some("resolver:static.rust-lang.org")
        );
        assert_eq!(
            enc("npm ERR! getaddrinfo ENOTFOUND registry.npmjs.org").as_deref(),
            Some("resolver:registry.npmjs.org")
        );
        assert_eq!(
            enc("npm ERR! getaddrinfo EAI_AGAIN registry.npmjs.org").as_deref(),
            Some("resolver:registry.npmjs.org")
        );
        assert_eq!(
            enc("dial tcp: lookup proxy.golang.org on 192.0.2.53:53: no such host").as_deref(),
            Some("resolver:proxy.golang.org")
        );
    }

    #[test]
    fn timeout_refused_and_server_errors() {
        assert_eq!(enc("curl: (28) Failed to connect to crates.io port 443 after 130000 ms: Connection timed out").as_deref(), Some("timeout:crates.io"));
        assert_eq!(
            enc("curl: (7) Failed to connect to ghcr.io port 443: Connection refused").as_deref(),
            Some("refused:ghcr.io")
        );
        assert_eq!(enc("error: failed to download from `https://static.crates.io/x` 503 Service Unavailable").as_deref(), Some("server_error:static.crates.io"));
        assert_eq!(enc("curl: (22) The requested URL returned error: 502 for https://objects.githubusercontent.com/a").as_deref(), Some("server_error:objects.githubusercontent.com"));
    }

    #[test]
    fn addresses_and_plain_lines_carry_nothing() {
        assert_eq!(
            enc("Failed to connect to 192.0.2.5 port 443: Connection refused"),
            None
        );
        assert_eq!(
            enc("Could not resolve host: localhost"),
            None,
            "no dot, not a DNS name we name"
        );
        assert_eq!(enc("error[E0308]: mismatched types"), None);
        assert_eq!(enc("test result: FAILED. 3 passed; 1 failed"), None);
        assert_eq!(
            enc("ftp://files.example.org 503 Service Unavailable"),
            None,
            "only http(s) URLs"
        );
    }

    #[test]
    fn the_last_matching_line_wins() {
        let log = "Could not resolve host: github.com\nretrying\ncurl: (7) Failed to connect to ghcr.io port 443: Connection refused\nError: Process completed with exit code 7.\n";
        assert_eq!(
            of_log(log).map(|s| s.encode()).as_deref(),
            Some("refused:ghcr.io")
        );
        assert_eq!(of_log("all fine\n"), None);
    }

    #[test]
    fn only_failures_get_one() {
        assert!(failed("failure") && failed("timed_out"));
        assert!(!failed("success") && !failed("cancelled") && !failed("skipped"));
    }
}
