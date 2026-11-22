# Plan: retire warte's artifact blob store, point at varve layers (warte#38)

> **Status 2026-11-22.** The owner chose plans and seam options first. The dead store's code was removed in v1.6.44; the artifacts table stays until the operator confirms it has 0 rows. The Layers view waits for varve's answer on the seam.


Status: plan plus seam options. Owner decision 2026-11-22: varve agrees the seam first; warte's side is built after that.

## Intent

warte stops holding artifact bytes and its own integrity code. It shows which signed varve layer each machine runs: layer, manifest digest, verify verdict, yank state. Agents fetch bytes with varve (`varve install`, `varve run`), not over HTTP from warte.

## What exists today (read 2026-11-22, branch feat/findings at badd052)

- `artifacts` table (migrations/001_initial.sql): `content BLOB`, `integrity_proof BLOB`, `integrity_type`, `access_scope`, plus a sync `version_vector`.
- `src/artifact/mod.rs` (216 lines): `store`, `download`, `list`, `verify_integrity`.
  - **Nothing in production writes to it.** `ArtifactStore::store` is called only from `tests/integration.rs:200`. `warte artifact upload` and `download` print a line and stop (`// TODO` in `src/main.rs`).
  - **The integrity check proves nothing.** `get_content_hash` re-hashes the same `content` column that `download` just hashed, so the two values always match. `verify_integrity` says `TODO: verify wsc signature`. `integrity_proof` is stored and never checked.
  - `download` returns the bytes as a JSON number array (`Vec<u8>`).
- `src/sigil/mod.rs` (269 lines, `SigilVerifier`): nothing outside the module calls it.
- Routes in `src/server/api.rs`: `GET /api/artifacts` and `GET /api/artifacts/{id}/download`, both in the read group. The auth middleware marks the download path as a sensitive read (token or login). The handler never checks the per-artifact `access_scope`.
- Other users: `dashboard::artifacts_partial`, the artifact count in `/api/dashboard/overview`, `warte artifact list`, and docs topic `artifact-distribution`. That topic says "the server verifies the proof before serving", which the code above does not do.
- Trace: DD-ARCH-004 (wsc signatures for artifacts), REQ-ARTIFACT-001..003 and REQ-SIGIL-001 are all `accepted`. The code does not meet them.

## varve as it is (varve 0.47.0, read from `--help` and `varve docs`)

