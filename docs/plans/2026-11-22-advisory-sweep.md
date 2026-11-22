# Plan: advisory sweep, answered per affected user (warte#47)

> **Status 2026-11-22.** The owner chose a written plan first; nothing is built yet.


Status: plan for review (owner, 2026-11-22: plan first, reviewed before code). No code yet.
Unknowns are marked **UNKNOWN**. They are measured in phase 0 and not guessed.

## 1. What exists today (read 2026-11-22)

- **Per-repo gate.** warte's `Dependency audit` job (`.github/workflows/ci.yml`) runs `cargo audit` 0.22.1 on every committed lockfile. Planted controls run first: a lock holding smallvec 1.6.0 must fail with RUSTSEC-2021-0003. The job runs only on push and PR, so it says nothing about unchanged code or about what was shipped.
- **GitHub source** (`src/ghsource`). Read-only REST client with an ETag cache (a 304 is free) and `rate_reserve` (default 150): no request goes out while GitHub reports that many or fewer left. It scans repos pushed in the last `active_days` (7), so it is a CI scanner, not an inventory of every repo.
- **Reports intake** (`src/reports`, warte#87, seam `pulseengine-warte/reports`): `POST /api/reports/<kind>` with `reports:write:<kind>`. Each report must carry its schema string. Size limit 2 MiB, latest wins, a stale flag after N days, signed-in views only. Its rule: "warte computes nothing and calls no API for them." The one kind today is `seam-check.v1`, posted by a weekly workflow.
- **Findings + glance** (`src/findings`, `src/glance`). The glance has an always-open `security` kind. Open high or critical findings and a silent detector (36 h) each become a `needs-you` item. Findings has "accept into baseline", the precedent for a VEX-style decision.
- **varve layers** (varve CLI, read locally):
  - The pulseengine realm's contents live in the public repo `pulseengine/pulseengine-layers`: `layer.toml`, `line-index/<line>.json` listing every layer of a line with its manifest digest (2026.11 is at counter 37), and `line-status/` (support window, yanks, known problems).
  - `varve inspect --json` (on an installed layer) lists each payload's `kind`, `name`, `version`, `platform`, `realm` and `proof_signer`, plus the composition. Layer 2026.10.30 has 54 payloads: 12 tools, 1 crate, 1 docs, 2 vsix.
  - `varve sbom` is CycloneDX 1.6, transcribed from the signed manifest. Each component has `purl` = `pkg:github/pulseengine/<repo>@<version>` and a `distribution` URL that names the release tag (`.../releases/download/v0.43.0/...`).
  - **A layer lists our tools, not their crate graphs.** Matching a crate advisory to a layer must go layer → tool `repo@tag` → `Cargo.lock` at that tag.
  - `inspect` and `sbom` work offline on an **installed** layer only; there is no remote or manifest-only read. **UNKNOWN**: the download size of `varve install` per layer for one platform.
- **OSV** (osv.dev API docs):
  - `POST /v1/querybatch` takes `{package:{name,ecosystem}|{purl}, version}` per query and returns only `{id, modified}` per result, in input order. Full records come from `GET /v1/vulns/{id}`. A query paginates past 1,000 results, a batch past 3,000.
  - "Currently there are no limits on the API." No auth.
  - crates.io names match case-insensitively, with `-` and `_` treated as equal. RustSec and GHSA are both sources.
  - Bulk export: `osv-vulnerabilities/<ECOSYSTEM>/all.zip` plus `modified_id.csv`.
  - **UNKNOWN**: the maximum batch size (not stated); the exact cargo purl form (not stated, so query by name + ecosystem `crates.io`); CVSS presence on RustSec records (samples show only `ecosystem_specific.severity`).
- **Not there**:
  - rivet's `release-artifact` / `sbom-component` / `vulnerability` types are not in warte's `rivet schema list`. **UNKNOWN** whether rivet ships a supply-chain schema.
  - The probe inventory does not report which layer a machine pins.
- **Org**: 27 public, non-archived Rust repos (`gh repo list`, 2026-11-22). One shared 5,000 requests/hour core budget (4,505 left when read).

## 2. Who is a "user"? (concrete)

warte cannot see external downloaders, so it answers only for users it can name, in this order:

1. **A layer in its support window.** Every layer in `line-index/<line>.json` whose line-status window has not ended (rolling = one month since varve v0.33.0). This is the issue's main case: people run layers.
2. **A repo that pins a layer.** `varve.toml` on the default branch of each public repo (warte itself pins `2026.10.30`). Answered as "warte pins 2026.10.30 → meld 0.65.1 → `<crate>@<v>` (RUSTSEC-…); first fixed layer: 2026.11.x".
3. **A GitHub release** of a tool, which covers people who download binaries directly. Shown per `repo@tag`, newest release only in v1.
4. **The default branch** (the next release). This is what the per-repo CI gate already covers, but only on push.

Out of scope: unknown external users, and machines. The probe does not report pins; that could be added later.

## 3. Matching

1. **Inventory.**
   - For each layer in scope, read `varve inspect --json` and keep `kind=tool|crate` rows of realm `pulseengine`.
   - Map each row to `(repo, tag)` from the SBOM `purl` and `distribution` URL. Do not assume a `v` prefix.
   - Payloads from a composed realm (`pulseengine-wasm`: upstream wasm-tools etc.) and non-Rust kinds (vsix, sdk, zephyr-module, docs) are listed as **not covered**. They are never shown as clean.
2. **Locks.**
   - `Cargo.lock` at each `(repo, tag)`. Immutable, so it is fetched once and cached forever.
   - `Cargo.lock` on each repo's default branch.
   - Keep `source = registry+…crates.io-index` packages. Git and path deps are listed as not covered.
3. **Query.**
   - Deduplicate `(crate, version)` across all locks and send them to OSV `querybatch` with ecosystem `crates.io`.
   - Read `GET /v1/vulns/{id}` only for ids that are new or whose `modified` changed. Keep the cache keyed by id + modified.
   - Skip withdrawn records.
   - Take aliases (CVE/GHSA/RUSTSEC), severity, `affected[].ranges` (the first `fixed`) and RustSec informational class (unmaintained/unsound) from the record.
4. **Answer per user.**
   - affected = (user, tool, crate@version, advisory).
   - **remedy** = the oldest layer in the same line (or the next line) whose tool tag's lock no longer matches. Failing that, "fixed in `<crate>` ≥ x, no layer yet", which feeds release planning.
   - The sweep re-uses its own lock and OSV answers for every candidate layer, so finding the remedy costs no extra requests.
5. **Known over-approximation.** `Cargo.lock` holds every target's dependencies, and a release may not be built `--locked`. A hit is "lock contains" until someone triages it. **UNKNOWN** per repo whether its release workflow builds `--locked`; phase 0 checks this.

## 4. Where the sweep runs

| | A: warte polls GitHub + OSV | B: a scheduled job posts `advisory-sweep.v1` to the reports intake |
|---|---|---|
| GitHub budget | Main locks via the contents API with ETags: about 27 conditional requests per sweep, mostly 304s. Tag locks once each, cached. Daily, that is about 60 requests/day: negligible against 5,000/h. | `git ls-remote --tags` and shallow fetches of tags read locks without REST calls. The job's own token, not the shared org budget. |
| OSV | outbound HTTPS from warte's host | outbound from the runner |
| varve layers | warte's host must hold varve, the trust root and every installed layer (disk, **UNKNOWN** size). The work instance has no use for any of it. | `varve install --platform <one triple>` into a job cache, once per layer |
| matching code | new in warte; duplicates what cargo-audit/OSV tooling already does | in the job. Can call `cargo audit --file` as a cross-check on the same locks |
| fits warte's rules | breaks "warte renders, producers compute" (#87), which applies to every report-shaped view since #76 | follows the reports seam exactly; warte stays read-only on GitHub |
| failure mode | logged in warte | a job that stops posting: the stale flag (and glance, below) catches it |

The budget does not decide this: both are far below 1% of the shared hourly budget. The deciding points are the reports rule and that varve layers must not land on warte's host. **Choose B.** The job lives in warte's repo, so producer and schema change in one reviewed PR, with a daily schedule. warte adds one report kind, one view and the glance inputs.

## 5. Report schema `advisory-sweep.v1` (through the intake)

```json
{ "schema": "advisory-sweep.v1", "generated_at": "RFC3339", "osv_queried": 4210,
  "controls": {"planted_lock_hit": true},
  "users": [ {"kind": "layer|pin|release|main", "id": "2026.10.30 | warte | meld@v0.65.1 | meld",
              "supported_until": "RFC3339|null", "pins_layer": "2026.10.30|null"} ],
  "hits": [ {"user": "<users[].id>", "tool": "meld", "tag": "v0.65.1", "crate": "x", "version": "1.2.3",
             "advisory": "RUSTSEC-…", "aliases": ["CVE-…","GHSA-…"], "class": "vulnerability|unmaintained|unsound",
             "severity": "critical|high|medium|low|unrated", "fixed": ">=1.2.4|null",
             "remedy": {"layer": "2026.11.9|null", "text": "…"}, "url": "https://osv.dev/vulnerability/…"} ],
  "not_covered": [ {"user": "…", "payload": "wasm-tools", "why": "composed realm|non-Rust|git dep|no lock at tag"} ],
  "errors": ["…"] }
```

warte's `check` validates the schema string, the lists, the enum values and a cap of 2,000 hits, and names the field that fails. The intake, not the producer, sets `stale_days: 2`. The seam is a new block, `pulseengine-warte/advisories` (provider: warte's job, consumer: warte), not an edit to the pulseengine.eu seam.

