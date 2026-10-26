# Distributed Database Options for Multi-Instance State Synchronization

**Project:** warte — the PulseEngine control tower
**Date:** 2026-10-26
**Status:** Final
**Scope:** Research only. No code changes.

---

## 1. Context and requirements

### 1.1 What warte synchronizes

warte runs on multiple instances (today: Hetzner primary + Netcup secondary, behind an
edge proxy, connected by a private WireGuard VPN — see `docs/architecture.md` and
`docs/deployment.md`). Each instance keeps a local SQLite database
(`src/db/migrations/001_initial.sql`). The state that must sync across instances:

| Table | Content | Write pattern | Conflict sensitivity |
|---|---|---|---|
| `agents` | Agent registrations (id, name, repo, public key, cert, health) | Low rate; instance-local writes, replicated | Medium — metadata updates can conflict; re-registration must be idempotent |
| `access_policies` | Access rights (agent × resource × action, granted/revoked) | Low rate, operator-initiated | **High — a lost or overwritten revocation is a security incident** |
| `artifacts` | Artifact metadata + content + integrity proofs | Low rate, operator/agent uploads | Medium — content is addressed by id; metadata can conflict |
| `agent_data` | Captured agent data (session context, learnings, friction reports) | **High rate, append-mostly**, partitionable by agent | Low — appends keyed by UUIDv7; deletes rare |
| `ci_status` | CI status cache for dashboard ETA | Medium rate, derived data | Low — rebuildable cache |
| `instances`, `sync_state` | Instance registry, version vectors | Low rate | Low |

Properties of the workload that drive the decision:

- **Small authoritative state, high read locality.** Agents, policies, and artifact
  metadata are thousands of rows. Captured data grows but is append-mostly and
  partitionable by agent.
- **Writes are rare and bursty** (registrations, grants/revokes, artifact uploads),
  except captured data, which is a steady stream of appends.
- **Two instances.** Not ten, not a hundred. Any quorum math must work with n=2
  (or n=3 if a witness is added).
- **Instances sit on a private WireGuard mesh** with per-instance mTLS certificates
  already issued by warte's CA (`docs/architecture.md`, agent registry).

### 1.2 Requirements derived from the use case

From `docs/architecture.md` (security model, revocation flow) and the STPA-Sec threat
model (`artifacts/security.yaml`):

| # | Requirement | Source |
|---|---|---|
| R1 | **Availability during partition (AP).** Both instances keep serving reads and accept writes when the link between them is degraded. A control tower must stay operable exactly when the network is not. | Architecture: multi-instance HA |
| R2 | **Eventual consistency with fast convergence.** Divergence detected and alerted (REQ-SYNC-003); revocation propagation target < 5 s, push-based. | Architecture: access control service |
| R3 | **Fail-safe conflict resolution for authorization.** Concurrent grant/revoke must resolve toward *deny*, never toward *allow*. A lost revocation is a security incident. | TS-003 (revocation delay exploitation) |
| R4 | **No split-brain, ever.** Two instances must never hold divergent state that cannot be reconciled automatically. | Architecture: divergence detection + alerting |
| R5 | **No loss of acknowledged writes.** A write acknowledged to an operator (e.g. "access granted") must survive instance crash and propagate. | General durability expectation |
| R6 | **Operational simplicity.** Two small VPS (2 vCPU / 4 GB). No DBA. Deployment, monitoring, backup must be scriptable and boring. | `docs/deployment.md` |
| R7 | **Security.** mTLS between instances (exists), integrity proofs on sync payloads (CG-006), encryption at rest desirable — the SQLite file holds agent public keys, certs, captured data. | TS-006 / CG-006 |
| R8 | **License compatibility.** warte is MIT. Dependencies must be MIT/Apache-2.0/BSD-compatible; BUSL/LGPL need scrutiny. | `Cargo.toml` |
| R9 | **Keep SQLite.** The codebase (sqlx), migrations, CLI, and migration paths (smithy/temper import) are built on SQLite. Replacing the engine is a rewrite, not a swap. | `Cargo.toml`, `src/db/` |

### 1.3 Non-goals

- Multi-region active-active with single-digit-ms write latency. Two datacenters;
  5–50 ms cross-DC latency is fine.
- Massive scale. Hundreds of agents, not millions.
- Strong consistency as a hard requirement. Nothing in the requirements demands
  linearizable reads across instances; the dashboard already shows freshness
  indicators (REQ-DASH-001).

---

## 2. Consistency models primer

A shared vocabulary, because every option below claims a different point on this
spectrum [8]:

| Model | Guarantee | Typical mechanism |
|---|---|---|
| **Eventual** | If no new updates are made, eventually all replicas return the same value. No time bound. [4] | Anti-entropy, read repair |
| **Causal** | Reads respect happens-before: if write A causally precedes write B, no replica sees B without A. | Version vectors / vector clocks [9] |
| **Convergent (strong eventual)** | Any two replicas that have seen the same set of updates have identical state. The defining CRDT property: state is a join-semilattice, merge is the join. [3] | CRDTs (CvRDT/CmRDT) |
| **Sequential** | All replicas agree on *some* total order of writes, but not necessarily real-time order. | Single leader / consensus |
| **Linearizable** | Reads/writes appear to execute atomically at a point in real time; a read never returns a value older than a completed write. [7] | Consensus (Raft), quorum reads |
| **Strict serializable** | Transactions appear to execute in a serial real-time order. The strongest model. [23] | Optimistic concurrency + total order (FoundationDB) |

