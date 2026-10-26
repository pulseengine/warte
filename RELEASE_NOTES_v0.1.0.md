# Release note — v0.1.0

61 artifact(s) in scope: 61 delivered, 0 withheld.

## Functionalities provided

### aadl-analysis-result

- `AADL-ANALYSIS-001` (accepted) — SPAR EMV2 fault tree — single points of failure
- `AADL-ANALYSIS-002` (accepted) — SPAR connectivity analysis — data flow paths

### aadl-component

- `AADL-COMP-001` (accepted) — EdgeProxy — sole internet-facing component (CTRL-004)
- `AADL-COMP-002` (accepted) — WarteInstance — the control tower (CTRL-001)
- `AADL-COMP-003` (accepted) — MobileClient — iOS/Android control (CTRL-003)
- `AADL-COMP-004` (accepted) — AgentRuntime — worker machine agent (CTRL-005)

### aadl-flow

- `AADL-FLOW-001` (accepted) — Agent registration flow (mTLS authenticated)
- `AADL-FLOW-002` (accepted) — Artifact download flow (integrity-verified)
- `AADL-FLOW-003` (accepted) — Multi-instance sync flow (CRDT, mTLS)
- `AADL-FLOW-004` (accepted) — Mobile control flow (mTLS + biometric)

### design-decision

- `DD-ARCH-001` (accepted) — Rust + Axum + Tokio for the server stack
- `DD-ARCH-002` (accepted) — SQLite for local state, with CRDT-based sync for multi-instance
- `DD-ARCH-003` (accepted) — mTLS with per-agent certificates for agent authentication
- `DD-ARCH-004` (accepted) — Sigil/wsc signatures for artifact integrity
- `DD-ARCH-005` (accepted) — Push-based revocation with active disconnection
- `DD-ARCH-006` (accepted) — WebSocket/SSE for real-time dashboard and mobile push
- `DD-ARCH-007` (accepted) — Edge proxy as the sole internet-facing component (Hetzner/Netcup)
- `DD-ARCH-008` (accepted) — Mobile app uses native iOS (Swift) and Android (Kotlin)
- `DD-ARCH-009` (accepted) — Migration from smithy — agent fleet data import
- `DD-ARCH-010` (accepted) — Migration from temper — repo hardening config import
- `DD-ARCH-011` (accepted) — CLI follows PulseEngine CLI conventions
- `DD-ARCH-012` (accepted) — RBAC with per-resource, per-action scoping

### requirement

- `REQ-ACCESS-001` (accepted) — Access rights are managed via a central policy service
- `REQ-ACCESS-002` (accepted) — Access revocation propagates within 5 seconds
- `REQ-ACCESS-003` (accepted) — Access rights are scoped per resource and action
- `REQ-AGENT-REG-001` (accepted) — Agent registration requires cryptographic identity proof
- `REQ-AGENT-REG-002` (accepted) — Agent identity is bound to a unique agent ID
- `REQ-API-001` (accepted) — REST API with structured JSON responses
- `REQ-API-002` (accepted) — API requires authentication for all mutating operations
- `REQ-ARTIFACT-001` (accepted) — Artifact downloads require agent authentication
- `REQ-ARTIFACT-002` (accepted) — Artifacts carry verifiable integrity proofs
- `REQ-ARTIFACT-003` (accepted) — Artifact download is scoped to agent access rights
- `REQ-CAPTURE-001` (accepted) — Agents capture and report structured data
- `REQ-CAPTURE-002` (accepted) — Captured data is queryable via the API
- `REQ-CLI-001` (accepted) — CLI follows PulseEngine CLI conventions
- `REQ-DASH-001` (accepted) — Dashboard displays status with freshness indicators
- `REQ-DASH-002` (accepted) — Dashboard provides agent registry overview
- `REQ-DASH-003` (accepted) — Dashboard provides CI status overview
- `REQ-EDGE-001` (accepted) — Edge proxy terminates TLS 1.3 and enforces rate limiting
- `REQ-EDGE-002` (accepted) — Edge proxy validates all requests against schemas
- `REQ-EDGE-003` (accepted) — Edge proxy auto-patches within 24 hours
- `REQ-MIGRATE-001` (accepted) — Migrate agent fleet data from smithy
- `REQ-MIGRATE-002` (accepted) — Migrate repo hardening config from temper
- `REQ-SYNC-001` (accepted) — Instances synchronize state via an authenticated sync protocol
- `REQ-SYNC-002` (accepted) — Sync conflicts are detected and resolved deterministically
- `REQ-SYNC-003` (accepted) — Instance divergence is detected and alerted

### verification

