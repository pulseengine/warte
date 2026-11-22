# Plan: sign warte releases with sigil, ship warte via the varve layer (warte#53)

> **Status 2026-11-22.** The owner chose plans and seam options first. Phase 1's pin bump shipped in v1.6.44; release.yml is unchanged until sigil#354 ships.


Status: plan plus seam options. Owner decision 2026-11-22: sigil and varve agree the seams first; warte's side is built after that.

## Intent

Every warte binary a machine runs has two properties. It is signed by our own tool (sigil/wsc), and it was installed from a digest-pinned, rollback-protected varve layer. The operator's deploy installs from the layer and refuses a binary that does not verify. Today's chain stays the safety net until the new one has proven itself: cosign over `SHA256SUMS.txt`, and GitHub build-provenance attestations.

## Facts (checked 2026-11-22)

- **warte is already in the realm.** The realm repo pulseengine/pulseengine-layers has a `layer.toml` entry for warte, added 2026-11-08: `version = "v1.6.38"`, raw per-platform binaries, ingested under the cosign-signed `SHA256SUMS.txt`. Installed layer `2026.11.23` carries warte **1.6.29** for aarch64-apple-darwin, aarch64-linux and x86_64-linux, and `varve inspect --json` shows `ingest_proof: "cosign-sums"`. warte has no x86_64-apple-darwin build. The issue's finding ("no warte entry", layer 2026.10.30) is out of date.
- **What is still missing for REQ-LAYER-001:**
  - warte's own `varve.toml` pins `2026.10.30`, so `varve which warte` in this repo fails ("not part of layer 2026.10.30").
  - The layer lags the release: v1.6.38 in the realm vs v1.6.54 tagged. Someone bumps the realm entry by hand.
