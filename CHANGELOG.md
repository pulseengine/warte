# Changelog

Newest first. Releases up to v1.3.0 were merged while the rivet CI gate was red (#34); their "Cuttable" claims were checked only by a local rivet.

## Unreleased — v1.4.1

- `warte watch --ca-cert <pem>` (or `WARTE_CA_CERT`) trusts a reverse proxy's local CA, e.g. Caddy's `tls internal`. Without it, watching `https://<host>/warte` failed with `UnknownIssuer`. A file holding no certificate is an error up front.
- **`warte watch --ci`** (REQ-WATCH-001/002, DD-WATCH-001) blocks until a PR's or branch's CI runs start or finish, then prints JSON and exits: 0 on a change, 3 on timeout, 2 on usage errors, 1 on errors. Agents run it as a background command instead of polling. The server side is the long-poll `GET /api/watch`. Its cursor is the CI state itself, so a caller resuming with `--since` cannot miss a change and the server keeps no event log. Holds are capped at 900 s, 8 per client and 1024 in total.
- **The Overview is a control tower** (REQ-DASH-014). It shows a runway gauge with one cell per runner, KPIs with two-hour trends and six hours of traffic, all drawn from the ground station's own history. Each runner is a flight strip showing what it is running, for how long, its cgroup CPU and memory, and whether GitHub's busy flag disagrees with the host. A departures board lists queued runs, longest waiting first. It estimates start from the pool's measured median wait and marks runs past p90 as overdue. Every view gets warte's own look: night sky, tower cyan for what is moving, amber for what is waiting, and a UTC tower clock. No new requests and no external assets: the page is still offline-capable, and motion stops under `prefers-reduced-motion`.
- `GET /api/queue` and `GET /api/admission` give agents a machine-readable queue and an `open | batch | hold` verdict before they open a PR (#55). The verdicts come from the ground station's snapshot, the thresholds live in warte, and stale data never gates. Documented in the new `/api/openapi.json`.

## Unreleased — v1.4.0

- **Ground station** (absorbed smithy hub, DD-SMITHY-001): `warte hub` takes smithy hub's flags and reads its data directory as-is; it serves the hub API unchanged. Behaviour on the ground-station host's real data was checked against smithy's own binary: 0 differences in 26 requests (`scripts/hub-parity.sh`).
- `warte serve` runs the ground station in-process when warte.toml has a `[hub]` section, and mounts its API under `/ci`. The Queue, Critical Path, Ask and Health views now read the hub's live data, escape repo/branch/job names, and show the snapshot's real age.
- **Removed duplicates of the hub:**
  - the poller (never started), the GitHub and GitLab webhooks, and the `eta` module;
  - the `ci_jobs`, `wait_history` and `ci_status` tables (migration 005);
  - `warte migrate smithy`, which imported a format smithy never had;
  - the stub `/api/dashboard/ask`.
  REQ-POLLER-*, REQ-CI-DATA-001, REQ-WEBHOOK-001/002 and REQ-ASK-* are deprecated in rivet and traced to DD-SMITHY-001.
- **Fixed:**
  - Routing: since at least v1.0.0 every API route was mounted at `/api/api/...`, so `/api/health` returned 404 and the dashboard page was unreachable. The API is now at `/api` and the dashboard at `/`.
  - The dashboard Ask form posted to a route that requires auth (401). Ask is now a read-only GET.
  - Views no longer show a hardcoded "fresh" badge, and the footer shows the real version.
  - Every dashboard view now HTML-escapes agent- and author-controlled text: agent names and repos, artifact fields, hardware boards, friction and schedule entries, and Ask input. Before, an agent could put script into the page for every dashboard viewer.
  - The leftover authentication exemption for `/api/webhooks/*` is removed together with the webhooks.
  - The dashboard uses relative URLs, so it works behind a path prefix (e.g. `https://<host>/warte/`). Unknown paths return a text 404; before, browsers saved the empty 404 response as `document.txt`.
- **Dashboard in a browser** (found on the ground-station host; HTTP-level tests had passed):
  - Overview was always empty: its response was an out-of-band swap into an element that didn't exist.
  - Ask fetched the whole page into the content area. Ask is now a view with its own form.
  - The footer claimed an SSE connection that never existed (no extension loaded, nothing publishing). It is replaced by a 30-second refresh of the current view until `warte watch` brings live updates.
  - htmx is vendored and served by warte (2.0.4, Zero-Clause BSD) instead of loaded from unpkg.com.
  - Overview now shows the fleet (runners, queue, freshness).
  - New Playwright suite (`tests/playwright`, modelled on rivet's) with a CI job; on the old dashboard 8 of its 18 tests fail.
- Deployment is environment-specific and no longer lives in this repository: it moved to the operators' private Ansible (a role for warte: hash-checked binary, systemd unit, Caddy route for private-network clients including the LAN's IPv6 prefix, landing-page link). This public repository keeps only generic documentation.
- **Traceability corrected:** REQ-CRIT-001 and REQ-DASH-006 had been `accepted`, but the Critical Path view never showed the dependency chain they describe (`CriticalPathComputer` is not called by any view). Both are back to `approved` for v1.5.0 (#44).

## v1.3.2

- Release pipeline: the SBOM job installed `cargo-cyclonedx` unpinned and called a flag (`--output-file`) the current release doesn't have, so v1.3.1's release run failed and no GitHub release was created. The tool is now pinned (0.5.9) and called with `--override-filename sbom` (CycloneDX 1.5). A new oracle requires every `cargo install` in the workflows to pin `--version` or `--rev`.
- The v1.3.1 release run otherwise confirmed the pipeline end to end: the rivet gate passed for the tag, both builds passed the `warte --version` check, and checksums and cosign signing succeeded.

## v1.3.1 — tagged 2026-10-28, not released

Tagged at `b5ce41c`; the release run failed at SBOM generation (fixed in v1.3.2), so there are no v1.3.1 binaries.

- CI installs rivet from `pulseengine/rivet` git at a pinned rev (v0.41.1, matching the varve layer). It used to install crates.io `rivet-cli`, which is a different project (#34).
- The release gate runs `rivet release status` for the tag being cut and fails on rivet's exit code (#34).
- CI jobs (build, test, clippy, rivet) run on the smithy `rust-cpu` pool. Pull requests from forks are routed to GitHub-hosted runners. That is routing, not isolation: a fork PR can edit the workflow, so the smithy runner-group policy and fork-PR approval are the actual boundary. All release jobs run on GitHub-hosted runners and restore no caches (#35).
- rivet is installed into a job-scoped `$RUNNER_TEMP` directory and never reused from `$HOME`; third-party actions are pinned to commit SHAs; workflows default to `bash`.
- Release assets: binaries, `SHA256SUMS` and the cosign bundle are now laid out flat, so signing and the GitHub release find them. Before this, every Release workflow run (v1.0.0–v1.3.0) failed, and the published releases have no assets. A missing asset now fails the release (`fail_on_unmatched_files`).
- License changed from MIT to **Apache-2.0**, the PulseEngine default; `LICENSE` added (there was none before). This also lets smithy hub's code move into warte unchanged (v1.4.0).
- `tests/ci_workflows.rs` checks all of the above from the parsed workflow YAML; each check was confirmed to fail on an injected violation.
- Release notes moved from 13 root `RELEASE_NOTES_*` files into this changelog; root `SHA256SUMS*` files removed (they ship as release assets). `Cargo.toml` version now follows the release, and release CI checks that `warte --version` reports the tag (#39).

## v1.3.0

2 artifact(s) in scope: 2 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-CRIT-003` (accepted) — GitHub dependency graph API integration — auto-discover dependencies from GitHub

#### verification

- `TEST-CRIT-003` (accepted) — test_spdx_sbom_deserialization + test_integration_creation + test_integration_ghe — GitHub dependency graph API

### Limitations in relation to the committed scope

None — every artifact committed to v1.3.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 2 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-CRIT-003`

1 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-CRIT-003` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-CRIT-003` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 2 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v1.2.0

2 artifact(s) in scope: 2 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-CRIT-002` (accepted) — Rivet externals integration — auto-discover cross-repo dependencies for critical path

#### verification

- `TEST-CRIT-002` (accepted) — test_rivet_externals_deserialization + test_integration_creation + test_build_computer_with_manual_deps — rivet externals integration

### Limitations in relation to the committed scope

None — every artifact committed to v1.2.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 2 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-CRIT-002`

1 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-CRIT-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-CRIT-002` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 2 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v1.1.0

4 artifact(s) in scope: 4 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-CRIT-001` (accepted) — Critical path dependency graph — real repo dependencies from rivet/GitHub
- `REQ-DASH-012` (accepted) — FREE board listing in hardware availability — show all boards, not just claimed

#### verification

- `TEST-CRIT-001` (accepted) — test_critical_path_simple_chain + test_critical_path_with_blocking + test_critical_path_no_dependencies + test_repo_node_is_blocking — dependency graph critical path
- `TEST-DASH-009` (accepted) — FREE board listing in hardware availability — all boards from catalog + claims, FREE/CLAIMED status

### Limitations in relation to the committed scope

None — every artifact committed to v1.1.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

2 of 4 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-CRIT-001`
- `TEST-DASH-009`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-CRIT-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-012` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-CRIT-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DASH-009` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 4 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v1.0.0

4 artifact(s) in scope: 4 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-DASH-011` (accepted) — Hardware availability in queue view — boards free/claimed, by whom, since when
- `REQ-POLLER-002` (accepted) — Poller measured wait recording — record queued → started transitions from polling

#### verification

- `TEST-DASH-008` (accepted) — Hardware availability in queue view — boards CLAIMED/FREE, agent, duration
- `TEST-POLLER-002` (accepted) — Poller measured wait recording — queued → started transitions tracked, waits recorded in wait_history

### Limitations in relation to the committed scope

None — every artifact committed to v1.0.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

2 of 4 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DASH-008`
- `TEST-POLLER-002`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-DASH-011` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-POLLER-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-DASH-008` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-POLLER-002` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 4 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.9.0

3 artifact(s) in scope: 3 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-DASH-009` (accepted) — Wire StatusClient into outages view — real GitHub status in dashboard
- `REQ-DASH-010` (accepted) — Dashboard navigation — add buttons for queue, critical path, resources, outages, friction, schedule, health views

#### verification

- `TEST-DASH-007` (accepted) — StatusClient wired into outages view + nav buttons for all 11 dashboard views

### Limitations in relation to the committed scope

None — every artifact committed to v0.9.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 3 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DASH-007`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-DASH-009` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-010` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-DASH-007` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 3 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.8.0

6 artifact(s) in scope: 6 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-DEPLOY-001` (accepted) — Edge proxy deployment automation — Terraform for Hetzner/Netcup
- `REQ-STATUS-001` (accepted) — GitHub status API integration — real status from status.github.com
- `REQ-WEBHOOK-002` (accepted) — GitLab webhook endpoint — receive pipeline events

#### verification

- `TEST-DEPLOY-001` (accepted) — Terraform configuration — Hetzner edge + Netcup secondary + WireGuard mesh + DNS
- `TEST-STATUS-001` (accepted) — test_status_client_creation + test_status_client_custom + test_status_serialization + test_incident_serialization — GitHub status API
- `TEST-WEBHOOK-002` (accepted) — GitLab webhook endpoint — X-Gitlab-Token verification + pipeline event processing

### Limitations in relation to the committed scope

None — every artifact committed to v0.8.0 is release-ready.

### Result of verification measures

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

### Approval

0 of 6 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.7.0

3 artifact(s) in scope: 3 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-DASH-007` (accepted) — Dashboard resources view — who needs what (RAM, disk, runners, boards)
- `REQ-DASH-008` (accepted) — Dashboard outages + friction views — GitHub status, fleet health, tool errors

#### verification

- `TEST-DASH-006` (accepted) — Dashboard resources + outages + friction + schedule + health views — complete dashboard

### Limitations in relation to the committed scope

None — every artifact committed to v0.7.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 3 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DASH-006`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-DASH-007` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-008` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-DASH-006` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 3 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.6.0

3 artifact(s) in scope: 3 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-DASH-005` (accepted) — Dashboard queue view — runner pool, waiting jobs, hardware availability
- `REQ-DASH-006` (accepted) — Dashboard critical path view — longest chain of dependent gates

#### verification

- `TEST-DASH-005` (accepted) — Dashboard queue + critical path views — runner pools, waiting jobs, bottlenecks, ETA pool stats

### Limitations in relation to the committed scope

None — every artifact committed to v0.6.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 3 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DASH-005`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-DASH-005` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-006` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-DASH-005` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 3 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.5.0

4 artifact(s) in scope: 4 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-POLLER-001` (accepted) — Periodic CI status poller — fallback when webhooks are not configured
- `REQ-WEBHOOK-001` (accepted) — GitHub webhook HTTP endpoint — receive workflow_run events

#### verification

- `TEST-POLLER-001` (accepted) — test_poller_config_default + test_poller_config_custom + test_poller_creation — periodic CI status poller
- `TEST-WEBHOOK-001` (accepted) — GitHub webhook endpoint — HMAC-SHA256 signature verification + workflow_run event processing

### Limitations in relation to the committed scope

None — every artifact committed to v0.5.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

2 of 4 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-POLLER-001`
- `TEST-WEBHOOK-001`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-POLLER-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-WEBHOOK-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-POLLER-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-WEBHOOK-001` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 4 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.4.0

4 artifact(s) in scope: 4 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-CI-DATA-001` (accepted) — GitHub CI data collection — webhook receiver + status poller for ETA estimation
- `REQ-DASH-004` (accepted) — Dashboard HTML frontend — HTMX-based overview, ask, queue views

#### verification

- `TEST-CI-DATA-001` (accepted) — test_parse_datetime + test_infer_runner_pool + test_webhook_deserialization + test_process_webhook — GitHub CI data collection
- `TEST-DASH-004` (accepted) — Dashboard HTML frontend — HTMX-based overview, ask, agents, artifacts views with SSE

### Limitations in relation to the committed scope

None — every artifact committed to v0.4.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

2 of 4 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-CI-DATA-001`
- `TEST-DASH-004`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-CI-DATA-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-004` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-CI-DATA-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DASH-004` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 4 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.3.0

10 artifact(s) in scope: 10 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-ASK-001` (accepted) — ETA estimation from measured waits (the ask feature)
- `REQ-ASK-002` (accepted) — ETA estimation API endpoint
- `REQ-ASK-003` (accepted) — Measured wait history storage
- `REQ-DISCOVERY-003` (accepted) — Full mDNS wire protocol via mdns-sd ServiceDaemon
- `REQ-REALTIME-001` (accepted) — WebSocket for real-time dashboard and mobile updates
- `REQ-SIGIL-001` (accepted) — Sigil/wsc signature verification for artifacts

#### verification

- `TEST-ASK-001` (accepted) — test_eta_estimator_no_samples + test_eta_estimator_with_samples + test_percentile — ETA estimation from measured waits
- `TEST-DISCOVERY-002` (accepted) — Full mDNS wire protocol via mdns-sd ServiceDaemon — register, browse, goodbye, TTL expiry
- `TEST-REALTIME-001` (accepted) — test_event_broadcaster + test_event_serialization + test_all_event_types — WebSocket/SSE real-time updates
- `TEST-SIGIL-001` (accepted) — test_sigil_verifier_creation + test_sha256_fallback_sign + test_sha256_fallback_verify + test_wsc_signature_serialization — sigil/wsc signatures

### Limitations in relation to the committed scope

None — every artifact committed to v0.3.0 is release-ready.

### Result of verification measures

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

### Approval

0 of 10 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.2.0

29 artifact(s) in scope: 29 delivered, 0 withheld.

### Functionalities provided

#### design-decision

- `DD-DISCOVERY-001` (accepted) — mDNS via zeroconf crate, optional with static fallback
- `DD-DOCS-001` (accepted) — Embedded docs via clap + include_str! (like varve docs)
- `DD-PLATFORM-001` (accepted) — CiPlatform trait with GitHub + GitLab implementations
- `DD-RELEASE-001` (accepted) — Release pipeline via GitHub Actions + varve sbom + cosign
- `DD-RESEARCH-DB-001` (accepted) — BUILD: stay on SQLite + CRDT, evolve with per-table CRDT mapping
- `DD-RESEARCH-MDNS-001` (accepted) — ADOPT mdns-sd crate, BUILD discovery semantics in src/discovery/
- `DD-RESEARCH-TOOLS-001` (accepted) — BUILD: no adoption of similar tools — warte is purpose-built
- `DD-REUSE-001` (accepted) — Relay contract patterns: thin interfaces, seam traits, consumption contracts
- `DD-TEMPER-001` (accepted) — Temper as HTTP link, not code absorption

#### requirement

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

#### verification

- `TEST-DISCOVERY-001` (accepted) — test_discovery_service_create + test_discovery_add_static_peer + test_discovery_update_peer — mDNS discovery service
- `TEST-DOCS-001` (accepted) — test_all_topics_load + test_coverage_passes + test_unknown_topic — embedded docs load and coverage check
- `TEST-DOCS-002` (accepted) — warte docs --check — CLI docs coverage command
- `TEST-LAYER-001` (accepted) — varve layer integration — warte.toml pin, varve-realms.toml, CLI conventions, embedded docs
- `TEST-PLATFORM-001` (accepted) — test_github_cloud/enterprise + test_gitlab_cloud/self_hosted + test_fleet_aggregator — CiPlatform trait
- `TEST-RELEASE-001` (accepted) — release-pipeline.sh + release.yml — SHA256SUMS, SBOM, SLSA, cosign, V-model gate
- `TEST-REUSE-001` (accepted) — test_agent_contract + test_consumption_contract + test_contract_serialization — relay contract patterns
- `TEST-TEMPER-001` (accepted) — test_temper_client_creation + test_temper_client_trailing_slash + test_temper_link_creation + test_dashboard_summary_serialization — temper HTTP link

### Limitations in relation to the committed scope

None — every artifact committed to v0.2.0 is release-ready.

### Result of verification measures

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

### Approval

0 of 29 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.1.0

61 artifact(s) in scope: 61 delivered, 0 withheld.

### Functionalities provided

#### aadl-analysis-result

- `AADL-ANALYSIS-001` (accepted) — SPAR EMV2 fault tree — single points of failure
- `AADL-ANALYSIS-002` (accepted) — SPAR connectivity analysis — data flow paths

#### aadl-component

- `AADL-COMP-001` (accepted) — EdgeProxy — sole internet-facing component (CTRL-004)
- `AADL-COMP-002` (accepted) — WarteInstance — the control tower (CTRL-001)
- `AADL-COMP-003` (accepted) — MobileClient — iOS/Android control (CTRL-003)
- `AADL-COMP-004` (accepted) — AgentRuntime — worker machine agent (CTRL-005)

#### aadl-flow

- `AADL-FLOW-001` (accepted) — Agent registration flow (mTLS authenticated)
- `AADL-FLOW-002` (accepted) — Artifact download flow (integrity-verified)
- `AADL-FLOW-003` (accepted) — Multi-instance sync flow (CRDT, mTLS)
- `AADL-FLOW-004` (accepted) — Mobile control flow (mTLS + biometric)

#### design-decision

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

#### requirement

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

#### verification

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

### Limitations in relation to the committed scope

None — every artifact committed to v0.1.0 is release-ready.

### Result of verification measures

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

### Approval

0 of 61 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled
