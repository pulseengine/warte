//! Realized queue waits per pool, measured by the agent's queue scan and sent
//! to the ground station inside the queue telemetry (`Queue::waits`).
//!
//! GitHub was measured not starting jobs in creation order (20 of 55 pairs
//! inverted, 2026-10-22, inside a single pool too), so "N ahead / throughput" is
//! confidently wrong. The ground station answers "when will PR X run?" from
//! these MEASURED waits instead; the estimator lives in warte's ground station.
use crate::ciagent::model::WaitStats;

/// p50/p90/max of realized waits (seconds). `None` when there are no samples.
pub fn wait_stats(v: &mut [i64]) -> Option<WaitStats> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    Some(WaitStats {
        n: v.len() as u32,
        p50: at(0.5),
        p90: at(0.9),
        max: v[v.len() - 1],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        let w = wait_stats(&mut [600, 60, 120, 300, 3600, 180, 240, 90, 30, 900]).unwrap();
        assert_eq!((w.n, w.p50, w.p90, w.max), (10, 240, 900, 3600));
        assert!(wait_stats(&mut []).is_none());
    }
}
