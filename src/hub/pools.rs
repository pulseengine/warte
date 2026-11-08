//! Runner pools: what kind of runner serves a pool, on which OS, under which
//! name, and the inventory the config gives where GitHub will not list
//! runners (REQ-RUNNERS-001, REQ-RUNNERS-002, DD-RUNNERS-001).
//!
//! A pool is named from a job's labels ([`super::parse::pool_of`]):
//! self-hosted pools by their specific labels (`rust-cpu`, `light+rust-cpu`),
//! GitHub-hosted ones as `hosted:<runs-on label>` (`hosted:ubuntu-latest`).
//! Records from before the split carry the bare pool `hosted`; it stays
//! readable as "GitHub-hosted (not split)".

use serde::{Deserialize, Serialize};

/// The pool name of GitHub-hosted jobs before the split, and the prefix of
/// the split ones.
pub const HOSTED: &str = "hosted";

/// Is this a GitHub-hosted pool (split or not)?
pub fn is_hosted(pool: &str) -> bool {
    pool == HOSTED || pool.starts_with("hosted:")
}

/// The pool for GitHub-hosted runs-on labels: `hosted:<labels>`, lowercase,
/// sorted, joined by `+`.
pub fn hosted_pool(labels: &[String]) -> String {
    let mut l: Vec<String> = labels
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    l.sort();
    l.dedup();
    if l.is_empty() {
        HOSTED.to_string()
    } else {
        format!("hosted:{}", l.join("+"))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A runner image GitHub publishes as standard (`ubuntu-latest`,
    /// `macos-15`, `windows-2022`, ...).
    StandardHosted,
    /// Any other GitHub-hosted label: larger runners, billed per size.
    LargerHosted,
    SelfHosted,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Linux,
    Macos,
    Windows,
    Unknown,
}

/// The labels GitHub publishes for its standard hosted runners. Anything
/// else without `self-hosted` is a larger runner (or a custom image).
const STANDARD: [&str; 19] = [
    "ubuntu-latest",
    "ubuntu-24.04",
    "ubuntu-22.04",
    "ubuntu-20.04",
    "ubuntu-24.04-arm",
    "ubuntu-22.04-arm",
    "ubuntu-slim",
    "macos-latest",
    "macos-26",
    "macos-15",
    "macos-14",
    "macos-13",
    "macos-15-intel",
    "windows-latest",
    "windows-2025",
    "windows-2022",
    "windows-2019",
    "windows-11-arm",
    "windows-11",
];

/// One `[[runners.pool]]` entry: names, classifies, sizes and prices a pool.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PoolConfig {
    /// The runs-on labels that select the pool, as a workflow writes them
    /// (`["linux-16core"]`, `["self-hosted", "build"]`).
    pub labels: Vec<String>,
    /// A readable name ("Linux, 16 cores").
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<Kind>,
    #[serde(default)]
    pub os: Option<Os>,
    /// How many runners serve the pool, when GitHub will not list them.
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub cores: Option<u32>,
    #[serde(default)]
    pub memory_gb: Option<u32>,
    /// List price per runner minute, for the notional cost (REQ-RUNNERS-004).
    #[serde(default)]
    pub per_minute: Option<f64>,
}

/// `[runners]` in warte's config.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunnersConfig {
    /// The currency `per_minute` is in, shown next to the cost ("USD").
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub pool: Vec<PoolConfig>,
}

impl RunnersConfig {
    /// The entry for a pool name, matched by the pool its labels make.
    pub fn entry(&self, pool: &str, self_hosted: &[String]) -> Option<&PoolConfig> {
        self.pool
            .iter()
            .find(|p| super::parse::pool_with(&p.labels, self_hosted) == pool)
    }
}

/// What the dashboard and the API say about a pool.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PoolInfo {
    pub pool: String,
    pub name: String,
    pub kind: Kind,
    pub os: Os,
}

fn os_of(label: &str) -> Os {
    let l = label.to_ascii_lowercase();
    if l.contains("macos") || l.contains("mac-") {
        Os::Macos
    } else if l.contains("windows") || l.contains("win-") {
        Os::Windows
    } else if l.contains("ubuntu") || l.contains("linux") {
        Os::Linux
    } else {
        Os::Unknown
    }
}

