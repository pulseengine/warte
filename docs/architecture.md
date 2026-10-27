# Warte — Architecture

The PulseEngine control tower. A CI dashboard with an ask feature, agent
orchestration, access control, multi-instance sync, and mobile access.

German: "watchtower" — and "bitte warte auf mich", "please wait for me".

## Context

Warte is the OSS version of the internal control tower described in
[The orchestrator: who does what](https://pulseengine.eu/blog/the-orchestrator-who-does-what/).
It sits in the **toolchain layer** alongside meld, loom, synth, kiln, sigil,
and varve. It is not a dashboard on top of the factory — it is a tool in the
factory.

The orchestrator (the human owner) uses warte to:
1. **See** the whole factory — all repos, all gates, all agents
2. **Ask** — "when will this PR run?" — ETA from measured waits
3. **Control** — grant/revoke agent access, dispatch agents, manage instances
4. **Monitor** — from the phone, from anywhere

## System Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                         INTERNET                                     │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────────────┐  │
│  │  Mobile     │    │   Agent     │    │   Dashboard             │  │
│  │  (iOS/      │    │  (worker    │    │   (browser)             │  │
│  │   Android)  │    │   machine)  │    │                         │  │
│  └──────┬──────┘    └──────┬──────┘    └───────────┬─────────────┘  │
│         │                  │                       │                 │
│         └──────────────────┼───────────────────────┘                 │
│                            │ mTLS (TLS 1.3)                          │
│  ┌─────────────────────────▼─────────────────────────────────────┐   │
│  │              EDGE PROXY (Hetzner / Netcup)                     │   │
│  │  ┌─────────┐  ┌─────────────┐  ┌─────────┐  ┌──────────────┐  │   │
│  │  │ TLS 1.3 │  │ Rate Limit  │  │ Request │  │ Attack       │  │   │
│  │  │ Termin. │  │ (per-IP/    │  │ Schema  │  │ Detection    │  │   │
│  │  │         │  │  per-agent) │  │ Valid.  │  │ (WAF rules)  │  │   │
│  │  └────┬────┘  └──────┬──────┘  └────┬────┘  └──────┬───────┘  │   │
│  │       └───────────────┴───────────────┴───────────────┘        │   │
│  │                         │                                       │   │
│  │              Reverse Proxy (nginx / Caddy)                     │   │
│  └─────────────────────────┼─────────────────────────────────────┘   │
│                            │                                         │
│                    Private Network (WireGuard VPN)                   │
│                            │                                         │
│  ┌─────────────────────────▼─────────────────────────────────────┐   │
│  │              WARTE INSTANCE 1 (primary)                        │   │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐  │   │
│  │  │  API    │ │ Agent   │ │ Access  │ │ Artifact│ │  Sync   │  │   │
│  │  │ Server  │ │ Registry│ │ Control │ │ Store   │ │ Service │  │   │
│  │  │ (Axum)  │ │         │ │ Service │ │         │ │ (CRDT)  │  │   │
│  │  └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘  │   │
│  │       └───────────┴───────────┴───────────┴───────────┘        │   │
│  │                         │                                       │   │
│  │              SQLite (local state)                              │   │
│  └─────────────────────────┼─────────────────────────────────────┘   │
│                            │                                         │
│  ┌─────────────────────────▼─────────────────────────────────────┐   │
│  │              WARTE INSTANCE 2 (secondary, Hetzner)             │   │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐  │   │
│  │  │  API    │ │ Agent   │ │ Access  │ │ Artifact│ │  Sync   │  │   │
│  │  │ Server  │ │ Registry│ │ Control │ │ Store   │ │ Service │  │   │
│  │  │ (Axum)  │ │         │ │ Service │ │         │ │ (CRDT)  │  │   │
│  │  └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘ └────┬────┘  │   │
│  │       └───────────┴───────────┴───────────┴───────────┘        │   │
│  │                         │                                       │   │
│  │              SQLite (local state)                              │   │
│  └─────────────────────────┼─────────────────────────────────────┘   │
│                            │                                         │
│                    Sync Channel (mTLS, per-instance certs)          │
│                    Version vectors + CRDT merge                     │
│                    Divergence detection + alerting                  │
└─────────────────────────────────────────────────────────────────────┘
```

## Components

### 1. API Server (Axum)

The core HTTP server. All endpoints require authentication (mTLS or signed
bearer token). Endpoints:

| Endpoint | Method | Auth | Description |
|---|---|---|---|
| `/api/health` | GET | none | Health check with freshness |
| `/api/agents` | GET | agent/operator | List registered agents |
| `/api/agents/register` | POST | none (mTLS handshake) | Register a new agent |
| `/api/agents/:id/heartbeat` | POST | agent | Agent heartbeat |
| `/api/agents/:id/data` | POST | agent | Agent captures data |
| `/api/agents/:id/data` | GET | operator | Query agent data |
| `/api/artifacts` | GET | agent | List available artifacts |
| `/api/artifacts/:id/download` | GET | agent | Download artifact (integrity-verified) |
| `/api/access/grant` | POST | operator | Grant access rights |
| `/api/access/revoke` | POST | operator | Revoke access rights |
| `/api/access/list` | GET | operator | List access rights |
| `/api/dashboard/overview` | GET | operator | Dashboard overview data |
| `/ci/api/eta` | GET | public (read-only) | ETA query ("when will this PR run?"), from the ground station |
| `/api/sync/state` | GET | instance | Get sync state |
| `/api/sync/update` | POST | instance | Push state update |
| `/api/ws` | WS | agent/operator | WebSocket for real-time updates |
| `/api/events` | SSE | operator | Server-Sent Events for dashboard |

### 2. Agent Registry

Manages agent registrations. Each agent gets:
- A unique agent ID (UUIDv7)
- An mTLS client certificate (signed by warte CA, 24h expiry)
- A set of access rights (scoped per resource and action)
- A heartbeat tracker (last seen, health status)

Registration flow:
1. Agent generates an Ed25519 key pair (or uses existing TPM/SE key)
2. Agent sends registration request with public key + agent metadata
3. Control tower verifies the request (out-of-band approval or auto-approve for known agents)
4. Control tower issues an mTLS client certificate
5. Agent uses the certificate for all subsequent communication

### 3. Access Control Service

The single source of truth for access decisions. Manages:
- **Roles**: predefined (admin, operator, agent, viewer) + custom roles
- **Rights**: per-resource (artifact, agent, instance) × per-action (read, download, register, revoke, control)
- **Revocation**: push-based, < 5s propagation, active disconnection

RBAC model:
```
Agent → has → Role → grants → Rights (resource × action)
Operator → has → Role → grants → Rights (resource × action)
```

Revocation flow:
1. Operator calls `/api/access/revoke` with agent ID + rights
2. Access control service updates the policy database
3. Service pushes revocation to all instances via sync channel
4. Service actively disconnects the agent's WebSocket/HTTP connections
5. Service confirms propagation before acknowledging
6. Dashboard shows revocation status with freshness indicator

### 4. Artifact Store

Stores downloadable artifacts (tools, configs, data). Each artifact carries:
- Content (the artifact bytes)
- Integrity proof (sigil/wsc detached signature, or SHA-256 in signed manifest)
- Access scoping (which agents can download)
- Metadata (version, description, created_at)

Download flow:
1. Agent requests `/api/artifacts/:id/download` with mTLS cert
2. Access control service checks agent's rights for this artifact
3. Control tower verifies the artifact's integrity proof
4. Control tower serves the artifact + integrity proof
5. Agent verifies the integrity proof before use

### 5. Sync Service (CRDT-based)

Synchronizes state across warte instances. Uses:
- **Version vectors** for causality tracking
- **CRDTs** for commutative state (agent registrations, access rights)
- **Last-writer-wins** for non-commutative state (with version vector tiebreak)
- **Divergence detection** (version vector distance > threshold → alert)

Sync protocol:
1. Instances exchange version vectors on connect
2. Instances exchange missing updates (delta sync)
3. Conflicts resolved deterministically (CRDT merge or LWW)
4. Divergence beyond threshold triggers operator alert

### 6. Edge Proxy (Hetzner/Netcup)

The sole internet-facing component. Runs:
- **TLS 1.3 termination** (nginx or Caddy)
- **Rate limiting** (per-IP: 100 req/min, per-agent: 1000 req/min)
- **Request validation** (JSON schema / OpenAPI)
- **Attack detection** (WAF rules: brute force, injection, scanning)
- **Auto-patching** (unattended-upgrades, 24h max for critical CVEs)
- **Reverse proxy** to internal warte instances (WireGuard VPN)

### 7. Mobile Client (iOS/Android)

Native apps for remote control:
- **iOS**: SwiftUI, mTLS with device-bound certificate in Keychain
- **Android**: Jetpack Compose, mTLS with device-bound certificate in Keystore
- **Auth**: biometric (fingerprint/face) or hardware token per command
- **Features**: real-time status (WebSocket), agent management, access control, alerts

## Security Model

### Defense in Depth

| Layer | Control | Requirement |
|---|---|---|
| Network | Edge proxy only internet-facing | REQ-EDGE-001 |
| Transport | TLS 1.3, mTLS for agents | REQ-AGENT-REG-001, REQ-MOBILE-001 |
| Authentication | Per-agent certificates, operator biometric | REQ-AGENT-REG-001, REQ-MOBILE-002 |
| Authorization | RBAC, per-resource per-action | REQ-ACCESS-003 |
| Integrity | Sigil/wsc signatures on artifacts | REQ-ARTIFACT-002 |
| Availability | Rate limiting, divergence detection | REQ-EDGE-001, REQ-SYNC-003 |
| Freshness | Timestamps + freshness indicators | REQ-DASH-001 |

### Threat Model (STPA-Sec)

See `artifacts/security.yaml` for the full TARA. Key threats:
- **TS-001**: Agent credential theft → CG-001 (mTLS)
- **TS-002**: Artifact tampering → CG-002 (sigil signatures)
- **TS-003**: Revocation delay exploitation → CG-003 (push-based revocation)
- **TS-004**: Mobile session hijack → CG-004 (mTLS + biometric)
- **TS-005**: Edge proxy compromise → CG-005 (defense-in-depth, auto-patch)
- **TS-006**: Sync channel injection → CG-006 (mTLS + integrity proofs)

## Migration

### From smithy hub (fourpi.local)

smithy hub was absorbed as warte's ground station (DD-SMITHY-001): `warte hub`
reads its data directory as-is. See `warte docs migration` for the parallel run
and cut-over, and `warte docs ground-station` for the data path.

### From temper (GitHub App)

Import:
- Branch protection rules → warte access control policies
- Issue labels → warte label taxonomy
- PR/issue templates → warte templates
- Dependabot config → warte dependency policies
- Dashboard data model (org repos, CI status, PR overview)
- Rivet oracle integration (rivet-fetch, rivet-oracle patterns)

Command: `warte migrate temper --config /path/to/temper/config.yml`

## Deployment

### Hetzner (primary instance)
- 2 vCPU, 4 GB RAM, 40 GB SSD
- Ubuntu 24.04 LTS
- warte instance 1 (primary)
- SQLite local state

### Netcup (edge proxy + secondary instance)
- Edge proxy: 2 vCPU, 4 GB RAM (nginx/Caddy + WAF)
- Secondary instance: 2 vCPU, 4 GB RAM (warte instance 2)
- WireGuard VPN between edge and internal instances

### fourpi.local (Raspberry Pi, to be replaced)
- Runs smithy hub (the CI fleet ground station) behind Caddy at `/ci/`
- `warte hub` replaces it in place after the parallel run; no data import

## Release Plan

### v0.1.0 — Server skeleton (current)

Scope: 48 artifacts (12 design decisions + 27 requirements + 9 verifications).

| Phase | Items | Status |
|---|---|---|
| Bootstrap | varve pin, rivet init, STPA/STPA-Sec, requirements, design decisions | done |
| Server skeleton | Axum server, CLI, health endpoint | done |
| Agent auth | mTLS, registration, heartbeat | done (mTLS cert issuance pending) |
| Access control | RBAC, grant/revoke, push-based revocation | done |
| Artifact store | Download, integrity verification, scoping | done (sigil signatures pending) |
| Multi-instance sync | CRDT, version vectors, divergence detection | done |
| Dashboard API | Overview, ask, queue, critical path | partial |
| Migration | smithy import, temper import | pending |
| Edge deployment | Hetzner/Netcup config, WireGuard, auto-patch | docs done |

### v0.2.0 — Toolchain integration, discovery, multi-platform

Scope: 18 artifacts (6 design decisions + 12 requirements).

| Phase | Items | Status |
|---|---|---|
| Embedded docs | `warte docs <topic>`, `warte docs check --coverage` | pending |
| Service discovery | mDNS/Bonjour for multi-instance (zeroconf crate) | pending |
| Multi-platform CI | GitHub Enterprise, GitLab (CiPlatform trait) | pending |
| Temper link | HTTP integration, not code absorption | pending |
| Release pipeline | SHA256SUMS, SBOM, SLSA, cosign, rivet V-model gate | pending |
| Varve layer | warte binary in the pulseengine layer | pending |
| Reuse relay patterns | Contracts, seam traits, consumption contracts | pending |

### Release cycle (continuous)

Once v0.1.0 is cuttable:
1. `rivet release status v0.1.0` — confirm all verified
2. Release artifact pipeline (SHA256SUMS, SBOM, SLSA, cosign)
3. Tag, push, open PR, wait for CI green, merge
4. Create GitHub release with artifacts
5. Scope v0.2.0 requirements in rivet
6. Repeat

### Research (ongoing)

Three research tracks, all with "build over buy" preference:
- **Bonjour/mDNS**: service discovery for multi-instance (subagent: research-bonjour)
- **Distributed databases**: CRDT-SQLite vs LiteFS vs rqlite vs FoundationDB (subagent: research-distributed-db)
- **Similar tools**: CI dashboards, agent orchestrators, control towers (subagent: research-similar-tools)

Findings go to `docs/research/` with build/adopt/ignore decisions.