- `TEST-AADL-001` (accepted) — spar parse + spar analyze — AADL model validates with 0 errors
- `TEST-ACCESS-001` (accepted) — test_access_control_grant_revoke — grant, check, revoke lifecycle
- `TEST-ACCESS-002` (accepted) — test_access_control_orset_fail_safe — OR-Set concurrent grant+revoke resolves to REVOKED
- `TEST-AGENT-HB-001` (accepted) — test_agent_heartbeat — heartbeat updates agent health status
- `TEST-AGENT-REG-001` (accepted) — test_agent_registration — agent registration creates valid agent record
- `TEST-API-AUTH-001` (accepted) — test_token_issue_and_verify + test_token_expired + test_token_invalid_format + test_token_tampered + test_base64url_roundtrip — bearer token auth
- `TEST-ARTIFACT-001` (accepted) — test_artifact_store_and_integrity — store, download, verify integrity
- `TEST-CA-001` (accepted) — test_ca_create_and_load + test_issue_agent_cert — CA creates, loads, and issues agent certificates
- `TEST-CAPTURE-001` (accepted) — agent data capture and query endpoints
- `TEST-CLI-001` (accepted) — test_cli_version_format — CLI version follows PulseEngine conventions
- `TEST-DASH-001` (accepted) — dashboard overview returns agent and artifact counts
- `TEST-EDGE-003` (accepted) — Edge proxy auto-patching configuration — unattended-upgrades with 24h CVE window
- `TEST-HEALTH-001` (accepted) — health endpoint returns structured status with freshness
- `TEST-MIGRATE-001` (accepted) — test_migrate_smithy + test_migrate_temper — migration commands import data from smithy and temper
- `TEST-SYNC-001` (accepted) — test_sync_version_vectors — version vector increment, merge, divergence

## Limitations in relation to the committed scope

None — every artifact committed to v0.1.0 is release-ready.

## Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

25 of 61 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `AADL-ANALYSIS-001`
- `AADL-ANALYSIS-002`
- `AADL-COMP-001`
- `AADL-COMP-002`
- `AADL-COMP-003`
- `AADL-COMP-004`
- `AADL-FLOW-001`
- `AADL-FLOW-002`
- `AADL-FLOW-003`
- `AADL-FLOW-004`
- `TEST-AADL-001`
- `TEST-ACCESS-001`
- `TEST-ACCESS-002`
- `TEST-AGENT-HB-001`
- `TEST-AGENT-REG-001`
- `TEST-API-AUTH-001`
- `TEST-ARTIFACT-001`
- `TEST-CA-001`
- `TEST-CAPTURE-001`
- `TEST-CLI-001`
- `TEST-DASH-001`
- `TEST-EDGE-003`
- `TEST-HEALTH-001`
- `TEST-MIGRATE-001`
- `TEST-SYNC-001`

11 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `AADL-ANALYSIS-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `AADL-ANALYSIS-002` — 0 link(s), 0 source marker(s); no rule governs its type
- `AADL-COMP-001` — 0 link(s), 0 source marker(s); aadl-component-has-allocation missing (info)
- `AADL-COMP-002` — 0 link(s), 0 source marker(s); aadl-component-has-allocation missing (info)
- `AADL-COMP-003` — 0 link(s), 0 source marker(s); aadl-component-has-allocation missing (info)
- `AADL-COMP-004` — 0 link(s), 0 source marker(s); aadl-component-has-allocation missing (info)
- `AADL-FLOW-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `AADL-FLOW-002` — 0 link(s), 0 source marker(s); no rule governs its type
- `AADL-FLOW-003` — 0 link(s), 0 source marker(s); no rule governs its type
- `AADL-FLOW-004` — 0 link(s), 0 source marker(s); no rule governs its type
- `DD-ARCH-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-002` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-003` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-004` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-005` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-006` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-007` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-008` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-009` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-010` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-011` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-012` — 0 link(s), 0 source marker(s); decision-justification met
- `REQ-ACCESS-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-ACCESS-002` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-ACCESS-003` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-AGENT-REG-001` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-AGENT-REG-002` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-API-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-API-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-ARTIFACT-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-ARTIFACT-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-ARTIFACT-003` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-CAPTURE-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-CAPTURE-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-CLI-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-DASH-001` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-DASH-002` — 2 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-003` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-EDGE-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-EDGE-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-EDGE-003` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-MIGRATE-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-MIGRATE-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-SYNC-001` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-SYNC-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-SYNC-003` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `TEST-AADL-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-ACCESS-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-ACCESS-002` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-AGENT-HB-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-AGENT-REG-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-API-AUTH-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-ARTIFACT-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-CA-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-CAPTURE-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-CLI-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DASH-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-EDGE-003` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-HEALTH-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-MIGRATE-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-SYNC-001` — 0 link(s), 0 source marker(s); no rule governs its type

## Approval

0 of 61 delivered artifact(s) carry a `reviewed-by` record.

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
