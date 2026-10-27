# Release note — v0.3.0

10 artifact(s) in scope: 10 delivered, 0 withheld.

## Functionalities provided

### requirement

- `REQ-ASK-001` (accepted) — ETA estimation from measured waits (the ask feature)
- `REQ-ASK-002` (accepted) — ETA estimation API endpoint
- `REQ-ASK-003` (accepted) — Measured wait history storage
- `REQ-DISCOVERY-003` (accepted) — Full mDNS wire protocol via mdns-sd ServiceDaemon
- `REQ-REALTIME-001` (accepted) — WebSocket for real-time dashboard and mobile updates
- `REQ-SIGIL-001` (accepted) — Sigil/wsc signature verification for artifacts

### verification

- `TEST-ASK-001` (accepted) — test_eta_estimator_no_samples + test_eta_estimator_with_samples + test_percentile — ETA estimation from measured waits
- `TEST-DISCOVERY-002` (accepted) — Full mDNS wire protocol via mdns-sd ServiceDaemon — register, browse, goodbye, TTL expiry
- `TEST-REALTIME-001` (accepted) — test_event_broadcaster + test_event_serialization + test_all_event_types — WebSocket/SSE real-time updates
- `TEST-SIGIL-001` (accepted) — test_sigil_verifier_creation + test_sha256_fallback_sign + test_sha256_fallback_verify + test_wsc_signature_serialization — sigil/wsc signatures

## Limitations in relation to the committed scope

None — every artifact committed to v0.3.0 is release-ready.

## Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

4 of 10 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-ASK-001`
- `TEST-DISCOVERY-002`
- `TEST-REALTIME-001`
- `TEST-SIGIL-001`

6 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-ASK-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-ASK-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-ASK-003` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DISCOVERY-003` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-REALTIME-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-SIGIL-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-ASK-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DISCOVERY-002` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-REALTIME-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-SIGIL-001` — 0 link(s), 0 source marker(s); no rule governs its type

## Approval

0 of 10 delivered artifact(s) carry a `reviewed-by` record.

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
