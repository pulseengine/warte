# Re-queue after a merge-queue ejection by an unacquired hosted job (#122)

Requirements: REQ-QUEUE-003 (classify and count), REQ-QUEUE-004 (bounded
re-queue offer). Owner decision 2026-11-25: warte tells the PR's agent, and the
agent re-queues with its own broker token. warte stays read-only on GitHub.

## What warte already has

- Every finished job reaches warte as a `JobRecord`, from the fleet agent
  (`[hub]`) or from warte's own GitHub ledger (`[github]`). It carries
  `ev` (event), `br` (head branch), `runner` (empty when no runner took it),
  `r` (conclusion), `pool` and `sig` (an error signature from fixed patterns).
- A merge-group run's head branch names its PR: `gh-readonly-queue/<base>/pr-<N>-<sha>`.
- Nothing in a `JobRecord` says whether a step ran or what the annotations
  said. Those are the two conditions REQ-QUEUE-003 adds to "cancelled, no runner".

## Plan

### Phase 1: classify (REQ-QUEUE-003)

1. The ledger source sets `sig = "infra:not-acquired"` when a job is
   `cancelled`, has no `runner_name`, ran no step, and one of its check-run
   annotations contains `not acquired by Runner`. The annotation read is one
   GET per candidate job (cancelled and no runner), which is rare.
   - `[github]` mode: warte's ledger does it (`ghsource::ledger::record`).
   - `[hub]` mode: the fleet agent does it. **Seam A** (provider: the fleet
     agent, consumer: warte). Until the agent ships it, warte shows such jobs as
     "cancelled, no runner", never as `infra:not-acquired`, and offers nothing.
2. The queue view and `/api/queue` show the class apart from code failures.
3. A count per hosted pool and UTC day in the history store, shown on the
   runners page.

Tests: a fixture job for each of the four conditions failing alone gives no
class; all four give the class; the count groups by pool and day.

### Phase 2: offer the re-queue (REQ-QUEUE-004)

1. An ejection is a completed `merge_group` run whose conclusion is not
   success. It qualifies only when **every** job in it that did not succeed
   carries `infra:not-acquired`.
2. warte keeps per PR and UTC day: attempts (at most 3), and the earliest next
   time (30, 60, then 120 minutes after the ejection).
3. **Seam B** (provider: warte, consumer: the agents' CI skill in
   pulseengine-claude): `GET /api/requeue?repo=<repo>` returns the open offers,
   `[{pr, attempt, of: 3, earliest, pool}]`. The repo's agent reads it where it
   already waits on CI, and re-enqueues the PR with its own broker token once
   `earliest` has passed. Every required check still has to pass in the queue.
4. A factory-notice per offer ("relay#636 ejected by hosted macOS arm64
   capacity, re-queue 1/3 after 16:15 UTC"), and an escalation notice when the
   third attempt is ejected too.
5. Kill criterion: a PR re-queued after an offer and then ejected for any other
   reason stops offers for that repo until a person clears it on the page.

Tests: an ejection with one ordinary failure gives no offer; the backoff and the
daily cap; the kill switch trips and holds; the offer endpoint reveals only repo,
PR number, attempt and time.

## Seams to open before building

- **A**, with the fleet agent's owner: the `infra:not-acquired` signature.
- **B**, with pulseengine.eu (the CI skill): the offer endpoint and who
  enqueues.

Phase 1 in `[github]` mode needs neither seam and can ship first.
