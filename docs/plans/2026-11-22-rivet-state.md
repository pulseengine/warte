# Plan: durable state from SQLite to rivet YAML (warte#37)

> **Status 2026-11-22.** The owner chose a written plan first. Phase 1's code removal shipped in v1.6.44 (src/sync, its routes and the crdts dependency); migration 016 dropped the sync_state and instances tables after the operator confirmed they were empty. Blocker B1 is filed as rivet#1328.


Status: draft for owner review, 2026-11-22. No code until the owner approves.
Grounded in `main` at v1.6.54 and rivet 0.41.1 (the pinned varve binary).
Related artifacts already in the tree: `DD-STORE-001` (proposed, supersedes
`DD-ARCH-002`) and `REQ-STORE-001` in `artifacts/dogfood-plan.yaml`.

## 1. Intent

Keep records that people decide on (who an agent is, what it may do, what was
accepted) in typed, validated, git-versioned rivet YAML, so the git log is the
audit trail and `rivet validate` is the gate. Keep everything high-churn,
secret or rebuildable in SQLite. Delete the CRDT code that YAML-in-git would
make redundant. Dogfood rivet as a backend, and report what it lacks.

## 2. What is in SQLite today (facts)

The issue says "migrations 001..004, ~52 queries, 10 modules". That is stale:
there are 15 migrations and **76 `sqlx::query*` call sites in 15 files**.
`ci_jobs`, `wait_history` and `ci_status` were already dropped by `005`
(CI telemetry is the JSONL ledger under `<data>/ledger/`, `src/hub/belt.rs`).
The job ledger and history are **not** in SQLite, so they are out of scope.

| Table (migration) | Owner (queries) | Write pattern | Notes |
|---|---|---|---|
| `agents` (001, 013) | agent 9, ghtoken 1, migrate 1 | every register re-issues a 24 h cert (`cert_pem`, `cert_expires_at`, `updated_at`); every heartbeat writes `last_heartbeat`/`heartbeat_status`; `mark_stale_agents` sweeps | identity and liveness share one row |
| `access_policies` (001) + `access_policy_tombstones` (002) | access 7, agent 1 | rare; API grant/revoke | OR-Set, tombstone wins. `AccessControl::check` is **never called** outside `src/access`; request authz comes from signed `warte1.` tokens (`src/auth`) |
| `agent_data` (001) | api 7, dashboard 6, agent 2, migrate 2, alerts 1 | append-only; types `friction`, `hardware-catalog`, `hardware-claim`, `schedule`, `notice.v1` (alerts), `answer.v1`, plus any type posted to the API | reads `LIMIT 5000` on notices |
| `artifacts` (001) | artifact 5 | blobs + integrity proofs | `DD-LAYER-002` already proposes moving this to varve |
| `sync_state` (001) | sync 4 | on `/api/sync/update` | see section 5 |
| `instances` (001) | none | none | dead table |
| `probe_tokens` (006), `probe_revocations` (007) | probe/enroll 4 | one-time token hashes; revocations rare | tokens are secrets (hashes), short-lived |
| `news_items` (008) | news 4 | feed polling; state found/decided/landed | mostly machine-written |
| `agent_github` (009, 010) | ghtoken 5 | operator CLI only, rare | per-agent token scope |
| `attention_schedules` (011) | attention 2 | per login, user-edited | keyed by IdP subject |
| `telemetry_sa` (012) | hub/quic 5 | `last_epoch` per handshake | holds `k_id` key material |
| `reports` (014) | reports 2 | latest per kind, overwritten | producer feed |
| `security_findings`, `security_reports`, `security_baseline` (015) | findings 8 | findings per host report; baseline by a person | baseline = human decision |

Migrations are immutable once released (`REQ-STORE-003`, SHA-384 pinned in
`warte::db::RELEASED_MIGRATIONS`), so every drop below is a new `016+`.

## 3. What rivet 0.41.1 can and cannot do (tested, not guessed)

Tested in a throwaway project in the scratchpad, never in the warte tree.

Can:
- **Custom types**: a project-local schema (`schemas/<name>.yaml`, listed in
  `rivet.yaml` `schemas:`) declared an `agent` type with an enum field and a
  `list<string>` field; `rivet add`/`modify` refused a value outside the enum
  before writing.
- **Write API (CLI)**: `add` (with `--id`, `--field`, `--file`), `modify`
  (incl. `--where` in one pass), `link`/`unlink`, `remove`, `batch <file>`;
  `rivet sql` can `UPDATE artifacts SET status|title|description` only.
  The MCP server has the same five mutations, stdio only.
- **Reads**: `list`/`get --format json`, `query`, `sql` (SELECT, JOINs).
  Fast: about 20-90 ms per call on warte's 458 artifacts.

