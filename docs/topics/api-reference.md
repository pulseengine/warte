# API reference

All endpoints return JSON. Mutating endpoints (POST, PUT, DELETE) require
authentication (mTLS or signed bearer token).

## Health

```
GET /api/health
```

Returns: `{ status, instance_id, version, uptime_secs, timestamp }`

## Agents

```
GET  /api/agents                    # list all agents
POST /api/agents/register           # register a new agent
GET  /api/agents/{id}               # get agent details
POST /api/agents/{id}/heartbeat     # record heartbeat
POST /api/agents/{id}/data          # capture agent data
GET  /api/agents/{id}/data          # query agent data
```

## Access control

```
POST /api/access/grant              # grant access rights
POST /api/access/revoke             # revoke access rights
GET  /api/access/list/{agent_id}    # list access rights
```

## Artifacts

```
GET /api/artifacts                  # list artifacts
GET /api/artifacts/{id}/download    # download artifact (integrity-verified)
```

## Dashboard

```
GET /api/dashboard/overview         # dashboard overview data
```

## Ground station (`/ci`, when `[hub]` is configured)

The absorbed smithy hub API, unchanged; the contract is `/ci/api/openapi.json`.
See `warte docs ground-station`.

```
GET /ci/api/eta?repo=X&pr=N         # when does PR N's next job start (or branch=)
GET /ci/status.md                   # fleet status as Markdown
GET /ci/api/state                   # latest snapshot + telemetry link stats
GET /ci/api/history?hours=H         # per-minute trend points
GET /ci/report.md?month=YYYY-MM     # monthly conveyor-belt report
```

## Sync

```
GET  /api/sync/state                # current sync state
POST /api/sync/update               # push state update
```