## 6. View, glance, alerts

- **Advisories view** (signed-in, like Seams). One table per advisory: affected users grouped by kind (layer, pin, release, main), the remedy layer or "no fixed layer yet", and severity. Then a "not covered" list and the generation time with a stale pill. Also `GET /api/advisories`, the raw report, for agents.
- **Glance** (new `Inputs`):
  - An item for a **critical** hit on any user.
  - An item for a **high** hit on a supported layer or main **when a fix exists**.
  - An item for a sweep report older than 2 days (the same idea as the silent detector).
  - Everything else counts in `quiet`. Unmaintained/unsound are never items.
  - **UNKNOWN**: how `unrated` RustSec records should rank. Proposal: rank as high, owner to confirm.
- **No GitHub writes in v1.** No issues opened, no line-status notices.

## 7. Phases (v1 small)

- **Phase 0, measure (1 run, no warte code).** A one-off run over the supported layers and the 27 mains, to get:
  - the install size per layer;
  - the number of distinct `(crate, version)` pairs;
  - the share of tool payloads that resolve to a lock at their tag;
  - which release workflows build `--locked`;
  - hits today, cross-checked against `cargo audit` on the same locks.

  Its numbers decide the kill criterion (§9).
- **Phase 1, v1.**
  - The job runs daily, with planted controls first: smallvec 1.6.0 → RUSTSEC-2021-0003, as in ci.yml. A missing control hit sets `controls.planted_lock_hit=false`, and the view shows the report as failed.
  - Users 1, 2, 4. Report kind, view, API, glance items. One requirement in rivet (REQ-ADVISORY-001).
