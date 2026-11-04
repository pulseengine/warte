# API reference

All endpoints return JSON. Mutating endpoints (POST, PUT, DELETE) require
authentication (mTLS or signed bearer token).

## Bearer tokens for agents

Write routes (`POST /api/agents/register`, `/api/agents/{id}/heartbeat`,
`/api/agents/{id}/data`, `/api/costs`, ...) answer `401` without
`Authorization: Bearer <token>`. Nothing issues a token over HTTP; the
operator mints one on the ground station host:

```
warte token mint <agent-id> [--valid-hours 720]   # prints only the token
```

- Rights in the token: `agents:register`, `agents:data`, `agents:heartbeat`.
  The write routes check signature and expiry today, not the rights.
- `--valid-hours` is 1 to 8760; the default is 24.
- The signing seed is a file, `token-seed` next to the CA key
  (`<state-dir>/token-seed`) unless `token_seed_path` in `warte.toml` or
  `WARTE_TOKEN_SEED_PATH` names another. The server and `token mint` create it
  with mode 0600 if missing and refuse a seed other users can read. Tokens
  survive restarts as long as the file does. Back it up like the CA key and
  never commit it; deleting it invalidates every token.
- Give the agent the token through its environment or a 0600 file, not the
  command line, e.g. `PULSEENGINE_WARTE_TOKEN_FILE=<path>`.

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

## Agents: queue and PR admission

Read-only JSON from the ground station's latest snapshot; the contract is
`/api/openapi.json`. Ask before opening a PR, so small changes don't pile onto a
saturated fleet.

```
GET /api/queue[?repo=spar]                       # self_hosted, hosted, repo, as_of
GET /api/admission?repo=spar&kind=docs           # {verdict: open|batch|hold, reason, recheck_s}
    kind: docs | fix | feature | security | red-main
```

- `security` and `red-main` are always `open`.
- `docs` batch whenever the repo has CI queued or running.
- `feature` holds when every self-hosted runner is busy and 50+ self-hosted jobs wait,
  and batches when the repo already has 2+ runs queued. `fix` batches at 2+.
- Stale data (snapshot older than 5 minutes) never gates: the verdict is `open` and
  the reason says the data is stale.
- `503` when no ground station is configured or no snapshot has arrived.

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
