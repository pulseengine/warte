# Research: Bonjour / mDNS (multicast DNS, DNS-SD) for warte multi-instance service discovery

- **Date:** 2026-10-26
- **Status:** Research report — no code written
- **Scope:** Peer discovery for warte's multi-instance sync (Hetzner, Netcup, fourpi.local) without hardcoded peer addresses
- **Related docs:** `docs/architecture.md` (§5 Sync Service), `docs/deployment.md` (WireGuard 10.0.0.0/24), `src/sync/mod.rs`, `src/config/mod.rs` (`sync_peers`)

## TL;DR

1. **Adopt `mdns-sd` for the wire protocol, build warte's discovery layer on top.** mDNS/DNS-SD (RFC 6762/6763) is a standard wire protocol with real complexity (probing, conflict resolution, tiebreaking, known-answer suppression, goodbye packets, rate limiting). `mdns-sd` is the strongest Rust crate for it: pure safe Rust, no async-runtime dependency, actively maintained (release 2026-10-19, commits through 2026-10-25), ~6.8M downloads, Apache-2.0/MIT, zero RustSec advisories, and it publishes an RFC-compliance table. The purpose-built, self-owned part is warte's discovery semantics: service type, TXT attestation, peer lifecycle, and integration with `SyncService`.
2. **mDNS is a discovery mechanism, not a sync transport.** It answers "which warte instances exist and where do I reach them?" The sync channel stays mTLS over WireGuard. Because sync is already authenticated (per-instance certs, TS-006/CG-006), a spoofed mDNS record can at most cause a failed mTLS handshake — it cannot inject state.
3. **mDNS does not cross WireGuard.** WireGuard is layer-3 and does not transport multicast. Keep static `sync_peers` for the 10.0.0.0/24 overlay (already deployed); use mDNS only on the physical LAN (fourpi.local segment). Advertise the WireGuard sync address in the TXT record so LAN-discovered peers sync over the VPN.
4. **mDNS is unauthenticated by design** (RFC 6762 §21). Mitigations: treat all mDNS data as untrusted input, attest announcements with Ed25519 signatures (warte already depends on `ed25519-dalek`), enforce IP TTL=255 / source-port checks, keep 5353 off the WAN, and never let discovery bypass the mTLS sync handshake.
5. **Fallback ladder:** mDNS (LAN, zero-config) → static `sync_peers` (exists today; primary on the VPN) → unicast DNS-SD/SRV on the edge (etcd pattern, future) → gossip/SWIM (only if the fleet grows) → operator CLI.

---

## 1. Background: what mDNS/DNS-SD is, and the problem it solves for warte

**mDNS (RFC 6762)** lets hosts perform DNS-like operations on the local link with no configured DNS server: queries go to the link-local multicast addresses `224.0.0.251` (IPv4) / `ff02::fb` (IPv6) on UDP port 5353, and responders answer from the same port [1]. Names under `.local` are special-use, link-local names; queries for them MUST be sent to the multicast address, never to a unicast resolver [1].

**DNS-SD (RFC 6763)** layers service discovery on top: a service instance is a `PTR` → `SRV` → `TXT` (+ `A`/`AAAA`) record chain under a service type like `_http._tcp.local`. A browser sends one multicast query and learns instance name, host, port, and key/value attributes [2].

**warte's problem:** `SyncService` (`src/sync/mod.rs`) synchronizes CRDT state across instances, but peer addresses come from the static `sync_peers` config list (`src/config/mod.rs`, `docs/deployment.md`). That works for the fixed WireGuard overlay (10.0.0.2, 10.0.0.3) but not for instances that appear on the local network without configuration — e.g., the fourpi.local Raspberry Pi replacement, a dev machine, or a future instance on the home LAN. mDNS is the standard zero-config answer to "find my peers on this link."

**What mDNS is not:** it is not a transport, not a membership protocol, and not a secure channel. It is a broadcast-based, unauthenticated, best-effort announcement/query mechanism scoped to a single link.

---

## 2. Q1 — Rust crate landscape for mDNS/Bonjour

Data from crates.io API, GitHub API, and repo READMEs, collected 2026-10-26. "Advisories" = RustSec Advisory Database entries against the crate (checked per-crate directories in `rustsec/advisory-db`; all returned 404 = none on file) [9].

### 2.1 Comparison table