/// Kind, OS and name of a pool; the config's entry overrides each.
pub fn info(pool: &str, cfg: &RunnersConfig, self_hosted: &[String]) -> PoolInfo {
    let (kind, os, name) = if pool == HOSTED {
        (
            Kind::StandardHosted,
            Os::Unknown,
            "GitHub-hosted (not split)".to_string(),
        )
    } else if let Some(labels) = pool.strip_prefix("hosted:") {
        let parts: Vec<&str> = labels.split('+').collect();
        let standard = parts.iter().all(|l| STANDARD.contains(l));
        let os = parts
            .iter()
            .map(|l| os_of(l))
            .find(|o| *o != Os::Unknown)
            .unwrap_or(Os::Unknown);
        (
            if standard {
                Kind::StandardHosted
            } else {
                Kind::LargerHosted
            },
            os,
            labels.to_string(),
        )
    } else {
        let os = pool
            .split('+')
            .map(os_of)
            .find(|o| *o != Os::Unknown)
            .unwrap_or(Os::Linux);
        (Kind::SelfHosted, os, pool.to_string())
    };
    let e = cfg.entry(pool, self_hosted);
    PoolInfo {
        pool: pool.to_string(),
        name: e.and_then(|e| e.name.clone()).unwrap_or(name),
        kind: e.and_then(|e| e.kind).unwrap_or(kind),
        os: e.and_then(|e| e.os).unwrap_or(os),
    }
}

