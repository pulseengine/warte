# API reference

All endpoints return JSON. Mutating endpoints (POST, PUT, DELETE) require
authentication (mTLS or signed bearer token).

## Bearer tokens for agents

Write routes (`POST /api/agents/register`, `/api/agents/{id}/heartbeat`,
`/api/agents/{id}/data`, `/api/costs`, `/api/skill-usage`, ...) answer `401` without
`Authorization: Bearer <token>`. Nothing issues a token over HTTP; the
operator mints one on the ground station host:

```
warte token mint <agent-id> [--valid-hours 720] [--any-agent] [--right <r>]...
```

- Each write route needs one right in the token (REQ-API-004). A token
  without it gets `403`; a missing or invalid token stays `401`.

  | Route | Right |
  |---|---|
  | `POST /api/agents/register` | `agents:register` |
  | `POST /api/agents/{id}/heartbeat` | `agents:heartbeat` |
  | `POST /api/agents/{id}/data` | `agents:data` |
  | `POST /api/access/grant` | `access:grant` |
  | `POST /api/access/revoke` | `access:revoke` |
  | `POST /api/sync/update` | `sync:update` |
  | `POST /api/costs` | `costs:write` |
  | `POST /api/skill-usage` | `skills:write` |

  A write route with no row in that table (`MUTATING_ROUTES` in
  `src/auth/mod.rs`) is refused with `403`. The probe routes authenticate
  themselves and are not in it.
- A bare mint carries `agents:register`, `agents:data` and `agents:heartbeat`:
  enough for an agent, not for `access:*`, `sync:update`, `costs:write` or `skills:write`.
  `--right <r>` (repeatable) adds a right, e.g. `--right costs:write`; a name
  that is not in the table is refused at mint time.