CAP [5] and PACELC [6]: during a **P**artition you choose **A**vailability or
**C**onsistency; **E**lse (no partition) you trade **L**atency for **C**onsistency.
Every option below is a point on this tradeoff surface. warte's requirements (R1, R2)
sit on the AP side; R3 is about *which* value wins when concurrent writes collide,
which is a merge-semantics question, not a consensus question.

---

## 3. Options

### 3.1 SQLite + CRDT (current approach)

**How it works.** Each instance keeps a full local SQLite database. The sync service
(`src/sync/mod.rs`) tracks causality with **version vectors**, merges commutative
state with **CRDTs**, resolves non-commutative state with **last-writer-wins**
(version-vector tiebreak), and detects divergence when version-vector distance grows.
Sync runs over the existing mTLS HTTP endpoints (`/api/sync/state`, `/api/sync/update`).
The `crdts` crate (v7) is already a dependency in `Cargo.toml`.

- **Consistency.** Convergent (strong eventual) for commutative state; LWW — a form
  of eventual consistency with deterministic last-writer resolution — for
  non-commutative state; version vectors give causal tracking (who is behind whom).
- **Failure modes.**
  - *Partition:* both instances keep accepting writes; merges converge on heal. Fully AP.
  - *Split-brain:* impossible by design — CRDT merge is commutative, associative, and
    idempotent [3]. There is no leader to disagree with itself.
  - *Data loss:* only writes made on an instance that crashes *before any peer
    receives them* are lost. Bounded by the sync interval; mitigated by WAL
    durability, anti-entropy on reconnect, and an optional Litestream backup.
  - *Known weaknesses of the current implementation:* LWW uses wall-clock
    timestamps (vulnerable to NTP skew between Hetzner and Netcup); no tombstone
    garbage collection; no periodic anti-entropy sweep (a peer that was down at
    write time only catches up when it reconnects and pulls); `apply_remote_update`
    is still a TODO — the merge logic is not implemented yet.
- **Operational complexity.** Lowest of all options. No new processes, no new
  daemons. Backup = SQLite online backup per instance (plus Litestream, optional).
  Monitoring = divergence detection (already in place) + sync lag metrics.
- **Security.** mTLS channel (existing rustls infrastructure, per-instance certs);
  payload integrity via sigil signatures (CG-006); encryption at rest via SQLCipher
  or an encrypted volume (optional).
- **License.** MIT (warte) + MIT/Apache-2.0 (`crdts` crate) [35].
- **Fit.** Matches R1–R9. The gaps are implementation gaps, not architecture gaps.

### 3.2 Custom gossip protocol over WireGuard (evolution of 3.1)

**How it works.** Keep SQLite + CRDT storage, replace/augment the HTTP push sync
with **gossip**: SWIM-style membership and failure detection [11] plus
Scuttlebutt-style anti-entropy state exchange [12] carrying CRDT updates, running
over the existing WireGuard mesh. Rust ecosystem: `chitchat` (Quickwit's Scuttlebutt
implementation with a phi-accrual failure detector, Apache-2.0) [37], `al8n/memberlist`
(SWIM in Rust), or a hand-rolled anti-entropy sweep on top of the version vectors
that already exist. Epidemic/gossip dissemination gives O(log n) convergence with
randomized peer selection [10].

- **Consistency.** Same as 3.1 (convergent/eventual); gossip improves convergence
  speed and removes the dependency on a peer being reachable at write time.
- **Failure modes.**
  - *Partition:* fully AP — writes are local; anti-entropy heals any partition,
    however long.
  - *Split-brain:* impossible (same argument as 3.1).
  - *Data loss:* same as 3.1; message loss is handled structurally by periodic
    anti-entropy rounds, not by hoping the peer was up.
- **Operational complexity.** Low–moderate. In-process, no new daemons. The cost is
  that you own the protocol: versioning, testing, monitoring. With two instances the
  protocol can be deliberately simple (full-state or delta exchange on a timer plus
  on-write push).
- **Security.** WireGuard transport (Noise protocol, Curve25519, per-peer keys) plus
  the existing mTLS layer and signed payloads.
- **License.** Own code — MIT.
- **Fit.** This is not a new database; it is the recommended *evolution* of the
  current approach (see §8.2).

### 3.3 LiteFS

**How it works.** A FUSE filesystem that replicates SQLite at the page level: it
intercepts WAL writes, packs page sets into LTX files, and streams them to followers.
A single **primary** is elected via a Consul lease; followers hold a full local copy
and serve reads locally. Replication is **asynchronous**. [14][15]

- **Consistency.** Writes are strongly consistent *through the single primary*;
  follower reads are eventually consistent (stale reads possible; no read-your-writes
  on followers). Single-writer.
- **Failure modes.**
  - *Partition / primary failure:* the primary's unreplicated writes are lost on
    failover (async replication); a new primary is elected after lease expiry —
    which requires Consul to be up.
  - *Split-brain:* transiently possible. LiteFS detects divergence via a rolling
    checksum of the full database and heals by **re-snapshotting the out-of-sync node
    from the new primary — discarding the old primary's unreplicated writes** [14].
    That is a designed-in data-loss event (violates R5 for those writes).
  - *Availability:* the minority side cannot write (violates R1).