Cannot (blockers or constraints):
- **B1, no write safety under concurrency (blocker).** 20 parallel `rivet add`
  on one file, three runs: 23 entries expected; got 16/22/23 entries with only
  3/5/5 unique IDs. In run 1 the `artifacts:` header itself was lost, the file
  loaded 0 artifacts, and **`rivet validate` returned PASS** (one warning).
  So rivet has no lock and no atomic replace, the next-id step races, and
  validate does not catch total loss. warte must be the single writer and
  serialise every call; this should also be filed in rivet.
- **B2, IDs must be `PREFIX-NNN`, uppercase.** `--id <uuid>` is refused. Agent
  ids are UUIDv7 today and appear in tokens (`agent_id` in the payload) and in
  `agent_data.agent_id`. Needs a mapping (rivet id + `uuid` field) or a token
  change.
- **B3, status is a fixed set** (draft, proposed, approved, implemented,
  verified, released, accepted, deprecated, rejected); `revoked` was refused.
  "Revocation = status change" means reusing `deprecated`/`rejected`, or a
  custom field. Whether a schema can redefine the lifecycle was not found in
  `rivet docs`; treat as unknown until asked in rivet.
- **B4, appending to `artifacts: []`** wrote a list item after a flow sequence
  and broke the file (the parse error was caught by the next `add`). Seed files
  must be written as `artifacts:` with no value.
- **No network write API**; the MCP server is local stdio. `rivet-core` is not
  published, so warte shells out to the pinned binary (today `src/rivetext`
  runs `rivet` from `PATH` with a fallback). Fine for one host, but it adds a
  runtime binary dependency that also has to exist in GitHub-only/work mode.
- **rivet does not commit or push.** The git audit trail exists only if warte
  (or the operator) commits after each write; that needs a repository and, for
  push, a credential on the host.

## 4. What moves, what stays, and why

Rule: a record moves only if it changes at human speed (under ~1 change a day
fleet-wide), is not secret, cannot be rebuilt from another source, and gains
something from review in git. Everything else stays.

Moves (candidates, in pilot order):
1. **Agent identity** (name, repo, identity type, public key, granted rights):
   changes when an agent is added or its scope changes. Not the cert, not the
   heartbeat.
2. **`agent_github`** scope: operator-set, rare, and an audit trail of who got
   `actions:write`/`workflows:write` is exactly what git history is good at.