- `varve inspect [--layer L] [--json]`: every payload with `name`, `version`, `kind`, `platform`, `digest`, `ingest_proof` (for example `cosign-sums`), plus the layer's `manifest_digest` and its composition.
- `varve verify [--all]`: checks the **pinned** layer offline (envelope, the digest of every entry for **this host's platform only**, anti-rollback, PATH shadowing). It has no `--json` and no `--layer`. The verdict is the exit code (0 ok, 1 cannot vouch; see `varve docs exit-codes`).
- `varve status [--json]`: support window and yank state. Exit 3 means yanked.
- `check-attestation` / `sign-attestation`: bind an SBOM, provenance or audit document to a layer.
- `deposit` is a CI step signed with the realm key. varve "runs no server and pushes nothing".
- The `varve-core` crate has `consumer::payload_status(...)`. It returns `Verified`, `DigestMismatch`, `AbsentFromLayer` and other variants, and takes the trust root as a parameter (`varve docs consumer-api`).
- varve's threat model: `verify` does not seal the directory, and `which`/`run` do not re-verify.

## What changes in warte

1. Remove the `artifacts` table, `src/artifact`, the unused `src/sigil` module, both routes, the dashboard panel, the overview count, `warte artifact`, and the docs topic. Replace them with a pointer.
2. New "Layers" view: per machine, the pinned realm/layer, manifest digest, verify verdict (exit code and time checked), status (yanked, support window), and payloads from `inspect --json`.
3. The probe reports these facts. warte already owns probe and history, and `src/probe/inventory.rs` already checks varve PATH shadowing. The server stores the reports and their history. It does not run varve on its own behalf.
4. Trace: supersede the storage half of DD-ARCH-004; mark REQ-ARTIFACT-001..003 and REQ-SIGIL-001 obsolete or rewrite them to describe the pointer; add a REQ for the Layers view.

## What varve must do

Answer the open questions below. At most, add `verify --json` or `verify --layer`. warte asks varve for no new subcommand beyond those.

## Seam

```seam
format: 1
seam: varve-warte/layer-verdict
kind: consumption
status: options
provider:
  repo: pulseengine/varve
  promise: "inspect --json fields and the exit-code table stay stable; additions only"
consumers:
  - repo: pulseengine/warte
    binding: "the probe runs varve inspect --json / verify / status --json and reports layer, manifest_digest, exit codes"
interface:
  - kind: file
    path: README.md
    symbols: ["varve"]
options:
  - {id: probe-cli, summary: "the probe on each machine shells out to the varve CLI and reports JSON plus exit codes; the server stores pointer and verdict history"}
  - {id: server-core, summary: "the warte server links varve-core and calls payload_status for the layer the warte host pins"}
  - {id: probe-core, summary: "the probe links varve-core (typed verdicts, no output parsing); the server stores the reports"}
recommendation: probe-cli
open_questions:
  - {q: "Is `inspect --json` plus the exit-code table the stable contract, or will verify get --json?", owner: pulseengine/varve}
  - {q: "Can verify check a named layer without a pin directory (verify --layer)?", owner: pulseengine/varve}
  - {q: "Is varve-core's consumer API semver-stable for an outside consumer (needed for probe-core)?", owner: pulseengine/varve}
  - {q: "Ad-hoc agent build outputs (not in a realm layer): is there a varve-verifiable form, or are they out of varve's scope?", owner: pulseengine/varve}
  - {q: "Does the artifacts table hold any rows on the operator's instance?", owner: pulseengine/warte}
```

The `interface` entry is a placeholder. When varve picks an option, point it at the file where the inspect JSON shape is defined.

Why probe-cli: the verdict that matters is the one on the machine that runs the tools, and `verify` checks only the host's platform. The CLI exit codes are varve's written, tested compatibility promise. warte gets no Rust dependency on varve's release cadence. server-core only answers for the warte host. probe-core is the better long-term form and becomes phase 4 once the API promise is confirmed.

## Phases (each ships alone)

1. **Retire the dead store.** Removes code with no production writer. Precondition: the operator confirms `SELECT count(*) FROM artifacts` is 0. The drop migration (016) runs behind a Rust pre-check that refuses to start if rows exist. warte embeds its migrations, and a dropped table cannot be restored. This phase does not depend on varve. Hold it per the owner's decision unless the owner releases it early.
2. **Probe reports the layer verdict.** Extend the inventory report with `{realm, layer, manifest_digest, verify_exit, status_exit, checked_at}`. The server stores it with history. Needs the seam `decided`.
3. **Layers view.** Panel and `warte layers [--format json]`. Shows drift: a machine pinned to an older layer, verify failing, yanked.
4. **Optional: probe-core.** Swap shell-out for `payload_status` once varve confirms the API promise.

## Risks

- **Irreversible drop.** Mitigated by the row-count precondition and the refusing pre-check. Phase 1 deletes nothing that anyone wrote.
- **False assurance.** A probe report is a claim, not a proof. A machine whose varve core someone can write reports "verified" while a planted binary is dispatched (varve threat model). The view must say "reported by the probe at T", never "verified by warte".
- **Platform blind spot.** `verify` skips other platforms, so each machine vouches only for itself. That is acceptable because every machine reports.
- **Trace churn.** Four `accepted` artifacts change status. `rivet validate` must pass in the same change.
- **Contract drift.** A varve release changes the JSON shape. Mitigation: the seam promise, and a warte test that pins the fields it reads.

## Kill criterion

Stop the pointer work (phases 2-4) if either holds. Phase 1 stands regardless.

- varve has not moved this seam to `decided` by **2027-01-31**.
- In the first two weeks of phase 2, the reported `verify_exit` disagrees with a hand-run `varve verify` on the same machine even once.

## Recommendation

Do phase 1 when the owner releases it: it removes about 500 lines that claim integrity they do not deliver. Propose `probe-cli` to varve now. Build phases 2-3 once varve answers the first two open questions.
