# Next release against the North Star (draft)

Work in progress, 2026-11-19. Nothing here is approved, built or tagged. The
scope is drafted in rivet as `release: v1.8.0` (`artifacts/north-star-plan.yaml`,
REQ-NS-001..004, all `draft`).

The yardstick is pulseengine.eu's `data/north-star.toml`: a safety-critical
product team can take a WebAssembly component from architecture model to signed
code running on a microcontroller, with evidence an assessor can re-check, using
open tools only. Measured today, from the published indicator files:

| Objective | State | Value |
|---|---|---|
| O1 Runs on target | unmeasured | |
| O2 One-command evidence bundle | measured 2026-11-17 | 1 of 2 consumer repositories |
| O3 Every verified claim is backed | unmeasured | |
| O4 Not only one person | measured 2026-11-18 | 1 of 839 merged pull requests in October; pick-up time not measured |
| O5 We run on our own chain | unmeasured | |

warte renders indicators and does not compute them (owner decision, 2026-11-12).
Its part is to make the numbers visible next to what the owner already reads,
and to publish the observations only it can make.

## How this plan was made

Two separate passes in fresh contexts. A planner was given north-star.toml, the
two indicator files, warte's rivet state, open issues, recent releases and its
cross-repo seams, and proposed five bets. A checker was given only
north-star.toml and the draft, and answered confirm, refute or cannot-verify
per bet. The plan below is the revision.

What the checker changed:
- **Dropped a separate Direction view.** pulseengine.eu's `/north-star` page
  already renders the file; warte keeps the history and the glance line.
- **O4 is never pooled.** Agent and maintainer pick-ups are reported apart; the
  O4 figure uses people other than the maintainer only, with n always shown.
- **The operator work moved out.** Decisions in the app, the attention
  schedule, the night glance and the new menu move no North Star indicator.
  They continue as operator work (#179, #181, #184, #187), not as bets here.
- **O2 membership is decided first, not after a pre-check.** Joining the
  denominator only if warte would pass is choosing the denominator to protect
  the number.
- **The gate inventory no longer claims to supply denominators.** It publishes
  which checks are required and the O5 guard observations.

Checker findings I did not adopt: it reported `measured_at`, the blind spot and
the 1/2 and 1/839 values as absent from north-star.toml. They are in the
published `O2.json` and `O4.json`, which the checker was deliberately not
given. `stale_after_days` and a schema version are new and are proposals to
pulseengine.eu.

## Bets

### 1. Indicator history and the North Star line in the night glance (REQ-NS-001)
- **Moves:** no indicator; makes O1..O5 visible where the owner already looks.
- **Ships:** every published observation kept; `GET /api/north-star`; one glance
  line with value, change, age and each guard's value ("guard unmeasured" when
  absent); a link to `/north-star`.
- **Acceptance:** a five-objective fixture with two files gives five rows, three
  `unmeasured` and none 0; a removed file turns `unmeasured` within one poll;
  live, a new `O2.json` shows within 24 hours.
- **Kill:** if the published shape changes twice in a month, stop and agree a
  versioned schema first.
- **Depends on:** pulseengine.eu keeping the files at stable public URLs.

### 2. Ready to first pick-up, per kind (REQ-NS-002), O4
- **Moves:** makes the second half of O4's indicator measurable. It does not
  raise merged pull requests by others.
- **Ships:** `GET /api/ready` with `ready_at`, `first_pickup_at` and
  `pickup_by_kind` per issue; warte alone can tell agents apart, from its agent
  registry and sessions.
- **Acceptance:** a replay of 10 recorded issues matches GitHub's timeline
  within one poll and classifies a planted agent pick-up as agent; the next O4
  layer carries a person-only median with its n, even below 5.
- **Kill:** fewer than 5 ready-labelled issues across the organisation after 30
  days (nothing is marked ready, so there is nothing to time); more than 20% of
  20 hand-checked pick-ups misclassified.
- **Depends on:** pulseengine.eu's O4 script and the ready label being named and
  applied in every public repository (not verified that it exists today).

### 3. Required checks and O5 guard observations (REQ-NS-003), O3 and O5 inputs
- **Moves:** neither O3 nor O5; supplies the "is a required check" predicate and
  the O5 guard facts (a gate behind a `*_GAP` switch, the equivalence test ran).
- **Acceptance:** for 3 hand-checked repositories the list equals GitHub's
  required checks; a planted required check that never runs shows "required, not
  seen"; any indicator built from it shows an absent numerator as `unmeasured`.
- **Kill:** required checks unreadable without an admin token on public
  repositories (then a script in pulseengine.eu owns it).
- **Cut first** if the release runs short.

### 4. warte's own evidence bundle (REQ-NS-004), O2: held for the owner
- **Question:** is warte a consumer repository in O2's denominator? warte is
  factory infrastructure, not a product a team ships to a microcontroller.
- **If yes:** the full bundle (trace, MC/DC, attestation), one negative control
  per piece. O2 may fall from 1/2 to 1/3 until warte passes.
- **If no:** the item is withdrawn; release signing continues outside the North
  Star.

## Not in this release
- The v1.7.0 milestone as scoped (38 of 39 artifacts not ready, none tied to an
  objective). Re-scoping it is a separate decision.
- O1: warte has nothing to show until on-target results are published.
- Authentication of the read API: real debt, its own release.