- **Operational complexity.** High, and the project is **effectively unmaintained**:
  Fly.io's docs state "LiteFS is outside our scope of support" [14]; LiteFS Cloud was
  sunset in late 2024 [16]; the last upstream commit is from April 2025 [15].
  Linux-only (FUSE). Requires Consul for leases — a second distributed system to run.
- **Security.** TLS optional; coordination security delegated to Consul; encryption
  at rest only at the filesystem level.
- **License.** Apache-2.0 [15].
- **Fit.** **Rejected.** Unmaintained, single-writer (violates R1), adds a Consul
  dependency, and its split-brain healing discards writes (violates R5).

### 3.4 rqlite

**How it works.** Distributed SQLite over **Raft**: writes go through the elected
leader and are replicated to followers; followers serve reads with configurable
consistency — `None`, `Weak` (default), `Linearizable`, `Strong` [17]. rqlite has been
through Jepsen-style testing (linearizable record insert/modify with strong reads)
[20][21]. Hot backups to S3/MinIO/GCS are built in. TLS for the HTTP API and
node-to-node traffic, optional mTLS, basic auth [18]. Non-voting read replicas are
supported. MIT licensed [19].

- **Consistency.** Strong: writes are serialized through the Raft leader;
  linearizable reads available (`strong`/`linearizable` levels).
- **Failure modes.**
  - *Partition:* CP — the minority side cannot commit writes (violates R1 for that
    side).
  - *Split-brain:* impossible (Raft elects at most one leader).
  - *Data loss:* committed writes survive (quorum replication); uncommitted writes
    on a failed leader are lost.
  - **The two-node problem:** with 2 voting nodes, quorum = 2. One instance down or
    partitioned means **no quorum, no writes anywhere**. rqlite recommends odd-sized
    clusters; a 2-node deployment tolerates zero failures. Fixing this means adding a
    third (lightweight) voter — more infrastructure for less availability than the
    current design.
- **Operational complexity.** Moderate. A Go sidecar daemon per instance (warte is
  Rust — rqlite would be a separate process behind an HTTP API), 3-node cluster
  recommended, backups built in. Heavier than 3.1/3.2 but well-trodden.
- **Security.** TLS + optional mTLS (HTTP and node-to-node) + basic auth [18]; no
  encryption at rest (encrypted filesystem recommended).
