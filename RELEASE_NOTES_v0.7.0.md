# Release note — v0.7.0

3 artifact(s) in scope: 3 delivered, 0 withheld.

## Functionalities provided

### requirement

- `REQ-DASH-007` (accepted) — Dashboard resources view — who needs what (RAM, disk, runners, boards)
- `REQ-DASH-008` (accepted) — Dashboard outages + friction views — GitHub status, fleet health, tool errors

### verification

- `TEST-DASH-006` (accepted) — Dashboard resources + outages + friction + schedule + health views — complete dashboard

## Limitations in relation to the committed scope

None — every artifact committed to v0.7.0 is release-ready.

## Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 3 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DASH-006`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-DASH-007` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-008` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-DASH-006` — 0 link(s), 0 source marker(s); no rule governs its type

## Approval

0 of 3 delivered artifact(s) carry a `reviewed-by` record.

## Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled
