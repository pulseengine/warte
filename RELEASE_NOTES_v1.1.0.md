# Release note — v1.1.0

4 artifact(s) in scope: 4 delivered, 0 withheld.

## Functionalities provided

### requirement

- `REQ-CRIT-001` (accepted) — Critical path dependency graph — real repo dependencies from rivet/GitHub
- `REQ-DASH-012` (accepted) — FREE board listing in hardware availability — show all boards, not just claimed

### verification

- `TEST-CRIT-001` (accepted) — test_critical_path_simple_chain + test_critical_path_with_blocking + test_critical_path_no_dependencies + test_repo_node_is_blocking — dependency graph critical path
- `TEST-DASH-009` (accepted) — FREE board listing in hardware availability — all boards from catalog + claims, FREE/CLAIMED status

## Limitations in relation to the committed scope

None — every artifact committed to v1.1.0 is release-ready.

## Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

2 of 4 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-CRIT-001`
- `TEST-DASH-009`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-CRIT-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-012` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-CRIT-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DASH-009` — 0 link(s), 0 source marker(s); no rule governs its type

## Approval

0 of 4 delivered artifact(s) carry a `reviewed-by` record.

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
