# Release note — v0.8.0

6 artifact(s) in scope: 6 delivered, 0 withheld.

## Functionalities provided

### requirement

- `REQ-DEPLOY-001` (accepted) — Edge proxy deployment automation — Terraform for Hetzner/Netcup
- `REQ-STATUS-001` (accepted) — GitHub status API integration — real status from status.github.com
- `REQ-WEBHOOK-002` (accepted) — GitLab webhook endpoint — receive pipeline events

### verification

- `TEST-DEPLOY-001` (accepted) — Terraform configuration — Hetzner edge + Netcup secondary + WireGuard mesh + DNS
- `TEST-STATUS-001` (accepted) — test_status_client_creation + test_status_client_custom + test_status_serialization + test_incident_serialization — GitHub status API
- `TEST-WEBHOOK-002` (accepted) — GitLab webhook endpoint — X-Gitlab-Token verification + pipeline event processing

## Limitations in relation to the committed scope

None — every artifact committed to v0.8.0 is release-ready.

## Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

3 of 6 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DEPLOY-001`
- `TEST-STATUS-001`
- `TEST-WEBHOOK-002`

3 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-DEPLOY-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-STATUS-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-WEBHOOK-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-DEPLOY-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-STATUS-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-WEBHOOK-002` — 0 link(s), 0 source marker(s); no rule governs its type

## Approval

0 of 6 delivered artifact(s) carry a `reviewed-by` record.

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