| Crate | Latest | Last release | Downloads | Maintenance | Implementation | Async model | Security notes | License |
|---|---|---|---|---|---|---|---|---|
| **mdns-sd** [3] | 0.21.6 | 2026-10-19 | 6.77M | **Active** (pushed 2026-10-25; 227★, 18 open issues) | Pure **safe** Rust | Runtime-agnostic: own daemon thread + `flume` channels; no async runtime dep | No advisories; publishes RFC 6762/6763 compliance table (probing, tiebreaking, conflict resolution, goodbye, rate limiting ✅; unicast responses ❌) | Apache-2.0 OR MIT |
| **libmdns** [4] | 0.10.1 | 2025-09-06 | 1.14M | Active-ish (88★, 15 open issues) | Pure Rust (RFC 6762 + 6763) | tokio (`sync`/`net`/`rt` only) | No advisories; production-proven — used by librespot for Spotify Connect LAN discovery; responder-focused, browsing API thinner | MIT |
| **zeroconf** [5] | 0.18.0 | 2026-04-07 | 391k | Active (94★, 5 open issues) | **FFI wrappers** around system implementations: `avahi-sys` (Linux), `bonjour-sys` (BSD), `dns-sd` (Windows) | Sync API | No advisories against the crate itself, but inherits the **system daemon's attack surface** — Avahi has a long CVE history (see §4.3); requires avahi-daemon installed on Linux; heavier dep tree | MIT (per repo; crates.io metadata reports "non-standard") |
| **mdns** [6] | 3.0.0 | 2021-01-29 | 464k | **Unmaintained** (~5.7 years) | Pure Rust, tiny (419 LoC) | Sync | No advisories; **client-only** (query, no responder) — cannot announce a warte instance | MIT |
| **simple-mdns** [7] | 0.7.0 | 2026-05-15 | 83k | Occasional | Pure Rust (built on `simple-dns`) | Sync + async modules (tokio optional) | No advisories; small dep set (`simple-dns`, `radix_trie`, `log`) | MIT |
| **agnostic-mdns** [8] | 0.4.2 | 2025-03-16 | 8.1k | New (created 2025-01), author active (40★) | Pure Rust, **sans-I/O protocol core** with pluggable runtime | Runtime-agnostic (tokio / async-std / smol features) | No advisories; MSRV 1.85 + edition 2024 — **warte is edition 2021**, so this would force a toolchain/edition decision | MIT OR Apache-2.0 |
| **async-zeroconf** | 0.2.2 | 2021-09-15 | 9.4k | **Stale** (5 years) | FFI wrappers | tokio | macOS-focused (categorized `os::macos-apis`); not viable for Linux servers | MIT OR Apache-2.0 |
| **libp2p-mdns** | 0.48.0 | ~2025 | — | Maintained (libp2p org) | Pure Rust (~1.2k LoC) | tokio | Only relevant if warte adopts libp2p — it does not | MIT |
| **if-watch** [10] | 3.2.2 | 2026-03-03 | 19.2M | Maintained (libp2p org) | Pure Rust + platform APIs | async | **Not an mDNS crate** — network-interface watcher used by `libp2p-mdns` to re-probe on interface changes; listed for completeness | MIT OR Apache-2.0 |

### 2.2 Reading the table

- **Popularity ≠ fit.** `if-watch` (19.2M) and `mdns` (464k) are popular but are not mDNS service-discovery libraries in the sense warte needs. `mdns-sd` is both the most-downloaded *actual* DNS-SD library (6.77M) and the best fit.
- **Pure Rust vs FFI is the key security fork.** `zeroconf` delegates to the OS (Bonjour on macOS/Windows, Avahi on Linux). That is battle-tested but adds a system-daemon dependency and inherits its CVE history (§4.3). For a server that must run unattended on Ubuntu 24.04, a pure-Rust responder with a small dep tree is the better default.
- **Responder + browser both needed.** warte instances must *announce* (`_warte-sync._tcp.local`) and *browse*. `mdns` (client-only) and `libmdns` (responder-first, thinner browsing) are weaker on one side; `mdns-sd` covers both with one API.
- **Runtime fit.** warte is tokio-based. `mdns-sd` needs no runtime at all (thread + channels), so it cannot conflict with warte's tokio version — a real advantage over tokio-coupled crates (`libmdns`, `simple-mdns` async module).
- **Compliance transparency.** `mdns-sd` publishes a per-feature RFC 6762/6763 compliance table in its README [3]. The gaps (unicast responses, multipacket known-answer suppression on the responder side) do not matter for warte's use case, which is multicast browse + announce on a LAN.

---

## 3. Q2 — How other distributed systems use mDNS (and what they use instead)

Pattern survey across systems that solve the same problem: *how do nodes find each other without hardcoded addresses?*

