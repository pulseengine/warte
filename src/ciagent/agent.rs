//! Agent: collect a snapshot on the CI host and push it to the hub. Read-only:
//! it observes the fleet and the existing automation, and changes nothing.
use crate::ciagent::collect;
use crate::ciagent::github::Gh;
use crate::ciagent::model::{Queue, Snapshot};
use crate::ciagent::telemetry;

pub struct Opts {
    pub org: String,
    pub token_cmd: String,
    pub token_args: Vec<String>,
    pub runner_prefix: String,
    pub no_github: bool,
    pub queue_interval: i64,
    pub active_days: i64,
    pub max_runs: usize,
    /// `[ci_agent]`: where the runners and the automation live.
    pub cfg: crate::ciagent::CiAgentConfig,
}

pub struct LedgerOpts {
    pub dir: String,
    pub interval: u64,
    pub budget: u32,
    pub backfill_days: i64,
    pub per_frame: usize,
}

/// The ledger runs in its own thread: a pass can take minutes of API calls and
/// must not stretch the 60 s telemetry cadence.
fn ledger_thread(o: &Opts, l: &LedgerOpts, store: std::sync::Arc<crate::ciagent::ledger::Store>) {
    let (org, token_cmd, token_args, active_days) = (
        o.org.clone(),
        o.token_cmd.clone(),
        o.token_args.clone(),
        o.active_days,
    );
    let (interval, budget, backfill) = (l.interval, l.budget, l.backfill_days);
    std::thread::spawn(move || loop {
        let started = std::time::Instant::now();
        let now = collect::now();
        match Gh::new(&token_cmd, &token_args).and_then(|gh| {
            gh.active_repos(&org, active_days.max(backfill), now)
                .map(|r| (gh, r))
        }) {
            Ok((gh, repos)) => {
                let st =
                    crate::ciagent::ledger::pass(&gh, &store, &org, &repos, now, backfill, budget);
                eprintln!("ledger pass: {} runs, {} jobs, {} calls, {} skipped errors, {} logs read, {} signatures, days {:?}, re-sent {:?}{}", st.runs, st.jobs, st.calls, st.errors, st.logs, st.sigs, st.days, st.resent,
                    st.stopped.map(|s| format!(", stopped: {s}")).unwrap_or_default());
            }
            Err(e) => eprintln!("ledger pass: {e}"),
        }
        std::thread::sleep(
            std::time::Duration::from_secs(interval).saturating_sub(started.elapsed()),
        );
    });
}

pub fn snapshot(o: &Opts, queue_cache: &mut Option<Queue>) -> Snapshot {
    let mut errors = vec![];
    let machine = collect::machine(&o.cfg, &mut errors);
    let mut runners = collect::runners(&o.cfg, &mut errors);
    let automation = collect::automation(&o.cfg);
    let now = collect::now();
    if !o.no_github {
        match Gh::new(&o.token_cmd, &o.token_args) {
            Ok(gh) => {
                match gh.org_runners(&o.org) {
                    Ok(list) => {
                        for g in list {
                            let id = g
                                .name
                                .strip_prefix(&format!("{}-", o.runner_prefix))
                                .and_then(|n| n.parse::<u32>().ok());
                            if let Some(r) =
                                id.and_then(|id| runners.iter_mut().find(|r| r.id == id))
                            {
                                r.gh_status = Some(g.status);
                                r.gh_busy = Some(g.busy);
                                r.labels = g.labels;
                            }
                        }
                    }
                    Err(e) => errors.push(format!("github runners: {e}")),
                }
                let stale = queue_cache
                    .as_ref()
                    .map(|q| now - q.scanned_at >= o.queue_interval)
                    .unwrap_or(true);
                if stale {
                    match gh.queue(&o.org, o.active_days, o.max_runs, now) {
                        Ok(q) => *queue_cache = Some(q),
                        Err(e) => errors.push(format!("queue scan: {e}")),
                    }
                }
            }
            Err(e) => errors.push(format!("github auth: {e}")),
        }
    }
    Snapshot {
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        host: collect::hostname(),
        taken_at: now,
        machine,
        runners,
        queue: queue_cache.clone(),
        automation,
        errors,
    }
}

/// Every hub the agent feeds. Each packet is encoded ONCE and the
/// same bytes go to every hub, so all hubs see identical per-APID sequence
/// counters, which is what lets a parallel run compare them. One hub failing
/// never stops delivery to the others.
pub struct Hubs(Vec<(String, std::net::UdpSocket)>);