3. **`security_baseline`**: each row is a person accepting a finding.
4. **`access_policies`**: rare, but see the caveat in section 8.
5. **Schedules and friction** (`agent_data` types `schedule`, `friction`) only
   if the per-repo `.warte/` work (#18) lands in rivet YAML; they are
   agent-written today, so they need the same single-writer path.

Stays in SQLite:
- Agent liveness and certs (`last_heartbeat`, `health_status`,
  `heartbeat_status`, `cert_pem`, `cert_expires_at`): every heartbeat and every
  re-registration (24 h certs, `DD-ARCH-003`) writes them. In git that is at
  least one commit per agent per day for the cert alone.
- `agent_data` notices and answers: append-only, machine-written, read in
  batches of 5000.
- `probe_tokens`, `telemetry_sa`: secrets or key material; never in git.
- `probe_revocations`: rare, but on the authentication path and must work
  when git or rivet is broken; keep it, optionally mirror to YAML for audit.
- `news_items`, `reports`, `security_findings`, `security_reports`: machine
  feeds, overwritten or polled, rebuildable from their producers.
- `attention_schedules`: per-login personal data keyed by IdP subject; a git
  log of it is a privacy cost with little gain.
- `artifacts`: belongs to varve (`DD-LAYER-002`), not rivet.

Where the YAML lives: **not in the public warte repo** (agent names, repos and
scopes are operator data). A git repository in warte's data directory, owned
by warte, pushed (if at all) to a private remote the operator chooses.

## 5. CRDT sync: what it actually is today

`src/sync/mod.rs` (163 lines) exposes `/api/sync/state` and
`/api/sync/update`. `apply_remote_update` only merges version vectors and has a
`TODO: apply the update to the local database`; nothing in `src/` sends a
`SyncUpdate`. So **no record is synced between instances today**: the sync is
a stub, `instances` is dead, and the only working CRDT piece is the OR-Set in
`src/access`, which works locally. `REQ-SYNC-001..003` are `accepted`, not
implemented.

Effect of the move: for the moved records, git (pull/push between instances)
replaces the planned CRDT. Git merges text, not sets: a concurrent grant on one
instance and revoke on another becomes a merge conflict, not "tombstone wins".
The plan keeps fail-safe by rule: on any conflict or failed pull, the instance
treats the record as revoked/unknown and alerts. With one writer instance
(the expected deployment) conflicts cannot happen.

Independently of #37: the sync endpoints and `sync_state`/`instances` can be
removed now as dead code, with `REQ-SYNC-*` set to deprecated by a decision.

## 6. Migration path (each phase ships alone, each has a rollback)

Phase 0, decisions and rivet issues (no warte code). New DD superseding
`DD-ARCH-002` (amend `DD-STORE-001`); file B1-B3 in rivet. Rollback: none
needed. Exit: owner approves this plan; B1 has an answer (fix or "warte is the
single writer" accepted).

Phase 1, delete the dead sync. Remove `src/sync`, the two routes, the auth and
contract entries; migration `016` drops `sync_state` and `instances`.
Rollback: revert the release; a dropped empty table is recreated by reverting
016 in a new 017. Independent of rivet; worth doing even if #37 stops here.

Phase 2, pilot: agent identity in YAML, SQLite still authoritative (shadow).
Add `schemas/warte-state.yaml` (type `agent`, ids `AGENT-NNN`, field `uuid`)
in the data repo; a single writer task in warte serialises every rivet call
(one mutex, one process at a time) and commits after each write. Every
register/rm also writes YAML; a nightly check compares YAML to SQLite and
`rivet validate`s it, alerting on any difference. Reads stay on SQLite.
Rollback: switch the shadow off by config; SQLite was never touched.

Phase 3, flip identity reads to YAML. Registry reads go to an in-memory cache
built from `rivet list --format json`, reloaded after each write; liveness and
cert columns stay in `agents`. Decide (owner) whether registration of an
unknown name still auto-creates an identity, or whether YAML becomes the
allow-list the operator edits. Rollback: config flag back to SQLite reads; the
shadow writes kept SQLite current.

Phase 4, `agent_github` and `security_baseline`, same shadow-then-flip steps.

Phase 5, access policies and the OR-Set. Only if section 8's caveat is
settled. Migration drops `access_policy_tombstones` and `access_policies`
after one release of shadow parity. Rollback: restore from the YAML into a
re-created table (an export command written in phase 5, tested before the drop).

## 7. Gains and losses

Gains: reviewable diffs and a git log for who granted what; schema-enforced
fields (enum refusal tested); fewer bespoke tables; the dead sync and the
OR-Set code go away (`src/sync` 163 lines, most of `src/access` 303); rivet gets a real
backend user, which surfaces bugs like B1 early.

Losses: a runtime dependency on an external binary and on a git repository on
the host; a write becomes a process spawn plus a commit (tens of ms, plus git);
no transactions across YAML and SQLite (registry and cert can disagree after a
crash; the nightly parity check is the guard); fail-safe set semantics become
a rule we enforce instead of a data structure; another place for operator data
that must never reach the public repo.

## 8. Risks

- R1 data loss from concurrent writers (B1, reproduced). rivet writes the
  file itself, so warte cannot make the write atomic. Mitigation: one writer
  task, strict serialisation, and after each write both `rivet validate` and
  a check that the artifact count did not drop (validate alone passed on loss).
- R2 access policies are a record, not enforcement: `check` is unused. Moving
  them gives audit value but no security change; mis-selling it as a security
  improvement is the risk. Decide first whether they should be enforced at all.
- R3 ID mismatch (B2) leaks into tokens and `agent_data` joins.
- R4 a broken YAML file (B4, or a bad hand edit) takes the registry down.
  Mitigation: keep the last good cache in memory and refuse to load a file
  that fails validate or shrinks.
- R5 git push credentials on the host widen what a compromise can reach.
- R6 work/GitHub-only mode lacks the varve binary; warte must run without it.

## 9. Kill criterion (falsifiable)

Stop and keep SQLite for the registry if, after the phase 2 shadow has run
for 14 days, **any** of these holds:
- one or more nightly parity mismatches not explained by a warte bug fixed
  within the window;
- any rivet write that leaves `rivet validate` failing or the artifact count
  lower than before the write;
- p95 latency of `POST /api/agents/register` rises by more than 200 ms over
  the SQLite baseline measured in the week before;
- B1 has no rivet-side fix or accepted single-writer rule by end of phase 2.

## 10. Pilot

The agent registry, identity fields only, in shadow mode (phase 2). It is the
right pilot because it is small, rarely written, already has `REQ-STORE-001`,
and exercises every blocker (B1 writes, B2 ids, B3 status for `rm`). It is
the wrong pilot for proving sync, because the sync it would replace does not
exist yet.

## 11. Recommendation

- **Do phase 1 now**, independent of rivet: the CRDT sync is a stub and the
  `instances` table is dead; delete them by a decision.
- **Do not move** liveness, certs, notices, tokens, `telemetry_sa`, probe
  revocations, news, reports, findings or attention schedules. They are
  high-churn, secret, on the auth path or rebuildable; git adds cost there.
- **Move only human-decision records**, starting with the agent identity pilot
  in shadow mode, then `agent_github` and `security_baseline`.
- **Hold access policies** until the owner decides whether they are enforced;
  today they are an unused record.
- **Blocked until rivet answers B1** (no write lock, silent loss passes
  validate); B2 and B3 shape the schema and need a decision before phase 2.