| System | Discovery mechanism | mDNS role | Source |
|---|---|---|---|
| **etcd** | Three bootstrap modes: static `--initial-cluster`, hosted discovery service, **DNS SRV** (`--discovery-srv`, e.g. `_etcd-server._tcp.example.com`) | **None.** etcd deliberately uses unicast DNS SRV records, not multicast | [11][12] |
| **Consul** | **Serf gossip (SWIM)** for membership + failure detection (LAN pool per DC, WAN pool between DCs); Raft for catalog consistency; **DNS interface** (port 8600, `<service>.service.consul`) + HTTP API for service lookup | **None** for cluster formation. mDNS is not part of Consul's architecture | [13][14] |
| **Nomad** | Serf gossip for cluster membership; **Consul integration** for service discovery; can bootstrap the cluster via Consul service discovery | **None** | [15][16] |
| **Kubernetes** | Headless Services + cluster DNS (CoreDNS/kube-dns): DNS SRV (`_port._protocol.service.namespace.svc`) | **None** — unicast DNS only | (standard k8s architecture) |
| **Docker Swarm** | Gossip-based membership (memberlist-style) over the overlay network | **None** | (swarmkit architecture) |
| **libp2p (Rust)** | **mDNS** as a `NetworkBehaviour` for LAN peer discovery: nodes announce, peers discover and add multiaddrs to the topology (`Discovered`/`Expired` events); `if-watch` re-probes on interface changes | **Primary LAN mechanism** — exactly warte's use case, one layer down | [17][18][10] |
| **Home Assistant / ESPHome** | **mDNS/DNS-SD** for device discovery on the LAN; requires an mDNS reflector/repeater (Avahi `reflector=yes`, `mdns-repeater`) to cross VLANs | **Primary mechanism** for IoT on one link | [19][20] |
| **iroh (n0)** | **mDNS for local** address lookup + **pkarr** (signed DNS TXT records published to an HTTPS relay, resolved via regular DNS) for global discovery; identity = public key (`EndpointId`) | mDNS for the local case; signed unicast DNS for the wide-area case | [21] |
| **Tailscale** | **MagicDNS**: unicast DNS on the tailnet; every machine gets a unique IP and a name; subnet routers advertise routes | **Explicitly not mDNS.** From the engineering blog: "We've been trained not to trust DNS for service discovery, but it was never the DNS protocol that was the problem. It was caching, latency, and polling." | [22] |
| **Spotify Connect (librespot)** | **libmdns** responder announces the device on the LAN; clients browse | Production mDNS responder in Rust | [4] |

### 3.1 Synthesis — the pattern

1. **mDNS is the standard answer for "peers on the same link, zero config"** — LANs, IoT, P2P nodes. It is *not* what serious cluster membership uses.
2. **Cluster membership in distributed systems uses gossip (SWIM/memberlist) or unicast DNS SRV** — etcd (DNS SRV), Consul/Nomad/Swarm (gossip). These mechanisms are authenticated or at least overlay-scoped, and they work across routed networks.
3. **The modern hybrid (iroh, Tailscale) is instructive:** mDNS for the local link, *signed/unicast DNS* for everything routed. Tailscale's lesson applies directly to warte's WireGuard overlay: on a routed VPN, use unicast DNS or static config — not multicast.
4. **Implication for warte:** mDNS should be scoped to the physical LAN segment only. The WireGuard overlay already has a working, authenticated answer (static `sync_peers` + mTLS). Don't try to stretch mDNS across the VPN; if the overlay ever needs dynamic discovery, the etcd/Tailscale answer is unicast DNS-SD on the edge, not multicast bridging.

---

## 4. Q3 — Security analysis: mDNS on a local network

### 4.1 The baseline: mDNS is unauthenticated by design

RFC 6762 §21 (Security Considerations) positions mDNS as appropriate for links where participants are mutually trusted: the protocol provides **no authentication, no integrity protection, and no confidentiality** for queries or responses [1]. Any host on the link can:

- **claim any name** (subject only to the probing/conflict-resolution ritual, which is a race, not an authorization check),
- **respond to any query**, and
- **observe all traffic** (it is multicast).

The one link-scoping control in the RFC: "All Multicast DNS responses (including responses sent via unicast) SHOULD be sent with IP TTL set to 255" [1] — receivers can (and should) verify TTL=255 to confirm a packet originated on the local link rather than being routed in from elsewhere.

### 4.2 Threat model for warte

Academic and industry literature on mDNS attacks [23][24][25][26]:

