# Architecture

Warte is a control tower — a CI dashboard with an ask feature, agent
orchestration, access control and mobile access.

## System overview

```
Internet → Edge Proxy (TLS 1.3 + rate limit + WAF)
              │ WireGuard VPN (<tunnel-range>)
              ├──→ warte instance 1 (primary)
              └──→ warte instance 2 (secondary)

Mobile (iOS/Android) ──mTLS + biometric──→ Edge Proxy ──→ warte
Agents (workers)     ──mTLS certs────────→ Edge Proxy ──→ warte
Dashboard (browser)  ──TLS───────────────→ Edge Proxy ──→ warte
```

## Components

| Component | Module | Description |
|---|---|---|
| API Server | `server/` | Axum HTTP server with 16 endpoints |
| Agent Registry | `agent/` | Agent registration, identity, heartbeat |
| Access Control | `access/` | RBAC, grant/revoke, push-based revocation |
| Database | `db/` | SQLite with migrations |
| CLI | `cli/` | clap-based CLI (serve, status, agents, access, migrate) |
| Config | `config/` | warte.toml + environment overrides |

## Design decisions

See `rivet list --type design-decision` for the full list. Key decisions:
- Rust + Axum + Tokio (matches PulseEngine toolchain)
- SQLite (embedded, no external dependency)
- mTLS with per-agent certificates (strong identity at transport layer)
- Push-based revocation with active disconnection (< 5s propagation)
- Edge proxy as sole internet-facing component (defense-in-depth)
