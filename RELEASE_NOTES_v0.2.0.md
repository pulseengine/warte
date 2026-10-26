# Release note — v0.2.0

29 artifact(s) in scope: 29 delivered, 0 withheld.

## Functionalities provided

### design-decision

- `DD-DISCOVERY-001` (accepted) — mDNS via zeroconf crate, optional with static fallback
- `DD-DOCS-001` (accepted) — Embedded docs via clap + include_str! (like varve docs)
- `DD-PLATFORM-001` (accepted) — CiPlatform trait with GitHub + GitLab implementations
- `DD-RELEASE-001` (accepted) — Release pipeline via GitHub Actions + varve sbom + cosign
- `DD-RESEARCH-DB-001` (accepted) — BUILD: stay on SQLite + CRDT, evolve with per-table CRDT mapping
- `DD-RESEARCH-MDNS-001` (accepted) — ADOPT mdns-sd crate, BUILD discovery semantics in src/discovery/
- `DD-RESEARCH-TOOLS-001` (accepted) — BUILD: no adoption of similar tools — warte is purpose-built
- `DD-REUSE-001` (accepted) — Relay contract patterns: thin interfaces, seam traits, consumption contracts
- `DD-TEMPER-001` (accepted) — Temper as HTTP link, not code absorption

### requirement

- `REQ-DISCOVERY-001` (accepted) — mDNS/Bonjour service discovery for multi-instance
- `REQ-DISCOVERY-002` (accepted) — Discovery security — mDNS over authenticated channel
- `REQ-DOCS-001` (accepted) — Embedded queryable documentation (warte docs)
- `REQ-DOCS-002` (accepted) — Documentation coverage check (warte docs check)
- `REQ-LAYER-001` (accepted) — Warte binary included in the varve toolchain layer
- `REQ-PLATFORM-001` (accepted) — Support enterprise GitHub (GHE) in addition to github.com
- `REQ-PLATFORM-002` (accepted) — Support GitLab CI as a platform
- `REQ-PLATFORM-003` (accepted) — Platform abstraction layer for CI providers
- `REQ-RELEASE-001` (accepted) — Release artifact pipeline (SHA256SUMS, SBOM, SLSA, cosign)
- `REQ-RELEASE-002` (accepted) — Release execution with V-model traceability gate
- `REQ-REUSE-001` (accepted) — Reuse relay's contract patterns for agent orchestration
- `REQ-TEMPER-001` (accepted) — Temper integration as optional link, not full absorption

### verification

- `TEST-DISCOVERY-001` (accepted) — test_discovery_service_create + test_discovery_add_static_peer + test_discovery_update_peer — mDNS discovery service
- `TEST-DOCS-001` (accepted) — test_all_topics_load + test_coverage_passes + test_unknown_topic — embedded docs load and coverage check
- `TEST-DOCS-002` (accepted) — warte docs --check — CLI docs coverage command
- `TEST-LAYER-001` (accepted) — varve layer integration — warte.toml pin, varve-realms.toml, CLI conventions, embedded docs
- `TEST-PLATFORM-001` (accepted) — test_github_cloud/enterprise + test_gitlab_cloud/self_hosted + test_fleet_aggregator — CiPlatform trait
- `TEST-RELEASE-001` (accepted) — release-pipeline.sh + release.yml — SHA256SUMS, SBOM, SLSA, cosign, V-model gate
- `TEST-REUSE-001` (accepted) — test_agent_contract + test_consumption_contract + test_contract_serialization — relay contract patterns
- `TEST-TEMPER-001` (accepted) — test_temper_client_creation + test_temper_client_trailing_slash + test_temper_link_creation + test_dashboard_summary_serialization — temper HTTP link

## Limitations in relation to the committed scope

None — every artifact committed to v0.2.0 is release-ready.

## Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

8 of 29 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DISCOVERY-001`
- `TEST-DOCS-001`
- `TEST-DOCS-002`
- `TEST-LAYER-001`
- `TEST-PLATFORM-001`
- `TEST-RELEASE-001`
- `TEST-REUSE-001`
- `TEST-TEMPER-001`

1 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `DD-DISCOVERY-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-DOCS-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-PLATFORM-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-RELEASE-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-RESEARCH-DB-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-RESEARCH-MDNS-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-RESEARCH-TOOLS-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-REUSE-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-TEMPER-001` — 0 link(s), 0 source marker(s); decision-justification met
- `REQ-DISCOVERY-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-DISCOVERY-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-DOCS-001` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-DOCS-002` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-LAYER-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-PLATFORM-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-PLATFORM-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-PLATFORM-003` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-RELEASE-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-RELEASE-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-REUSE-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-TEMPER-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `TEST-DISCOVERY-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DOCS-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DOCS-002` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-LAYER-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-PLATFORM-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-RELEASE-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-REUSE-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-TEMPER-001` — 0 link(s), 0 source marker(s); no rule governs its type

## Approval

0 of 29 delivered artifact(s) carry a `reviewed-by` record.

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