- **License.** MIT [19].
- **Fit.** **Strong contender — the named fallback.** The best "adopt" option if
  strong consistency ever becomes a hard requirement. Costs: sidecar daemon, all
  writes funnel through one leader (fine at warte's scale), the 2-node quorum
  problem, and a leader/follower write path instead of peer-to-peer.

### 3.5 dqlite

**How it works.** Canonical's embedded C library: SQLite + Raft, used by LXD and
MicroK8s [22]. Same Raft tradeoffs as rqlite (single leader for writes, quorum for
commit), but embedded in-process rather than a sidecar.

- **Consistency / failure modes.** Same as rqlite (strong via Raft; CP; no
  split-brain; committed writes survive quorum; same 2-node quorum math).
- **Operational complexity.** Moderate — no sidecar, but a C library means FFI from
  Rust, and the operational surface (gateway, voting members) still exists.
- **Security.** TLS for node-to-node; at-rest via filesystem.
- **License.** LGPLv3 with a static-linking exception [22] — needs scrutiny under R8.
- **Fit.** **Rejected for now.** FFI friction, same quorum math as rqlite, weaker
  Rust story. Revisit only if an embedded (non-sidecar) consensus SQLite is needed.

### 3.6 FoundationDB

**How it works.** Apple's ordered key-value store with **strictly serializable** ACID
transactions [23] — the strongest consistency model available. "Unbundled"
architecture: a sequencer grants a total order, resolvers handle conflict detection,
proxies commit, log servers replicate, storage servers shard the data [24].

- **Consistency.** Strict serializability [23].
- **Failure modes.** CP; no split-brain; committed transactions are durable.
  Designed for clusters of tens to hundreds of nodes with redundancy modes
  (single/double/triple).
- **Operational complexity.** **Very high.** Not embeddable — you run a cluster of
  `fdbserver` processes (cluster controller, proxies, resolvers, log servers, storage
  servers) with coordination, monitoring, and tuning [24][25]. For two instances this
  is absurd overkill. Rust bindings exist (`foundationdb` crate, a safe async wrapper
  over the C API; requires `libfdb_c` installed; bindings merged into the upstream
  repo) [25].
- **Security.** TLS with client certificates; at-rest encryption via the storage
  layer / disk encryption.
- **License.** Apache-2.0.
- **Fit.** **Rejected.** Would replace SQLite entirely (violates R9), imposes a
  large operational burden (violates R6), and its power — strictly serializable
  transactions at massive scale — is unused at warte's scale.

### 3.7 etcd

**How it works.** CNCF-graduated distributed key-value store built on Raft.
Linearizable reads by default (serializable optional), watch API, leases [27].
Jepsen analysis of etcd 3.4.3 found no prohibitive safety issues (it noted a
read-path liveness issue — reads served by a leader without checking for a newer
leader — since addressed) [28][29].

- **Consistency.** Linearizable (Jepsen suggested the more precise term "strict
  serializability") [28][29].
- **Failure modes.** CP — partition makes the minority unavailable; no split-brain;
  committed writes durable across quorum. Same 2-node quorum math as rqlite.
- **Operational complexity.** Moderate — 3-node cluster recommended, single static
  binary, very mature monitoring. But it is a **KV store**: warte's relational schema
  (foreign keys, joins, migrations) would need remodeling into flat keys — a data-model
  migration, not a drop-in (violates R9 in spirit).
- **Security.** TLS + client certificates for both client and peer traffic; RBAC
  authentication [26]. **No encryption at rest** — the docs are explicit: "etcd doesn't
  encrypt key/value data stored on disk drives"; use dm-crypt or client-side
  encryption [26].
- **License.** Apache-2.0.
- **Fit.** **Rejected as the state store** (KV shape mismatch, quorum math, no at-rest
  encryption). Reasonable as a *coordination* sidecar (leader election, distributed
  locks) if such a need ever appears — warte does not have one today.

### 3.8 Consul

**How it works.** Service discovery + KV + service mesh. Raft for the KV/catalog
(server agents) [30]; SWIM-based gossip (Serf) for membership. Read consistency
modes: `default` (leader), `stale` (any server, possibly stale), `consistent`
(linearizable via Raft) [31]. Jepsen (2015) found that `consistent` reads could
return stale data from a node that considered itself leader — the same class of issue
as etcd's, since fixed [34].

- **Consistency.** Configurable; default reads may be stale [31].
- **Failure modes.** CP for KV writes (Raft quorum); gossip is AP for membership; no
  split-brain for KV. Same 2-node quorum math for the server set.
- **Operational complexity.** Moderate–high — server agents + client agents, gossip
  encryption, ACLs, WAN federation. Heavier than etcd for warte's needs.
- **Security.** mTLS, ACLs, gossip encryption — but **not secure by default** [32].
- **License.** **BUSL 1.1 since August 2023** (was MPL 2.0): source-available,
  converts to MPL 2.0 after four years, prohibits building a competing offering [33].
  Usable for warte, but a license-compatibility flag for an MIT OSS project (R8).
- **Fit.** **Rejected.** BUSL license, heavier than needed, KV shape mismatch, same
  quorum math.

### 3.9 Adjacent options (not full replacements)

- **Litestream** (Apache-2.0) [38]: streams a SQLite WAL to S3-compatible storage for
  durability and point-in-time restore of a **single-writer** database. Not
  multi-writer; not a sync mechanism between live instances. **Complements** the CRDT
  approach as a durability layer (R5): each instance backs up continuously; a
  destroyed instance is rebuilt from backup + anti-entropy catch-up.
- **Turso / libSQL** [39][40]: libSQL is an open-source SQLite fork. *Embedded
  replicas* keep a local read replica of a Turso **cloud primary** — writes go to the
  primary and reflect back (client/server, not peer-to-peer) [39]. *Turso Sync* is
  CDC-based, local-first sync with explicit `push()`/`pull()` [40]. Both tie warte to
  the Turso platform (managed cloud or self-hosted `sqld`) — a vendor dependency for
  a capability warte can build itself. Revisit only if "managed sync" (e.g. mobile or
  offline clients syncing directly) becomes a requirement.

---

## 4. Comparison table

| Approach | Consistency | Partition behavior | Split-brain | Data-loss risk | Ops complexity | Security (mTLS / at-rest) | License | Fit |
|---|---|---|---|---|---|---|---|---|
| **SQLite + CRDT** (current) | Eventual (convergent); causal tracking via version vectors; LWW for non-commutative state | **AP** — both instances keep writing; converge on heal | Impossible by design (commutative merges) | Only un-synced writes of a crashed instance (mitigate: WAL + anti-entropy + Litestream) | **Lowest** — in-process, no new daemons | mTLS (existing) + payload signing / SQLCipher optional | MIT + MIT/Apache-2.0 | **Recommended (build)** |
| **Gossip + CRDT over WireGuard** | Same as above; faster, structural convergence | **AP** | Impossible | Same as above | Low–moderate — in-process; own the protocol | WireGuard + mTLS + signing / SQLCipher optional | MIT (own code) | **Recommended evolution** |
| **LiteFS** | Strong via single primary; follower reads stale | CP for writes (primary only) | Transient possible; healed by discarding old primary's writes | Unreplicated primary writes lost on failover | High + **unmaintained** (Fly: out of support; Cloud sunset 2024) | TLS optional; relies on Consul / FS-level | Apache-2.0 | Rejected |
| **rqlite** | Strong/linearizable (writes via Raft leader; reads configurable) | CP — minority can't write | Impossible (Raft) | Committed writes survive quorum | Moderate — Go sidecar, 3-node recommended, backups built-in | TLS + optional mTLS + auth / none built-in (encrypted FS) | MIT | **Fallback** if strong consistency required |
| **dqlite** | Strong via Raft leader | CP | Impossible | Committed survive quorum | Moderate — embedded C lib (FFI from Rust) | TLS / FS-level | LGPLv3 + static-linking exception | Rejected (FFI, license) |
| **FoundationDB** | Strict serializable | CP | Impossible | Committed survive | **Very high** — full cluster, not embeddable | TLS + client certs / storage-level | Apache-2.0 | Rejected (overkill; replaces SQLite) |
| **etcd** | Linearizable (default); serializable optional | CP | Impossible | Committed survive quorum | Moderate — 3-node cluster, mature | TLS + client certs (client & peer) / **none built-in** | Apache-2.0 | Rejected as state store (KV shape, quorum) |
| **Consul** | Configurable (default/stale/consistent) | CP for KV writes; AP gossip for membership | Impossible for KV | Committed survive quorum | Moderate–high — servers + agents + ACLs | mTLS + ACLs + gossip encryption / none built-in | **BUSL 1.1** | Rejected (license, weight) |
| **Litestream** (adjunct) | n/a — single-writer durability | n/a | n/a | Bounded by WAL shipping interval | Low — sidecar | TLS to S3 / FS-level | Apache-2.0 | **Adopt** as durability layer |
| **Turso/libSQL Sync** (adjunct) | Eventual, CDC-based, local-first | AP | Impossible | Platform-dependent | Low if managed; medium if self-hosted | Platform TLS / platform | Open (verify current terms) | Revisit only if managed sync wanted |

---

## 5. Failure mode analysis

### 5.1 Network partition

- **AP options (SQLite+CRDT, gossip):** both instances remain fully writable; state
  converges after the heal. This is the decisive architectural argument for warte:
  with **two** nodes, a partition means *each side is a minority*. A Raft cluster of
  two has quorum = 2, so during any partition **neither side can commit writes** —
  the control tower goes read-only exactly when it is needed most (R1). CRDT/gossip
  keeps both sides writable and reconciles afterwards (R4).
- **CP options (rqlite, dqlite, etcd, Consul, FoundationDB, LiteFS-primary):** the
  minority side is unavailable for writes. With 3+ nodes this is the standard,
  acceptable trade — with 2 nodes it is not.

### 5.2 Split-brain

- **CRDT/gossip:** impossible. There is no leader; merges are commutative,
  associative, idempotent [3]. Two instances can never hold irreconcilable state.
- **Raft family (rqlite, dqlite, etcd, Consul, FDB):** impossible — quorum elects at
  most one leader per term.
- **LiteFS:** transiently possible (async replication + lease-based election).
  Detected via a rolling checksum; healed by re-snapshotting the divergent node from
  the new primary, **discarding the old primary's unreplicated writes** [14] — a
  designed-in data-loss event.

### 5.3 Data loss

- **CRDT/gossip:** an instance that crashes before any peer receives its updates
  loses those writes. Bounded by the sync interval; mitigated by (a) SQLite WAL
  durability, (b) anti-entropy sweep on reconnect, (c) Litestream backup per
  instance for disaster recovery.
- **Raft family:** committed writes survive (quorum replication); uncommitted writes
  on a failed leader are lost. etcd/Consul/FDB: same, FDB additionally offers
  point-in-time recovery.
- **LiteFS:** unreplicated writes on the primary are lost on failover (async
  replication) [14].

### 5.4 Slow or stalled convergence

- **Push-only CRDT sync (current):** a peer that is down at write time misses updates
  until it reconnects and pulls. Adding a periodic bidirectional anti-entropy sweep
  bounds worst-case propagation.
- **Gossip:** convergence is structural — periodic full/delta exchange rounds heal
  any partition regardless of when writes happened [10][12].
- **Raft systems:** convergence follows leader election + log replication; fast for
  committed writes, but only the leader's writes exist.

### 5.5 Compromised (Byzantine) instance

No option on this list is Byzantine-fault-tolerant. Plain CRDTs assume
non-malicious replicas: a compromised instance can inject arbitrary state (e.g. forge
a grant). The same is true of a compromised Raft leader. Practical mitigation for
warte: **per-update signatures** — each sync update carries the authorizing
operator's sigil signature; the apply path validates policy (instances relay,
operators originate). This closes the gap as far as is practical and aligns with
TS-006/CG-006 (sync channel injection → mTLS + integrity proofs).

---

## 6. Operational complexity

| Approach | Deployment | Monitoring | Backup |
|---|---|---|---|
| SQLite + CRDT | Nothing new — sync is in-process | Divergence detection (exists, REQ-SYNC-003); add sync lag + convergence metrics | SQLite online backup per instance; Litestream → S3 optional |
| Gossip + CRDT | Nothing new — in-process | Same + membership/failure-detector metrics, anti-entropy round duration | Same |
| LiteFS | FUSE mount per instance + **Consul cluster** for leases; Linux-only | LiteFS metrics; **project unmaintained** | Snapshots via LiteFS; restore = re-snapshot |
| rqlite | Go sidecar per instance; 3-node recommended | rqlite metrics (Raft state, lag); mature | Hot backups to S3/MinIO/GCS built in |
| dqlite | Embedded C lib (FFI); gateway process pattern | Raft metrics via dqlite API | SQLite-level backups |
| FoundationDB | Full `fdbserver` cluster (coordinator, proxies, resolvers, logs, storage) | Extensive; designed for dedicated ops | Snapshots + PITR; heavy |
| etcd | 3-node cluster, single binary | Very mature (Prometheus, etcdctl) | `etcdctl snapshot` |
| Consul | Server agents (3+) + client agents; gossip + WAN federation | Mature but broad surface | `consul snapshot` |

warte's ops budget (R6: two 2-vCPU VPS, no DBA) rules out FoundationDB outright and
makes the sidecar/cluster options (rqlite, etcd, Consul, LiteFS+Consul) a real cost.

---

## 7. Security

| Approach | mTLS between nodes | Encryption at rest | Notes |
|---|---|---|---|
| SQLite + CRDT / gossip | Yes — existing rustls mTLS, per-instance certs from warte CA; WireGuard underneath for gossip | Optional: SQLCipher extension or encrypted volume | Add per-update operator signatures (sigil) to close the Byzantine gap (§5.5) |
| LiteFS | Optional TLS; coordination via Consul | Filesystem-level only | Security delegated to Consul; unmaintained |
| rqlite | Yes — TLS for HTTP API and node-to-node; optional mTLS (`-http-verify-client`, `-node-verify-client`) [18] | None built-in; encrypted FS recommended [18] | Basic auth; dedicated non-root user recommended [18] |
| dqlite | TLS for node-to-node | Filesystem-level | — |
| FoundationDB | TLS with client certificates | Storage-layer / disk encryption | — |
| etcd | Yes — TLS + client certs for client **and** peer traffic [26] | **None built-in** — docs recommend dm-crypt or client-side encryption [26] | RBAC auth; not enabled by default [26] |
| Consul | Yes — mTLS + ACLs + gossip encryption [32] | None built-in (Vault integration typical) | **Not secure by default** [32] |

warte's existing posture already covers the transport layer (per-instance certs,
rustls, WireGuard). The CRDT approach adds nothing new to defend; the adopt options
mostly add new surfaces (etcd/Consul with no at-rest encryption, LiteFS delegating to
Consul, rqlite's second port for Raft traffic).

---

## 8. Recommendation

### 8.1 Build vs adopt: **Build.** Stay on SQLite + CRDT and evolve it.

1. **The workload is small, relational, and already in SQLite** (R9). Every adopt
   option either replaces SQLite (FoundationDB, etcd, Consul — KV shape) or wraps it
   in a consensus layer that changes the write path (rqlite, dqlite, LiteFS).
2. **Two instances kill the quorum math.** Any Raft-based option with 2 voting nodes
   tolerates **zero** failures — during any partition or single outage, writes stop
   cluster-wide (violates R1). Fixing that means adding a third voter: more
   infrastructure for less availability than the current design.
3. **AP matches the operational reality.** warte is a control tower; it must stay
   operable exactly when the link between instances is degraded. CRDT/gossip keeps
   both instances writable and converges after (R1, R2, R4).
4. **Fail-safe authorization is a design choice, not a database feature.** R3
   (revoke-wins) is implemented in the merge semantics — add-wins tombstones — which
   we control. A strongly consistent store does not make revocation safer; it makes
   it slower (leader round-trip) and still requires correct conflict semantics at the
   application layer. (Zanzibar solves the same class of problem with careful
   versioning, not with weaker consistency [41].)
5. **Ops budget.** R6: two 2-vCPU VPS. The current approach adds zero daemons.
   rqlite adds a sidecar per instance; etcd/Consul add 3-node clusters; FoundationDB
   adds a cluster of processes; LiteFS adds FUSE + Consul and is unmaintained.
6. **License hygiene.** MIT project; the `crdts` crate is MIT/Apache-2.0 [35].
   Consul's BUSL [33] and dqlite's LGPL [22] are avoidable.
7. **Project philosophy.** warte is purpose-built. The sync service is ~160 lines
   with a clear, bounded TODO path (§8.3). The gaps are known; none of them require
   a different database.

### 8.2 Recommended architecture (target state)

```
warte instance 1 (Hetzner)                 warte instance 2 (Netcup)
┌───────────────────────────┐              ┌───────────────────────────┐
│  Axum API                 │              │  Axum API                 │
│  Access control           │              │  Access control           │
│  Agent registry           │              │  Agent registry           │
│  ┌─────────────────────┐  │  WireGuard   │  ┌─────────────────────┐  │
│  │ Sync service        │  │◄────────────►│  │ Sync service        │  │
│  │  • membership       │  │  mTLS +      │  │  • membership       │  │
│  │    (SWIM-style)     │  │  signed CRDT │  │    (SWIM-style)     │  │
│  │  • anti-entropy     │  │  updates     │  │  • anti-entropy     │  │
│  │    sweep            │  │              │  │    sweep            │  │
│  │  • CRDT apply with  │  │              │  │  • CRDT apply with  │  │
│  │    policy validation│  │              │  │    policy validation│  │
│  └──────────┬──────────┘  │              │  └──────────┬──────────┘  │
│             │             │              │             │             │
│        SQLite (WAL)       │              │        SQLite (WAL)       │
│        + Litestream ──────┼──► S3        │        + Litestream ──────┼──► S3
└───────────────────────────┘              └───────────────────────────┘
```

**Keep:** SQLite local-first; version vectors; divergence detection + alerting;
mTLS channel; per-instance certs; push-based revocation fast path (< 5 s).

**Change / add:**

1. **Transport: gossip-based anti-entropy** (SWIM-style membership + Scuttlebutt-style
   state exchange [11][12]) over the existing WireGuard mesh, augmenting the HTTP
   push endpoints. Convergence no longer depends on a peer being reachable at write
   time. Start with a hand-rolled anti-entropy sweep on top of the existing version
   vectors (simplest, fewest deps); adopt `chitchat` [37] only if membership and
   failure-detection needs grow beyond what two instances require.
2. **CRDT types per table** (using the `crdts` crate already in `Cargo.toml` [35];
   `cr-sqlite` [36] is an alternative for table-level CRDTs but is largely in
   maintenance mode — last release Jan 2024):
   - `access_policies`: **OR-Set with add-wins revocation.** Grants and revokes are
     both *adds* to sets; effective rights = granted − revoked. A concurrent
     grant+revoke resolves to **revoked** — fail-safe (R3). Never LWW-by-wall-clock
     for authorization.
   - `agents`: LWW-Register per field with **hybrid logical clocks** (HLC) [13], not
     wall clock — immune to NTP skew between Hetzner and Netcup. Registration itself
     is an OR-Set add (idempotent re-registration).
   - `agent_data`: append-only log (OR-Set of records keyed by UUIDv7) — no conflicts
     by construction; partitionable by agent.
   - `artifacts`: LWW-Register for metadata (HLC); content is addressed by id and
     carries a sigil integrity proof — content never merges, only metadata does.
   - `ci_status`: LWW-Register (HLC) — rebuildable cache.
3. **Tombstone garbage collection.** Version-vector-based safe deletion: a tombstone
   can be dropped once all live instances have seen it. Prevents unbounded growth of
   `access_policies` tombstones.
4. **Durability layer: Litestream** [38] per instance → S3-compatible object storage
   (Hetzner Object Storage or equivalent). Bounds the data-loss window (R5), gives
   point-in-time restore, and provides disaster recovery for a destroyed instance
   (restore from backup + anti-entropy catch-up).
5. **Signed sync updates.** Each update carries the authorizing operator's sigil
   signature; the apply path validates policy (instances relay, operators originate).
   Closes the Byzantine gap as far as practical (§5.5, TS-006/CG-006).
6. **Encryption at rest.** SQLCipher (or an OS-level encrypted volume) for the
   SQLite file — it contains agent public keys, certs, and captured data (R7).
7. **Observability.** Sync lag per instance, time-to-converge after a partition
   heals, divergence alerts (exists — REQ-SYNC-003), tombstone count, anti-entropy
   round duration. Expose via `/api/sync/state` (exists) plus a metrics endpoint.
8. **Revocation fast path (keep).** Push-based revocation < 5 s stays; the
   anti-entropy sweep (e.g. every 30–60 s) bounds worst-case propagation and is the
   safety net when the push path can't reach a peer.

### 8.3 Concrete gaps in the current sync service (`src/sync/mod.rs`)

Found during this research's code review:

- `apply_remote_update` is a TODO — the CRDT merge logic does not exist yet.
- LWW uses `timestamp: DateTime<Utc>` (wall clock) — replace with HLC [13].
- No anti-entropy sweep — a peer that was down misses updates until it reconnects
  and pulls. Make the sweep periodic and bidirectional.
- No tombstone GC.
- No per-update signatures on the apply path.
- `detect_divergence` only reports "behind" — add convergence metrics.

### 8.4 When to revisit this decision

| Trigger | Likely target |
|---|---|
| Instance count grows to ≥ 3 **and** a hard strong-consistency requirement appears (synchronous revocation, linearizable reads for audit) | **rqlite** (3-node, MIT, SQLite-native, Jepsen-style tested) [17][19][21] |
| Captured-data volume makes per-instance SQLite unwieldy | Move `agent_data` to content-addressed object storage (sigil-signed); keep CRDT sync for the small authoritative state |
| Managed sync becomes a requirement (mobile/offline clients syncing directly) | Turso Sync / libSQL [39][40] |
| A singleton-coordination need appears (leader election for a periodic job) | etcd sidecar (3-node) [26][27], or a lease built on the existing CRDT store |
| LiteFS maintenance resumes and synchronous replication ships | Reconsider for read scale-out (today: rejected — unmaintained, single-writer, Consul dependency) [14][15][16] |

---

## 9. References

**warte (this repo)**

1. warte — `docs/architecture.md` (system overview, sync service, security model, threat model).
2. warte — `src/sync/mod.rs`, `src/db/migrations/001_initial.sql`, `Cargo.toml`, `docs/deployment.md`.

**Theory**

3. Shapiro, Preguiça, Baquero, Zawirski — *Conflict-free Replicated Data Types* (SSS 2011). https://asc.di.fct.unl.pt/~nmp/pubs/sss-2011.pdf
4. Vogels — *Eventually Consistent* (ACM Queue / CACM 2009). https://queue.acm.org/detail.cfm?id=1466448
5. Gilbert & Lynch — *Brewer's Conjecture and the Feasibility of Consistent, Available, Partition-Tolerant Web Services* (SIGACT News 2002). https://dl.acm.org/doi/10.1145/564585.564601
6. Abadi — *Consistency Tradeoffs in Modern Distributed Database System Design* (PACELC, IEEE Computer 2012). http://cs-www.cs.yale.edu/homes/dna/papers/abadi-pacelc.pdf
7. Herlihy & Wing — *Linearizability: A Correctness Condition for Concurrent Objects* (ACM TOCS 1990). https://dl.acm.org/doi/10.1145/78969.78972
8. Jepsen — *Consistency models* (reference page). https://jepsen.io/consistency
9. Fidge — *Timestamps in Message-Passing Systems That Preserve the Partial Ordering* (1988). https://dl.acm.org/doi/10.1145/62280.62308
10. Demers et al. — *Epidemic Algorithms for Replicated Database Maintenance* (PODC 1987). http://bitsavers.informatik.uni-stuttgart.de/pdf/xerox/parc/techReports/CSL-89-1_Epidemic_Algorithms_for_Replicated_Database_Maintenance.pdf
11. Das, Gupta, Motivala — *SWIM: Scalable Weakly-consistent Infection-style Process Group Membership Protocol* (2002). https://ieeexplore.ieee.org/document/1028914
12. van Renesse, Dumitriu, Antonsen — *Efficient Reconciliation and Flow Control for Anti-entropy Protocols* (Scuttlebutt, Cornell 2012); see also the Quickwit engineering blog [37].
13. Kulkarni et al. — *Logical Physical Clocks and Consistent Snapshots in Globally Distributed Databases* (2014). https://cse.buffalo.edu/tech-reports/2014-04.pdf

**LiteFS**

14. Fly.io — *How LiteFS Works* (async replication, Consul-lease primary election, split-brain handling via checksum + re-snapshot; "LiteFS is outside our scope of support"). https://fly.io/docs/litefs/how-it-works/
15. superfly/litefs — GitHub (Apache-2.0; last commit Apr 2025). https://github.com/superfly/litefs
16. Fly.io community — *Sunsetting LiteFS Cloud* (2024). https://community.fly.io/t/sunsetting-litefs-cloud/20829

**rqlite**

17. rqlite — *Read Consistency* (None/Weak/Linearizable/Strong; Jepsen-style testing note). https://rqlite.io/docs/api/read-consistency/
18. rqlite — *Security* (TLS, optional mTLS, auth, no at-rest encryption). https://rqlite.io/docs/guides/security/
19. rqlite — GitHub (MIT). https://github.com/rqlite/rqlite
20. O'Toole — *Faster Reads, Same Guarantees: Linearizable Consistency in rqlite 8.32*. https://philipotoole.com/faster-reads-same-guarantees-linearizable-consistency-in-rqlite-8-32/
21. O'Toole — *Testing rqlite read consistency* (Jepsen-style). https://philipotoole.com/testing-rqlite-read-consistency/

**dqlite**

22. Canonical — dqlite docs and GitHub (embeddable SQLite + Raft; LGPLv3 with static-linking exception). https://canonical.com/dqlite/docs · https://github.com/canonical/dqlite

**FoundationDB**

23. FoundationDB — *Consistency* (strict serializability). https://apple.github.io/foundationdb/consistency.html
24. Zhou et al. — *FoundationDB: A Distributed Unbundled Transactional Key-Value Store* (SIGMOD 2021). https://www.foundationdb.org/files/fdb-paper.pdf
25. `foundationdb` Rust crate (safe async wrapper over the FDB C API; requires `libfdb_c`). https://crates.io/crates/foundationdb · bindings PR: https://github.com/apple/foundationdb/pull/14187

**etcd**

26. etcd — *Transport security model* (TLS + client certs for client and peer traffic; no encryption at rest). https://etcd.io/docs/v3.6/op-guide/security/
27. etcd — *etcd versus other key-value stores*. https://etcd.io/docs/v3.5/learning/why/
28. Jepsen — *etcd 3.4.3* analysis. https://jepsen.io/analyses/etcd-3.4.3
29. etcd blog — *Latest Jepsen Results against etcd 3.4.3* (2020). https://etcd.io/blog/2020/jepsen-343-results/

**Consul**

30. HashiCorp — *Consensus* (Raft in Consul). https://developer.hashicorp.com/consul/docs/concept/consensus
31. HashiCorp — *Consistency Modes* (default/stale/consistent). https://developer.hashicorp.com/consul/api-docs/features/consistency
32. HashiCorp — *Secure Consul* (mTLS, ACLs, gossip encryption; not secure by default). https://developer.hashicorp.com/consul/docs/security
33. HashiCorp — *HashiCorp adopts Business Source License* (Aug 2023; BUSL 1.1). https://www.hashicorp.com/en/blog/hashicorp-adopts-business-source-license
34. Jepsen — *Jepsen: etcd and Consul* (2015). https://aphyr.com/posts/316-jepsen-etcd-and-consul

**Rust CRDT / gossip ecosystem**

35. `crdts` crate — docs.rs / crates.io (pure-Rust CRDTs: GCounter, PNCounter, LWWRegister, MVRegister, GSet, ORSet, VClock, …; MIT/Apache-2.0). https://docs.rs/crdts · https://crates.io/crates/crdts/7.3.2
36. vlcn-io/cr-sqlite — *Convergent, Replicated SQLite* (MIT; table-level CRDTs as a SQLite extension; largely maintenance mode). https://github.com/vlcn-io/cr-sqlite · https://vlcn.io/docs/cr-sqlite/quickstart
37. Quickwit — *Decentralized cluster membership in Rust* (Chitchat: Scuttlebutt + phi-accrual failure detector; Apache-2.0). https://quickwit.io/blog/chitchat · https://crates.io/crates/chitchat
38. Litestream — *How it works* / *Alternatives* (WAL streaming to S3; single-writer durability; Apache-2.0). https://litestream.io/how-it-works/ · https://litestream.io/alternatives/

**Adjacent**

39. Turso — *Embedded Replicas* (local read replica of a Turso cloud primary). https://docs.turso.tech/features/embedded-replicas/introduction
40. Turso — *Turso Sync: a much, much, much better way to sync* (CDC-based, local-first, push/pull). https://turso.tech/blog/sync-benchmark
41. Pang et al. — *Zanzibar: Google's Consistent, Global Authorization System* (2019). https://research.google/pubs/pub48190/
42. crdt.tech — *CRDT implementations* (survey). https://crdt.tech/implementations

---

*Research conducted 2026-10-26. All external sources accessed on that date. License
and maintenance-status claims reflect the state of the sources at access time and
should be re-verified before any adoption decision.*
