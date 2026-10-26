# Security model

Warte's security model is based on STPA/STPA-Sec analysis (see
`artifacts/security.yaml` for the full TARA).

## Defense in depth

| Layer | Control |
|---|---|
| Network | Edge proxy only internet-facing |
| Transport | TLS 1.3, mTLS for agents |
| Authentication | Per-agent certificates, operator biometric |
| Authorization | RBAC, per-resource per-action |
| Integrity | Sigil/wsc signatures on artifacts |
| Availability | Rate limiting, divergence detection |
| Freshness | Timestamps + freshness indicators |

## Key threats (STPA-Sec)

| Threat | Mitigation |
|---|---|
| Agent credential theft | mTLS with per-agent certificates |
| Artifact tampering | Sigil/wsc signatures, SHA-256 verification |
| Revocation delay | Push-based revocation, < 5s propagation |
| Mobile session hijack | mTLS + biometric authentication |
| Edge proxy compromise | Defense-in-depth, auto-patching, WAF |
| Sync channel injection | mTLS + integrity proofs on sync messages |

## Losses prevented

- L-001: Unauthorized system access via agent impersonation
- L-002: Loss of work due to orchestration failure
- L-003: Unauthorized control via mobile channel
- L-004: Inconsistent state across instances
- L-005: Bad decisions from incorrect dashboard status
