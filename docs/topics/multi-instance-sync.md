# Multi-instance sync

Warte instances synchronize state using a CRDT-based approach with
version vectors for causality tracking.

## How it works

1. Each instance maintains a version vector (instance_id → counter)
2. On state change, the instance increments its counter
3. Instances exchange version vectors and missing updates (delta sync)
4. Conflicts are resolved deterministically (CRDT merge or LWW)
5. Divergence beyond a threshold triggers an operator alert

## Service discovery

Instances discover each other via:
- **Static config**: `sync_peers` in warte.toml
- **mDNS/Bonjour** (v0.2.0): automatic discovery on local network

## Security

Sync communication uses mTLS with per-instance certificates. mDNS
discovery is NOT trusted for authentication — discovered peers still
authenticate via mTLS before exchanging data.

## API

```bash
GET  /api/sync/state    # current sync state
POST /api/sync/update   # push state update
```