- **Phase 2.**
  - Users of kind 3 (releases).
  - Triage in warte: accept as not-affected with a reason, which is the VEX statement, modelled on the findings baseline-accept.
  - A line in agents' status Markdown.
  - Repos with no advisory gate in CI as a temper/hygiene finding.
  - The composed realm's upstream tools.
- **Phase 3 (needs owner decisions and seams).**
  - Deduplicated issues in affected repos, through the token broker (`issues:write` per agent).
  - Known-problem entries in `pulseengine-layers` line-status, so `varve status` tells consumers which layer to move to (seam with pulseengine-layers).
  - Recording in rivet's supply-chain schema, if rivet ships one.

## 8. Risks

- **False positives from whole-graph locks** (Windows-only crates in Linux binaries). Mitigation: the "lock contains" wording now, VEX triage in phase 2.
- **Layer cardinality.** About 37 layers in one month, but few distinct `(repo, tag)` pairs, and tag locks are cached forever. Restrict to the support window.
- **Install cost on the runner.** One platform only, plus a cache. If phase 0 shows it is too large, ask varve for a manifest-only `inspect` (report-tool-friction) and do not fall back to `layer.toml` history, which is unsigned intent, not what shipped.
- **OSV undocumented limits.** No stated batch maximum. Batch at 500, back off on 429/400, and keep the bulk `crates.io` zip as a fallback (**UNKNOWN**: directory name).
- **A silent producer** looks like "no advisories". Mitigation: the stale glance item and planted controls.
- **The view could claim more than the sweep checked.** Mitigation: the `not_covered` list is rendered, never hidden.

## 9. Kill criterion (falsifiable)

Measured 60 days after phase 1 ships, from the stored reports and the repos' CI history:

**Stop at phase 1, and do not build phases 2–3, if both hold:**

- **(a)** Zero hits on a layer, pin or release that were not also present on that repo's default branch at the same time. That would mean the per-repo CI gates plus a daily main sweep answer everything, and the per-user mapping bought nothing.
- **(b)** Every main hit was also reported by that repo's own audit job within 24 h.

**Kill the layer half outright after phase 0** if fewer than 80% of the `pulseengine`-realm tool payloads in supported layers resolve to a `Cargo.lock` at their tag. Then "answered per user" cannot be answered this way, and the honest fix is a lock or SBOM-of-dependencies in each layer, which is varve's to add.

## 10. Recommendation

Build it as a **producer job posting `advisory-sweep.v1` through the existing reports intake** (option B), after a **phase 0** measurement run. v1 answers for supported layers, repos that pin a layer, and default branches. It renders an Advisories view and API, and raises only critical hits, fixable high hits and a stale sweep on the glance. GitHub writes, VEX triage and rivet supply-chain records wait for phases 2–3 and their seams.

The request budget is not a constraint either way: about 60 GitHub requests/day, or zero over git. The reasons for B are architectural.
