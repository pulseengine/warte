# Release note — v1.3.0

2 artifact(s) in scope: 2 delivered, 0 withheld.

## Functionalities provided

### requirement

- `REQ-CRIT-003` (accepted) — GitHub dependency graph API integration — auto-discover dependencies from GitHub

### verification

- `TEST-CRIT-003` (accepted) — test_spdx_sbom_deserialization + test_integration_creation + test_integration_ghe — GitHub dependency graph API

## Limitations in relation to the committed scope

None — every artifact committed to v1.3.0 is release-ready.

## Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 2 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-CRIT-003`

1 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-CRIT-003` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-CRIT-003` — 0 link(s), 0 source marker(s); no rule governs its type

## Approval

0 of 2 delivered artifact(s) carry a `reviewed-by` record.

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