- **REQ-SIGN-001 cannot be met as written.**
  - wsc 0.12.2 (the realm's current version) `sign --format` accepts `wasm | elf | mcuboot`.
  - `wsc sign --keyless --format elf` refuses: "Keyless signing is currently supported only for WASM format. Use key-based signing for ELF and MCUboot artifacts." I reproduced this locally on 2026-11-22. It is tracked open as sigil#354.
  - There is no Mach-O and no detached-blob format (sigil#330, open).
  - `wsc attest` takes "Input WASM file", so it does not apply to native binaries. `wsc build-env --json [--from-env]` captures the build environment "for embedding in SLSA provenance".
  - sigil's own release never self-signed its ELF binaries; sigil#352 made that refusal visible.
- **release.yml** runs on GitHub-hosted runners (ubuntu-24.04, ubuntu-24.04-arm, macos-latest) and must stay there.
  - It builds three binaries, sums them with the SBOM and operator kit into `SHA256SUMS.txt`, and cosign-signs that file (`SHA256SUMS.txt.cosign.bundle`). varve's assembler looks for exactly these names.
  - It also attests provenance for `dist/warte-*`.
- **Unknowns** (questions below, not guesses):
  - Does ELF signing embed into the binary, changing its bytes, or write a detached file with `-S`?
  - Can varve's assembler record or check a sigil signature as an ingest proof?
  - Which certificate identity does the assembler require on the cosign bundle?

## What changes in warte

1. **Pin** (`varve.toml`): move to a layer that carries warte. Re-point REQ-LAYER-001's evidence at `varve which warte`.
2. **release.yml, additive only.** Add a `sign-sigil` job after `build` and before `checksums`/`attest`:
   - Install wsc, verified: from the varve layer, or a sigil release checked against its cosign-signed sums.
   - Sign the two Linux ELF binaries keylessly with the job's GitHub OIDC identity.
   - **Verify them in the same job**: `wsc verify --keyless --format elf --cert-identity <this workflow's ref> --cert-oidc-issuer https://token.actions.githubusercontent.com`.
   - Fail closed: no `continue-on-error`, no `|| echo` (the sigil#352 lesson).
   - If signing embeds, `checksums` and `attest` must run over the signed bytes. Order: build, then sign, then attest and sum.
   - Also record `wsc build-env --json` and publish it as a release asset.
3. **Operator kit.** `deploy/operator` gets a documented install-and-verify recipe: `varve install`, `varve verify`, `varve status` (exit 3 means yanked), `varve which warte`, plus a `wsc verify` once phase 2 ships. The operator's own deploy adopts it. warte ships the recipe and the operator runs it.
4. **Trace.**
   - Reword REQ-SIGN-001 to match what wsc can do, and drop `wsc attest` from it.
   - Add a REQ: the layer carries the latest release within N days.
   - Link DD-SEC-002.

## What the other repos must do

- **sigil:** ship keyless ELF (sigil#354), and later Mach-O and detached blobs (sigil#330). Answer the embed-or-detach question and name the verify recipe and identity format.
- **varve / realm:**
  - Say whether a sigil signature can be an ingest proof alongside `cosign-sums`.
  - Name the cosign identity the assembler checks.
  - Decide whether realm bumps for warte are automated (release event to a reviewed `layer.toml` PR) or stay manual.

## Seams

```seam
format: 1
seam: sigil-warte/release-signature
kind: identity
status: options
provider:
  repo: pulseengine/sigil
  promise: "wsc sign/verify --keyless --format elf with GitHub OIDC; flag names and identity semantics stable"
consumers:
  - repo: pulseengine/warte
    binding: "release.yml signs and verifies the Linux binaries keylessly on GitHub-hosted runners"
interface:
  - kind: file
    path: src/cli/main.rs
    symbols: ["keyless"]
options:
  - {id: keyless-elf, summary: "wait for sigil#354; sign ELF keylessly with the release job's OIDC identity; no key custody"}
  - {id: managed-key, summary: "key-based ELF signing with a release key held as a repo secret; works today; custody and rotation burden"}
  - {id: blob-sums, summary: "sigil signs SHA256SUMS.txt as a detached blob (sigil#330); covers all three platforms, Mach-O included, in one signature"}
recommendation: keyless-elf
open_questions:
  - {q: "Does --format elf embed the signature (changing bytes) or write only a detached -S file?", owner: pulseengine/sigil}
  - {q: "Release target for #354 keyless ELF, and for #330 macho/blob?", owner: pulseengine/sigil}
  - {q: "The exact --cert-identity a consumer should pin for a GitHub workflow signer?", owner: pulseengine/sigil}
```

```seam
format: 1
seam: varve-warte/layer-ingest
kind: consumption
status: options
provider:
  repo: pulseengine/varve
  promise: "the assembler ingests warte-<v>-<os>-<arch> under SHA256SUMS.txt + .cosign.bundle; the ingest_proof field names how"
consumers:
  - repo: pulseengine/warte
    binding: "release.yml asset names and sums file stay exactly as the assembler reads them"
interface:
  - kind: file
    path: README.md
    symbols: ["deposit"]
options:
  - {id: cosign-only, summary: "keep cosign-sums as the sole ingest proof; sigil signatures travel as release assets only"}
  - {id: dual-proof, summary: "the assembler also checks the wsc signature and records it (e.g. ingest_proof cosign-sums+wsc-elf)"}
  - {id: attestation, summary: "bind the wsc signature or build-env as a sign-attestation (kind provenance) to the layer"}
recommendation: cosign-only
open_questions:
  - {q: "Which cosign certificate identity does the assembler require for warte?", owner: pulseengine/varve}
  - {q: "Can warte's release trigger a reviewed layer.toml bump in the realm repo, so the layer stops lagging?", owner: pulseengine/varve}
  - {q: "Would dual-proof fail the whole deposit when one tool's wsc check fails?", owner: pulseengine/varve}
```

Recommendations:

- **keyless-elf**: the release already has the OIDC identity, and keyless avoids a long-lived secret.
- **cosign-only** first: the layer keeps its proven ingest path while the sigil signature proves itself. Revisit `dual-proof` after phase 2's soak.
- The `interface` paths are placeholders until each provider names the defining file.

## Phases (each ships alone)

0. **Pin and trace (warte only, now).** Bump `varve.toml` to a layer that carries warte, and correct REQ-LAYER-001's evidence. This touches no signing path.
1. **Layer freshness.** Once varve answers the bump question, warte shows the realm's warte version against the latest tag, and alerts when the lag passes N days.
2. **sigil signing, additive.** Requires sigil#354 released and in the realm. Add the `sign-sigil` job with its in-job verify. Rehearse first on a `workflow_dispatch` pre-release. cosign and the attestations stay unchanged.
3. **Install from the layer.** The operator kit's recipe; the operator switches its deploy. Self-update of the probe comes later and uses the same refusal rule.
4. **macOS.** When sigil#330 lands, sign Mach-O, or the sums as a blob, and decide whether cosign retires. Cosign retires only by a separate owner decision.

## Risks

- **A broken signature blocks every consumer.** Mitigations:
  - Phase 2 is additive.
  - Consumers keep verifying cosign and attestations until wsc verification has passed on three consecutive releases.
  - Each signature is verified inside the job that made it, so a bad one never publishes.
- **Embedding changes bytes.** It would silently invalidate attestations and sums computed earlier, so job order is a hard requirement once the embed question is answered.
- **A step that cannot fail.** sigil's own release masked the refusal twice. Every new step exits non-zero on refusal, and `tests/ci_workflows.rs` should assert there is no `continue-on-error` on signing jobs.
- **Supply chain of the signer.** wsc must be pinned and verified on the hosted runner, never fetched unpinned. Self-hosted runners must not be used.
- **managed-key fallback.** A leaked key signs anything. Avoid it unless the owner accepts custody.

## Kill criterion

Drop sigil binary signing and reword REQ-SIGN-001 to "cosign-signed sums plus GitHub provenance" if either holds:

- sigil#354 is not in a released wsc by **2027-03-01**.
- After phase 2, `wsc verify --keyless` of a published warte binary fails for any consumer (operator deploy, a fresh machine, the varve assembler) on two consecutive releases.

## Recommendation

Do phase 0 now. Post both seam blocks, recommending `keyless-elf` and `cosign-only`. Do not touch release.yml until sigil#354 ships and its embed-or-detach answer is known. The blocker is sigil, not warte or varve.