| # | Threat | Mechanism | Impact on warte if unmitigated |
|---|---|---|---|
| T1 | **Service impersonation / spoofing** | Attacker on the LAN answers the `_warte-sync._tcp.local` browse first with their own SRV/TXT. "An mDNS querier typically takes the first response it receives… if this race is won by the attacker, it can lead to various related Man-in-the-Middle (MiTM) attacks" [23] | warte connects to attacker's "instance" → **contained by mTLS**: sync channel requires per-instance certs signed by the warte CA (TS-006/CG-006). Handshake fails; no state injected |
| T2 | **Name squatting / conflict** | Attacker probes and claims the instance name before the real instance starts (RFC 6762 §8–9 conflict resolution is first-come, not authorized) | Discovery DoS for that instance name → mitigated by signed TXT + static fallback + operator-pinned keys |
| T3 | **Cache poisoning via legacy unicast queries** | Queries from source port ≠ 5353 may be answered via unicast (RFC 6762 §6.7); a spoofed unicast response poisons the querier's cache | Mitigated by accepting responses only from source port 5353 and TTL=255; verify the crate enforces this (§8 checklist) |
| T4 | **Reflection/amplification (DrDoS)** | mDNS exposed to the Internet is a known reflected-amplification vector: spoofed-source queries to UDP 5353, botnet-driven (INCIBE) [24]. A 46-byte query can elicit 4–10× larger responses [25] | warte's edge is already default-deny (ufw: only 22/51820/443) [27]; 5353/udp must stay off the WAN and off the WireGuard `AllowedIPs` — WG does not route multicast anyway (§5) |
| T5 | **Information disclosure** | TXT records are multicast in cleartext: instance IDs, addresses, software versions, topology | Keep TXT minimal; no secrets; sign but don't encrypt (or encrypt to the peer's pinned key). Treat mDNS data as public |
| T6 | **Responder implementation bugs** | The system mDNS daemon is itself attack surface: Avahi has a long CVE history — e.g. CVE-2025-68471 (remote DoS via crafted packet, ≤0.9-rc2) [28], CVE-2026-24401 (crafted mDNS response with recursive CNAME, CVSS 6.5) [29], CVE-2025-59529 [30], ~20 CVEs in Debian 13 alone [31] | Prefer **pure-Rust** responder (`mdns-sd`/`libmdns`) over FFI to system Avahi (`zeroconf` crate); run `cargo audit`/`cargo deny` in CI (warte's CI already exists [32]) |
| T7 | **Reflector/repeater trust-boundary violation** | An mDNS reflector (Avahi `reflector=yes`) or repeater copies multicast between segments — services become discoverable across VLANs/subnets that were never meant to see each other [19][20] | If ever deployed: restrict `allow-interfaces`, reflect only between trusted segments, never toward untrusted IoT VLANs. warte does not need a reflector for v1 |
| T8 | **Replay of announcements** | Captured TXT/SRV records replayed later to resurrect a dead peer or fake liveness | Include a timestamp in the signed TXT; enforce a freshness window (e.g. ±5 min); rely on goodbye packets (TTL=0) + sync-level heartbeats for liveness |

### 4.3 The decisive mitigation: discovery is untrusted input; sync is authenticated

warte's existing security model already contains the answer. The architecture doc's threat model lists **TS-006: sync channel injection → CG-006 (mTLS + integrity proofs)** [33]. If mDNS is used *only* to produce candidate peer addresses, then the worst case of T1–T3 is: warte dials an attacker, the mTLS handshake fails (attacker has no CA-signed instance cert), the peer is dropped and an alert is raised. **mDNS cannot bypass the sync channel's authentication because it never carries sync data.**

The one thing mDNS *can* do is waste connection attempts and leak metadata (T4/T5). Both are bounded by the mitigations above.

### 4.4 Defense-in-depth checklist for warte's mDNS layer

1. Verify Ed25519 signature in TXT against pinned instance public keys before adding a peer to the sync set (warte already has `ed25519-dalek`).
2. Reject announcements for our own `instance_id`.
3. Enforce timestamp freshness (replay window).
4. Prefer the WireGuard address from TXT for syncing; treat the mDNS-resolved LAN address as a fallback only.
5. Firewall: 5353/udp allowed on LAN interfaces only; never on WAN or wg0 (default-deny already covers this [27]).
6. Rate-limit processing of announcements; ignore malformed packets (pure-Rust parser = memory-safe, but still).
7. Log and alert on: signature failures, unknown instance keys, mTLS handshake failures to discovered peers, and name conflicts.
8. Run `cargo audit` + `cargo deny` on the new dependency in CI [32].

---

## 5. Q4 — mDNS and the WireGuard VPN

### 5.1 The hard constraint: WireGuard does not transport multicast

WireGuard is a layer-3, point-to-point routed VPN. It has no broadcast/multicast domain: mDNS packets (224.0.0.251:5353) do not traverse `wg0`, and an mDNS reflector on a WG peer won't see queries from the other side [34][35]. This is well documented in the field: "WireGuard connections do not support multicast by default… even with a mDNS reflector it's not going to see the mDNS requests over the tunnel" [34].

### 5.2 Options for crossing the VPN, honestly assessed

| Option | How it works | Assessment for warte |
|---|---|---|
| **A. Static config on the overlay (status quo)** | `sync_peers = ["https://10.0.0.3:8080"]` — already in `warte.toml` and `docs/deployment.md` | **Keep as primary for the VPN.** 3 instances, stable addresses, mTLS. Zero moving parts |
| **B. Unicast DNS-SD on the edge (etcd/Tailscale pattern)** | Run a tiny DNS server (CoreDNS) on the edge (10.0.0.1) answering SRV/TXT for `_warte-sync._tcp.warte.internal`; instances resolve via unicast DNS over WG | **Best future option** if the overlay grows. Same DNS-SD semantics as mDNS, but over routed unicast — authenticated at the TLS/DNS layer, no multicast needed. This is exactly what etcd does with `--discovery-srv` [11] and what Tailscale built MagicDNS to avoid needing mDNS for [22] |
| **C. Avahi reflector / mdns-repeater between LAN and VPN** | A daemon with interfaces on both segments copies mDNS packets across [19][20] | **Not viable over WG** — a reflector needs layer-2 adjacency on both segments; WG is routed L3 point-to-point. Would only work LAN↔LAN (e.g., across VLANs at fourpi.local), which warte doesn't need for v1. Also note reflectors have their own bugs (e.g. `mdns-repeater` not setting IP TTL=255 [36]) and create trust boundaries (T7) |
| **D. VXLAN (or other L2 tunnel) over WireGuard** | Create a VXLAN interface on top of `wg0` on each peer, bridge multicast across it [37] | **Rejected.** Adds a broadcast domain over a routed VPN: complexity, MTU/fragmentation issues, and it re-introduces exactly the multicast trust problems mDNS has, now across sites. A known homelab hack, not a server architecture |
| **E. mDNS on the LAN, advertise the WG address** | Instance announces `_warte-sync._tcp.local` on its LAN interface; the TXT record carries its **WireGuard** sync address (10.0.0.x:8080) | **Recommended.** LAN peers discover each other via multicast, then sync over the VPN with mTLS. mDNS stays link-local; the sync traffic stays on the authenticated overlay |

### 5.3 Recommendation

- **LAN segment (fourpi.local, dev machines):** mDNS announce + browse, option E.
- **WireGuard overlay (Hetzner edge, Netcup, fourpi replacement):** static `sync_peers` (already deployed), mTLS sync. No mDNS on `wg0` — it cannot work, and we shouldn't pretend otherwise.
- **Future, if instance count grows:** option B — unicast DNS-SD served by CoreDNS on the edge, discovered via WG. This is the etcd `--discovery-srv` pattern and the Tailscale MagicDNS lesson applied to warte's own overlay.

---

## 6. Q5 — Fallback when mDNS is not available

mDNS fails when: the link has no multicast (VPN, some cloud VPCs), the host firewall blocks 5353, the LAN is segmented (VLANs without a reflector), or multicast is administratively disabled. warte needs a ladder, not a single point of failure:

| Priority | Mechanism | When it applies | Effort | Source pattern |
|---|---|---|---|---|
| 1 | **mDNS/DNS-SD on the LAN** | Same-link instances, zero config | Adopt `mdns-sd` + build `src/discovery/` | libp2p, ESPHome, Spotify Connect [17][19][4] |
| 2 | **Static `sync_peers` config** | Known instances on the WireGuard overlay — **already implemented** in `src/config/mod.rs` and `docs/deployment.md` | None — exists | etcd `--initial-cluster` static bootstrap [11] |
| 3 | **Unicast DNS SRV / DNS-SD on the edge** | Routed overlay grows beyond a handful of instances | CoreDNS on 10.0.0.1 + SRV/TXT records | etcd `--discovery-srv` [11][12]; Tailscale MagicDNS [22]; iroh pkarr (signed DNS TXT) [21] |
| 4 | **Gossip membership (SWIM)** | Fleet grows to ~10+ instances, churn is high, config is a burden | Adopt `memberlist` (Rust port of HashiCorp's SWIM memberlist) or build a minimal SWIM core | Consul/Nomad/Swarm [13][14][15] |
| 5 | **Operator action** | Everything else failed; human in the loop | `warte peers add/remove` CLI + dashboard; also used to pin instance keys | — |

Design rule: **static config is always loaded; mDNS peers are additive, never exclusive.** An instance's peer set = `sync_peers` ∪ (mDNS-discovered, signature-verified peers) − (peers that sent goodbye or failed mTLS). mDNS can add peers and remove them (goodbye/timeout); it can never remove a statically configured peer. This makes mDNS strictly an availability improvement, never a regression vector.

---

## 7. Architecture recommendation: how mDNS fits warte's multi-instance sync

### 7.1 Design principles

1. **Discovery ≠ transport.** mDNS produces candidate addresses; the CRDT sync protocol (`SyncService`, version vectors, delta sync) runs unchanged over mTLS.
2. **Untrusted input, authenticated channel.** All mDNS data is untrusted until the sync handshake authenticates the peer.
3. **LAN-only scope.** mDNS binds to physical LAN interfaces, never `wg0`.
4. **Additive, not exclusive.** Static `sync_peers` always applies (§6).

### 7.2 Proposed design

**New module: `src/discovery/`** (self-built, purpose-built — this is the part warte owns):

```
src/discovery/mod.rs
├── MdnsDiscovery      — announce + browse lifecycle (wraps the adopted wire crate)
├── Attestation        — TXT schema: build + verify Ed25519-signed announcements
├── PeerSet            — merge of static + discovered peers, dedupe by instance_id
└── (integration)      — feeds SyncService; emits events to dashboard/SSE
```

**Service registration (DNS-SD, RFC 6763):**

- Service type: `_warte-sync._tcp.local.`
- Instance name: the instance's UUIDv7 `instance_id` (already in `Config::instance_id()`)
- SRV/A records: the instance's **LAN** address and API port (what mDNS resolves)
- TXT record (the important part):

```
id=<instance_id uuidv7>
wg=<wireguard sync address, e.g. 10.0.0.2:8080>   ← preferred sync target
lan=<lan address:port>                              ← fallback if no WG route
vv=<hex sha256 of current version vector>           ← informational, for dashboard
ts=<unix seconds>                                   ← replay protection
sig=<ed25519 signature over "id|wg|lan|vv|ts">     ← attestation
```

**Browse flow:**

```
1. warte starts → register service (probe → announce per RFC 6762 §8–10)
2. Browse _warte-sync._tcp.local continuously (continuous query, known-answer suppression)
3. On discovered instance:
   a. parse TXT → reject if id == self
   b. verify sig against pinned instance keys (config) → reject + alert if unknown
   c. check ts freshness (±5 min) → reject if stale
   d. add wg= address to PeerSet → SyncService dials it with mTLS
   e. mTLS handshake fails → drop peer, alert (spoof contained)
4. On goodbye (TTL=0) or heartbeat timeout → remove from PeerSet (never static peers)
5. Dashboard shows discovered peers, verification status, last-seen freshness
```

**Key distribution:** instance public keys pinned in `warte.toml` (`discovery.trusted_instance_keys = ["ed25519:…"]`), distributed out-of-band by the operator (same trust root as the mTLS CA — the CA cert path is already in config). TOFU-with-operator-approval is a possible later refinement; pinned keys are the v1 answer.

**Config sketch:**

```toml
[discovery]
enabled = true                 # default: true when sync_peers is non-empty
announce = true
browse = true
service_type = "_warte-sync._tcp.local"
interfaces = ["eth0", "en0"]   # LAN only — wg0 is never eligible
trusted_instance_keys = ["ed25519:…"]
replay_window_secs = 300
```

**Why this shape:** the TXT attestation means a spoofed announcement is detectable *before* any connection attempt (defense in depth on top of mTLS); advertising the WG address in TXT means LAN discovery and VPN transport compose correctly; and the whole layer is ~300–500 lines of warte-owned logic around a small, audited wire-protocol crate.

---

## 8. Build vs adopt recommendation

warte prefers self-built, purpose-built components. The recommendation splits the problem along the standard line — **adopt the protocol, build the policy**:

### 8.1 Adopt: `mdns-sd` (wire protocol only)

- RFC 6762/6763 packet machinery is standardized, fiddly, and security-relevant: probing, simultaneous-probe tiebreaking, conflict resolution, known-answer suppression, goodbye packets, rate limiting, packet-size limits. `mdns-sd` implements these and publishes a compliance table [3]. Even systemd needed 45 scripted conformance scenarios to *score* its mDNS implementation [38] — reimplementing this correctly is weeks of work with a real bug surface, for zero product differentiation.
- Profile: pure safe Rust, no async-runtime dependency (fits warte's tokio stack without version coupling), MSRV 1.71 (warte is edition 2021 — compatible), actively maintained (release 2026-10-19, commits through 2026-10-25), ~6.8M downloads, Apache-2.0/MIT (compatible with warte's MIT), zero RustSec advisories [3][9].
- Small dep tree: `fastrand`, `flume`, optional `log`/`serde`.
- **This is not "giving up self-built."** The wire protocol is a standard, like TLS or DNS itself. warte's self-built value is in what mDNS *means* for warte (see 8.2).

### 8.2 Build: warte's discovery layer (the purpose-built part)

Everything warte-specific stays in-house, in `src/discovery/`:

- Service type and TXT schema (`_warte-sync._tcp.local`, `id|wg|lan|vv|ts|sig`)
- Ed25519 attestation and key pinning (extends the existing CA trust model)
- Peer lifecycle: dedupe by `instance_id`, goodbye/timeout handling, static-peer protection
- Integration with `SyncService` (peer set → mTLS dial), config, CLI (`warte discover`), dashboard/SSE visibility
- Divergence/alerting hooks (a discovered-but-unverifiable peer is an alertable event)

### 8.3 Considered and rejected

| Alternative | Why rejected |
|---|---|
| `zeroconf` crate | FFI to system Bonjour/Avahi; requires avahi-daemon on Linux; inherits Avahi's CVE history (§4.2 T6); heavier deps |
| `libmdns` | Responder-focused; thinner browsing API; smaller ecosystem; tokio-coupled |
| `mdns` crate | Unmaintained since 2021; client-only (can't announce) |
| `simple-mdns` | Viable but smaller/less complete; no compliance transparency |
| `agnostic-mdns` | Interesting sans-I/O design, but created 2025-01, 8k downloads, MSRV 1.85 + edition 2024 (warte is edition 2021). Re-evaluate if warte moves to edition 2024 |
| `async-zeroconf` | Stale since 2021, macOS-focused |
| `libp2p-mdns` / `if-watch` | Only relevant inside a libp2p stack; warte is not one |
| **Full self-build of mDNS** (no crate) | Not recommended for v1. A minimal RFC 6762 responder+browser is feasible (~2–4k LoC) but reimplements a standard with a known attack surface (T3, T6) for no product gain. **Revisit triggers:** mdns-sd maintenance regresses; warte needs custom conflict semantics; a no-std target appears. If ever built, model it on `agnostic-mdns`'s sans-I/O core |

### 8.4 Verdict

**Adopt `mdns-sd` as the wire protocol; build `src/discovery/` in-house.** This honors the self-built preference where it matters (semantics, security policy, integration) and avoids reimplementing a standard where it doesn't.

---

## 9. Open questions / evaluation checklist (before implementation)

1. **Interface selection:** verify `mdns-sd`'s API for binding to specific interfaces (must exclude `wg0`, include only LAN NICs). If missing, contribute upstream or filter at the PeerSet layer.
2. **Packet hygiene:** verify the crate checks IP TTL=255 on received packets and source port 5353 for multicast responses (RFC 6762 link-scoping; T3). If not enforced by the crate, enforce at the PeerSet layer (drop non-conforming announcements).
3. **IPv6:** confirm `ff02::fb` behavior on the Raspberry Pi target (fourpi.local replacement).
4. **Key distribution UX:** pinned keys in `warte.toml` vs TOFU + operator approval — decide before v1; pinned is simpler and matches the CA model.
5. **Conflict behavior:** two instances with the same `instance_id` (misconfiguration) — RFC 6762 conflict resolution is first-come; the signature check makes the impostor detectable. Test this.
6. **Benchmarks:** announcement→discovery latency, goodbye propagation, behavior with 5+ instances on one link, packet rate (rate limiting per RFC 6762 §6).
7. **CI:** add `cargo audit` / `cargo deny` for the new dependency [32].
8. **Docs:** update `docs/deployment.md` (mDNS on LAN, static on VPN) and `docs/architecture.md` (§5 Sync Service + new Discovery component).

---

## 10. Sources

1. RFC 6762 — Multicast DNS (Cheshire & Krochmal, 2013). https://www.rfc-editor.org/rfc/rfc6762 (also https://datatracker.ietf.org/doc/html/rfc6762)
2. RFC 6763 — DNS-Based Service Discovery (Cheshire & Krochmal, 2013). https://www.rfc-editor.org/rfc/rfc6763.html
3. `mdns-sd` crate + README (RFC compliance table). https://crates.io/crates/mdns-sd · https://github.com/keepsimple1/mdns-sd · https://docs.rs/mdns-sd
4. `libmdns` crate (librespot-org) — pure-Rust RFC 6762/6763 responder, MSRV 1.74. https://crates.io/crates/libmdns · https://github.com/librespot-org/libmdns
5. `zeroconf` crate (windy1/zeroconf-rs) — cross-platform wrappers over Bonjour/Avahi. https://crates.io/crates/zeroconf · https://github.com/windy1/zeroconf-rs
6. `mdns` crate (dylanmckay) — multicast DNS client, unmaintained since 2021. https://crates.io/crates/mdns · https://github.com/dylanmckay/mdns
7. `simple-mdns` crate. https://crates.io/crates/simple-mdns · https://github.com/balliegojr/simple-dns
8. `agnostic-mdns` crate (al8n) — runtime-agnostic, sans-I/O core. https://crates.io/crates/agnostic-mdns · https://github.com/al8n/agnostic-mdns
9. RustSec Advisory Database (per-crate advisory checks for mdns-sd, libmdns, zeroconf, mdns, simple-mdns, agnostic-mdns, if-watch — none on file as of 2026-10-26). https://rustsec.org/advisories/ · https://github.com/rustsec/advisory-db
10. `if-watch` crate (libp2p) — network interface watcher. https://crates.io/crates/if-watch · https://github.com/libp2p/if-watch
11. etcd documentation — cluster bootstrap: static, discovery service, DNS SRV (`--discovery-srv`). https://etcd.io/docs/v3.4/op-guide/configuration/ · https://etcd.io/docs/v3.4/op-guide/clustering/ (mirror: https://chromium.googlesource.com/external/github.com/coreos/etcd/+/v3.0.0-beta.0/Documentation/op-guide/clustering.md)
12. Stack Overflow — bootstrapping etcd via DNS discovery + Consul (`_etcd-server._tcp` SRV records). https://stackoverflow.com/questions/54110773/
13. Consul docs — Gossip (Serf, LAN/WAN pools). https://developer.hashicorp.com/consul/docs/concept/gossip
14. HashiCorp — "Everybody Talks: Gossip, Serf, memberlist, Raft, and SWIM in HashiCorp Consul". https://www.hashicorp.com/en/resources/everybody-talks-gossip-serf-memberlist-raft-swim-hashicorp-consul · memberlist (SWIM): https://github.com/hashicorp/memberlist
15. Nomad docs — Consul integration / service discovery. https://developer.hashicorp.com/nomad/docs/networking/consul · https://developer.hashicorp.com/nomad/docs/networking/service-discovery
16. Nomad docs — cluster formation via Consul service discovery. https://developer.hashicorp.com/nomad/tutorials/archive/clustering
17. libp2p docs — mDNS peer discovery. https://libp2p.io/docs/mdns/ · https://docs.rs/libp2p/latest/libp2p/mdns/ · https://github.com/libp2p/rust-libp2p/blob/master/protocols/mdns/src/behaviour.rs
18. Stack Overflow — implementing peer discovery in libp2p (mDNS = LAN only). https://stackoverflow.com/questions/71106417/
19. ESPHome — Security Best Practices (mDNS broadcasts device presence; VLAN isolation). https://esphome.io/guides/security_best_practices/ · Avahi reflector across VLANs: https://blog.christophersmart.com/2020/03/30/resolving-mdns-across-vlans-with-avahi-on-openwrt/
20. Whizz Tech — AirPrint across VLANs with Avahi reflector (`reflector=yes`, `allow-interfaces`). https://whizz-tech.com/support/printers/airprint-across-vlans-avahi-reflector/ · avahi-daemon.conf `[REFLECTOR]` section (via DeepWiki): https://deepwiki.com/avahi/avahi/3.2-configuration-management
21. iroh docs — discovery: mDNS for local lookup, pkarr/DNS for endpoint discovery. https://docs.iroh.computer/concepts/discovery · https://docs.iroh.computer/llms-full.txt
22. Tailscale blog — "Understanding Tailscale DNS and the Magic Behind MagicDNS". https://tailscale.com/blog/magicdns-why-name
23. Atlasis, A. — "An Attack-in-Depth Analysis of Multicast DNS and DNS Service Discovery" (mDNS/DNS-SD spoofing, first-response race → MITM). https://sfc6326dbff511243.jimcontent.com/download/version/1492160351/module/10962123597/name/AAtlasis%20-%20An%20Attack-in-Depth%20Analysis%20of%20multicast%20DNS%20and%20DNS%20Service%20Discovery_wp.pdf
24. INCIBE-CERT — "DrDoS cyberattacks based on the mDNS protocol" (reflected amplification, source spoofing, port 5353 exposure). https://www.incibe.es/en/incibe-cert/blog/drdos-cyberattacks-based-mdns-protocol
25. HackTricks — 5353/UDP mDNS/DNS-SD (amplification factor, monitoring guidance). https://book.hacktricks.xyz/network-services-pentesting/5353-udp-multicast-dns-mdns
26. Red Fox Security — LLMNR/NBT-NS/mDNS spoofing: attack & defense. https://www.redfoxsec.com/blog/llmnr-nbt-ns-and-mdns-spoofing-a-practical-guide-to-name-resolution-poisoning
27. warte `docs/deployment.md` — ufw default-deny firewall (22/51820/443 only).
28. CVE-2025-68471 — Avahi mDNS daemon DoS (≤0.9-rc2). https://www.sentinelone.com/vulnerability-database/cve-2025-68471/
29. CVE-2026-24401 — Avahi crafted mDNS response, recursive CNAME (CVSS 6.5, patched 0.9-rc3). https://access.redhat.com/security/cve/cve-2026-24401
30. CVE-2025-59529 — Avahi mDNS/DNS-SD vulnerability. https://cve.akaoma.com/cve-2025-59529
31. Avahi CVE history — cvedetails vendor page; Debian 13: 20 CVEs. https://www.cvedetails.com/vulnerability-list/vendor_id-4481/Avahi.html · https://www.progressiverobot.com/2026/04/11/debian-13-avahi-vulnerability-patch-remediation/
32. warte CI workflow. `.github/workflows/ci.yml`
33. warte `docs/architecture.md` — threat model TS-006 / control CG-006 (sync channel injection → mTLS + integrity proofs).
34. Server Fault — "Ping and mDNS over WireGuard not working" (WG = L3, no multicast). https://serverfault.com/questions/1141369/
35. SNB Forums — mDNS over WireGuard (WireGuard is layer 3 and doesn't transport multicast). https://www.snbforums.com/threads/mdns-repeater-need-help-to-compile-for-arm-ac68u.19568/
36. `mdns-repeater` issue #10 — does not set IP TTL to 255 (quotes RFC 6762). https://github.com/geekman/mdns-repeater/issues/10
37. slonk.ing — "mDNS over VXLAN over WireGuard" (VXLAN + bridge fdb + nftables recipe). https://blog.slonk.ing/posts/mdns-tales
38. systemd PR #43220 — TEST-89: scoring systemd-resolved's mDNS/DNS-SD conformance across 45 scripted scenarios. https://github.com/systemd/systemd/pull/43220
39. `memberlist` Rust port (al8n) — runtime-agnostic SWIM gossip for membership/failure detection. https://docs.rs/memberlist · https://github.com/al8n/memberlist

---

*Research conducted 2026-10-26. Crate statistics from crates.io API and GitHub API at collection time; re-verify before implementation. No code was written for this task.*