impl Hubs {
    pub fn connect(addrs: &[String]) -> Result<Hubs, String> {
        if addrs.is_empty() {
            return Err("at least one --hub is required".into());
        }
        let mut v = Vec::with_capacity(addrs.len());
        for a in addrs {
            let sock =
                std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("udp bind: {e}"))?;
            sock.connect(a)
                .map_err(|e| format!("udp connect {a}: {e}"))?;
            v.push((a.clone(), sock));
        }
        Ok(Hubs(v))
    }

    /// Send one packet to every hub. Returns how many hubs took it.
    pub fn send(&self, pkt: &[u8]) -> usize {
        let mut ok = 0;
        for (addr, sock) in &self.0 {
            match sock.send(pkt) {
                Ok(_) => ok += 1,
                Err(e) => eprintln!("udp send to {addr}: {e}"),
            }
        }
        ok
    }
}

/// Send one snapshot as a burst of CCSDS telemetry packets over UDP.
/// Fire-and-forget on purpose: per-APID sequence counters let the hub count
/// what was lost, so the agent never blocks on a slow or absent ground station.
pub fn send(hubs: &Hubs, s: &Snapshot, seq: &mut telemetry::SeqCounters) -> Result<usize, String> {
    let pkts = telemetry::encode_snapshot(s, seq);
    let mut delivered_anywhere = false;
    for p in &pkts {
        delivered_anywhere |= hubs.send(p) > 0;
    }
    if !pkts.is_empty() && !delivered_anywhere {
        return Err("udp send: no hub accepted any packet".into());
    }
    Ok(pkts.len())
}

pub fn run(
    o: &Opts,
    hubs: &[String],
    interval: u64,
    once: bool,
    ledger: Option<LedgerOpts>,
) -> Result<(), String> {
    let sock = Hubs::connect(hubs)?;
    let mut seq = telemetry::SeqCounters::default();
    let mut queue = None;
    let store = match &ledger {
        Some(l) => {
            let st = std::sync::Arc::new(crate::ciagent::ledger::Store::open(&l.dir)?);
            if !once && !o.no_github {
                ledger_thread(o, l, st.clone())
            }
            Some(st)
        }
        None => None,
    };
    loop {
        let started = std::time::Instant::now();
        // ledger first: records paced per frame, plus every day's manifest
        if let (Some(st), Some(l)) = (&store, &ledger) {
            let recs = st.take_outbox(l.per_frame);
            let pkts = telemetry::encode_ledger(&recs, &st.manifests(40), &mut seq);
            for p in &pkts {
                sock.send(p);
                // back-to-back datagrams are where losses cluster; a few
                // ms apart costs < 0.1 s per frame
                std::thread::sleep(std::time::Duration::from_millis(3));
            }
        }
        let s = snapshot(o, &mut queue);
        match send(&sock, &s, &mut seq) {
            Ok(n) => eprintln!(
                "sent {n} CCSDS packets ({} runners, {} errors)",
                s.runners.len(),
                s.errors.len()
            ),
            Err(e) => eprintln!("{e}"),
        }
        if once {
            return Ok(());
        }
        // fixed cadence: a slow tick (queue scan) must not stretch the period
        let period = std::time::Duration::from_secs(interval);
        std::thread::sleep(period.saturating_sub(started.elapsed()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;
    use std::time::Duration;

    fn listener() -> (UdpSocket, String) {
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let a = s.local_addr().unwrap().to_string();
        (s, a)
    }

    fn recv_all(s: &UdpSocket, n: usize) -> Vec<Vec<u8>> {
        let mut buf = [0u8; 2048];
        (0..n)
            .map(|_| {
                let k = s.recv(&mut buf).expect("packet");
                buf[..k].to_vec()
            })
            .collect()
    }

    #[test]
    fn every_hub_gets_the_same_bytes() {
        let (a, aa) = listener();
        let (b, ba) = listener();
        let hubs = Hubs::connect(&[aa, ba]).unwrap();
        let s = Snapshot {
            host: "ci-test".into(),
            ..Default::default()
        };
        let mut seq = telemetry::SeqCounters::default();
        let n = send(&hubs, &s, &mut seq).unwrap();
        assert!(n > 0);
        assert_eq!(
            recv_all(&a, n),
            recv_all(&b, n),
            "hubs must see identical packets and sequence counters"
        );
    }

    #[test]
    fn one_dead_hub_does_not_stop_the_other() {
        let (a, aa) = listener();
        // a port nobody listens on: sends may error (ICMP) but must not block delivery to `a`
        let dead = {
            let s = UdpSocket::bind("127.0.0.1:0").unwrap();
            s.local_addr().unwrap().to_string()
        };
        let hubs = Hubs::connect(&[dead, aa]).unwrap();
        let s = Snapshot {
            host: "ci-test".into(),
            ..Default::default()
        };
        let mut seq = telemetry::SeqCounters::default();
        for _ in 0..3 {
            let n = send(&hubs, &s, &mut seq).unwrap();
            assert_eq!(recv_all(&a, n).len(), n);
        }
    }

    #[test]
    fn no_hub_is_an_error() {
        assert!(Hubs::connect(&[]).is_err());
    }
}
