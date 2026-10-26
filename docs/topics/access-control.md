# Access control

Warte uses Role-Based Access Control (RBAC) with per-resource, per-action
scoping. The access control service is the single source of truth.

## Roles

| Role | Rights |
|---|---|
| `admin` | `*:*` (all resources, all actions) |
| `operator` | agents:read, agents:control, access:grant, access:revoke, access:read, dashboard:read, artifacts:read |
| `agent` | artifacts:download, data:write, heartbeat:write |
| `viewer` | dashboard:read, agents:read |

## Grant and revoke

```bash
# Grant rights
warte access grant <agent-id> --rights "artifacts:download,data:write"

# Revoke rights (push-based, < 5s propagation)
warte access revoke <agent-id> --rights "artifacts:download"

# List rights
warte access list <agent-id>
```

## Revocation

Revocation is push-based: the access control service broadcasts the
revocation to all instances via the sync channel, actively disconnects
the agent's connections, and confirms propagation before acknowledging.

## API

```bash
POST /api/access/grant    # grant rights
POST /api/access/revoke   # revoke rights
GET  /api/access/list/{agent_id}  # list rights
```