- The agent binding: a token names an agent by the name it registers under.
  On `/api/agents/register` the `name` in the body must equal the token's
  agent id. Register assigns its own id (a UUID); on `/api/agents/{id}/...`
  the `name` of the agents row `{id}` must equal the token's agent id, and an
  unknown `{id}` is refused. Otherwise `403`. `--any-agent` adds `agents:any`, which lifts the binding
  for a token that also holds the route's right, e.g. one workstation token
  that posts notices for many repositories. It is not a right to any route.
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
GET /api/eta?repo=spar&pr=511                    # when PR 511's next job starts (or branch=)
GET /api/watch?repo=spar&pr=511[&since=<cursor>] # hold until its runs start or finish
```

`/api/eta` answers `{repo, pr, branch, status, running, queued, scan_age_s,
queued_complete, recheck_after_s?, scope, note}`: per running job `on`, `hosted`,
`workflow`, `job` and `for_s`; per queued part `run_id`, `workflow`, `pool`,
`jobs`, `waited_s`, `model` and `estimate` (`start_in_low_s`, `start_in_high_s`,
`overdue`, `basis`), estimated from measured waits in the same pool, `null` with
fewer than 5 samples.

## Ground station history and reports (REQ-SMITHY-006)

What the `/ci` page offered, under warte's own API, each with the same answer
as the standalone ground station's (`warte hub`); `503` without a ground station.

```
GET /api/status.md[?repo=spar&pr=511]    # fleet status as Markdown, or one PR's start estimate
GET /api/report?month=YYYY-MM            # monthly conveyor-belt report (JSON), default this month
GET /api/report.md?month=YYYY-MM         # the same as Markdown
GET /api/history/fleet?hours=H           # per-minute fleet points, H from 1 to 168 (default 24)
```

The dashboard shows them under **CI history** (trends over 6 h, 24 h and 7 d;
the belt report for this and the two previous months) and the CI host's timers
under **Health**. The ground station's whole snapshot is not published here: it
carries the CI host's raw automation log lines.

- `security` and `red-main` are always `open`.
- `docs` batch whenever the repo has CI queued or running.
- `feature` holds when every self-hosted runner is busy and 50+ self-hosted jobs wait,
  and batches when the repo already has 2+ runs queued. `fix` batches at 2+.
- Stale data (snapshot older than 5 minutes) never gates: the verdict is `open` and
  the reason says the data is stale.
- `503` when no ground station is configured or no snapshot has arrived.

## Machines (REQ-PROBE-003, REQ-PROBE-013)

```
GET /api/machines
```

Every machine's latest report, reduced for an unauthenticated read (no paths,
pids, command lines or error text), and the findings over all of them. Per
machine: `host`, `via` (`probe` or `local`), `received_at`, `age_s`, `stale`,
`cert_until` (unix seconds the probe's certificate ends, `null` for the ground
station itself) and `report`, whose `probe_version` is `null` unless it is a
plain version string. Probes older than 1.5.0 cannot renew their certificate;
the findings say so 14 days before it ends.

## Disk pressure (REQ-PRESSURE-001)

```
GET /api/pressure?host=<host>
```

Whether a host is low on disk, for agents to check before filling it further.
One verdict per writable volume from the host's latest probe report (`ok`,
`low`, `critical`), the host's `verdict` being its worst volume, and `unknown`
when there is no fresh report. Thresholds are GB free, set in warte's config:

```toml
[pressure]
low_gb = 20       # default
critical_gb = 10  # default
```

Each volume has `mount`, `free_gb`, `total_gb`, `verdict` and `held_gb`: the GB
agent scratchpads, `target/` directories and git worktrees hold there, as
totals per kind (`null` until the probe has sized them; it does so only while a
volume is low). The answer names no session, repo, pid or path: the read API is
unauthenticated. Who holds the space is shown on the machine itself by
`warte probe pressure`. `host` is the name the probe enrolled under; `warte probe
pressure` prints it. Without `host` the answer is `400` with the known hosts,
for an unknown host `404`.

## Machine history (REQ-MHIST-002)

Every machine's numbers over time, kept by warte itself (`history.redb` next to
the machines state): per minute for 7 days, hourly min/avg/max for 90 days
(`[history] keep_days`, at least 8). Read-only, same exposure as `/api/machines`:
host names, series names and numbers only.

```
GET /api/history/machines                      # hosts with history
GET /api/history/machines/<host>/metrics       # its series names and units
GET /api/history/machines/<host>/series?metric=load.1,mem.used_pct&from=<unix>&to=<unix>
```

`series` answers `{host, from, to, series: [{metric, unit, t0, step, values, min?, max?}]}`.
`to` defaults to now and `from` to 24 h before it. `step` is 60 s for ranges up to
7 days and 3600 s beyond (then with `min` and `max`); value `i` is at
`t0 + i*step`; a gap is `null`. Series names are dotted, with labels for mounts,
devices and runners: `load.1`, `load.per_core`, `mem.used_pct`, `swap.used_mb`,
`disk.used_pct{mount=/}`, `io.busy_pct{dev=sda}`, `psi.io.some`, `sessions`,
`runners.busy`, `runner.cpu{id=3}`, `queue.self_hosted`.

- `400` for `from` after `to`, a range over 400 days, or no `metric`.
- `404` for a host without history, or when this warte keeps none.

## Retired: `/ci`

Up to v1.6.2, `warte serve` also mounted the absorbed smithy-hub page and API
under `/ci`. Since v1.6.3 it is gone; since v1.6.4 every `/ci` path answers
`410 Gone` with JSON `{"error":"gone","see":…,"reason":…}`, where `see` is the
replacement relative to warte's base URL, query kept. Use the routes above
(`/ci/api/eta` → `/api/eta`, `/ci/status.md` → `/api/status.md`,
`/ci/api/report` → `/api/report`, `/ci/report.md` → `/api/report.md`,
`/ci/api/history` → `/api/history/fleet`). `/ci/api/state` has no successor: the
whole snapshot carries raw log lines; the telemetry link and the timers are on the
Health view. `warte hub`, the standalone ground station, still serves the old API
on its own port (`warte docs ground-station`).

## Sync

```
GET  /api/sync/state                # current sync state
POST /api/sync/update               # push state update
```