/// The runners to show when GitHub would not list them (REQ-RUNNERS-002):
/// for each configured pool with a `count`, the busy runners seen from running
/// jobs plus idle ones up to the count, marked `config`; busy runners of
/// pools the config does not describe stay, marked `inferred`. When GitHub
/// listed the runners (`listed`), its list wins and is returned unchanged.
pub fn fill(
    runners: Vec<super::model::Runner>,
    listed: bool,
    cfg: &RunnersConfig,
    self_hosted: &[String],
) -> Vec<super::model::Runner> {
    use super::model::Runner;
    if listed {
        return runners;
    }
    let pool_of = |r: &Runner| super::parse::pool_with(&r.labels, self_hosted);
    let mut out: Vec<Runner> = Vec::new();
    let mut used = vec![false; runners.len()];
    for e in cfg.pool.iter().filter(|e| e.count.is_some()) {
        let pool = super::parse::pool_with(&e.labels, self_hosted);
        if is_hosted(&pool) {
            continue; // GitHub runs hosted pools; there are no runners to list
        }
        let count = e.count.unwrap_or(0) as usize;
        let mut n = 0;
        for (i, r) in runners.iter().enumerate() {
            if !used[i] && pool_of(r) == pool {
                used[i] = true;
                n += 1;
                out.push(Runner {
                    source: Some("config".into()),
                    ..r.clone()
                });
            }
        }
        let name = e.name.clone().unwrap_or_else(|| pool.clone());
        for k in n..count {
            out.push(Runner {
                unit: format!("{name} #{}", k + 1),
                active: true,
                local_busy: false,
                gh_busy: Some(false),
                gh_status: Some("online".into()),
                labels: e.labels.clone(),
                source: Some("config".into()),
                ..Default::default()
            });
        }
    }
    for (i, r) in runners.into_iter().enumerate() {
        if !used[i] {
            out.push(Runner {
                source: Some("inferred".into()),
                ..r
            });
        }
    }
    for (i, r) in out.iter_mut().enumerate() {
        r.id = i as u32 + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // rivet: verifies REQ-RUNNERS-001
    #[test]
    fn hosted_pools_have_a_kind_and_an_os_and_the_config_overrides_them() {
        let none = RunnersConfig::default();
        let i = info("hosted:ubuntu-latest", &none, &[]);
        assert_eq!((i.kind, i.os), (Kind::StandardHosted, Os::Linux));
        let i = info("hosted:macos-15", &none, &[]);
        assert_eq!((i.kind, i.os), (Kind::StandardHosted, Os::Macos));
        let i = info("hosted:windows-2022", &none, &[]);
        assert_eq!((i.kind, i.os), (Kind::StandardHosted, Os::Windows));
        // anything GitHub does not publish as standard is a larger runner
        let i = info("hosted:linux-16core", &none, &[]);
        assert_eq!((i.kind, i.os), (Kind::LargerHosted, Os::Linux));
        let i = info("hosted:macos-15-xlarge", &none, &[]);
        assert_eq!((i.kind, i.os), (Kind::LargerHosted, Os::Macos));
        let i = info("hosted:big-box", &none, &[]);
        assert_eq!((i.kind, i.os), (Kind::LargerHosted, Os::Unknown));
        // self-hosted pools
        let i = info("rust-cpu", &none, &[]);
        assert_eq!((i.kind, i.os), (Kind::SelfHosted, Os::Linux));
        // records from before the split stay readable
        assert!(is_hosted("hosted") && is_hosted("hosted:x") && !is_hosted("hostedx"));
        assert_eq!(info("hosted", &none, &[]).name, "GitHub-hosted (not split)");

        let cfg: RunnersConfig = toml::from_str(
            r#"
            currency = "USD"
            [[pool]]
            labels = ["big-box"]
            name = "Build box, 32 cores"
            kind = "larger-hosted"
            os = "linux"
            cores = 32
            per_minute = 0.128
            [[pool]]
            labels = ["self-hosted", "mac-studio"]
            os = "macos"
            count = 2
            "#,
        )
        .unwrap();
        let i = info("hosted:big-box", &cfg, &[]);
        assert_eq!(i.name, "Build box, 32 cores");
        assert_eq!(i.os, Os::Linux);
        let i = info("mac-studio", &cfg, &[]);
        assert_eq!((i.kind, i.os), (Kind::SelfHosted, Os::Macos));
        assert_eq!(cfg.entry("mac-studio", &[]).unwrap().count, Some(2));
        // a typo in the config is refused, not ignored
        assert!(toml::from_str::<RunnersConfig>("[[pool]]\nlabels=[\"x\"]\ncount_=1").is_err());
        assert_eq!(
            hosted_pool(&l(&["Ubuntu-Latest", " "])),
            "hosted:ubuntu-latest"
        );
        assert_eq!(hosted_pool(&l(&[])), "hosted");
    }

    // rivet: verifies REQ-RUNNERS-002
    #[test]
    fn the_config_inventory_fills_what_github_will_not_list() {
        use crate::hub::model::Runner;
        let busy = |unit: &str, labels: &[&str]| Runner {
            unit: unit.into(),
            active: true,
            local_busy: true,
            gh_busy: Some(true),
            labels: l(labels),
            ..Default::default()
        };
        let cfg: RunnersConfig = toml::from_str(
            r#"
            [[pool]]
            labels = ["self-hosted", "build"]
            name = "Build"
            count = 3
            [[pool]]
            labels = ["linux-16core"]
            count = 9
            "#,
        )
        .unwrap();
        let seen = vec![
            busy("b-7", &["self-hosted", "build"]),
            busy("x-1", &["self-hosted", "other"]),
        ];
        // GitHub refused the list: 1 busy + 2 idle Build, the other inferred
        let r = fill(seen.clone(), false, &cfg, &[]);
        let build: Vec<_> = r
            .iter()
            .filter(|r| r.labels.contains(&"build".to_string()))
            .collect();
        assert_eq!(build.len(), 3);
        assert_eq!(build.iter().filter(|r| r.local_busy).count(), 1);
        assert!(build.iter().all(|r| r.source.as_deref() == Some("config")));
        assert!(build.iter().any(|r| r.unit == "Build #2"));
        let other = r.iter().find(|r| r.unit == "x-1").unwrap();
        assert_eq!(other.source.as_deref(), Some("inferred"));
        // a hosted larger runner pool lists no runners
        assert_eq!(r.len(), 4);
        let ids: Vec<u32> = r.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
        // GitHub listed them: its list wins, unchanged
        let api = fill(seen.clone(), true, &cfg, &[]);
        assert_eq!(api.len(), 2);
        assert!(api.iter().all(|r| r.source.is_none()));
        // no inventory configured: what was seen, marked inferred
        let r = fill(seen, false, &RunnersConfig::default(), &[]);
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|r| r.source.as_deref() == Some("inferred")));
    }
}
