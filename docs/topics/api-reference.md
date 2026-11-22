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
  | `POST /api/reports/seam-check` | `reports:write:seam-check` |
  | `POST /api/agents/{id}/data` | `agents:data` |
  | `POST /api/access/grant` | `access:grant` |
  | `POST /api/access/revoke` | `access:revoke` |
  | `POST /api/costs` | `costs:write` |
  | `POST /api/skill-usage` | `skills:write` |
  | `POST /api/briefing` | `briefing:write` |
  | `POST /api/triage/done` | `triage:write` |
  | `POST /api/news/state` | `news:write` |

  A write route with no row in that table (`MUTATING_ROUTES` in
  `src/auth/mod.rs`) is refused with `403`. The probe routes authenticate
  themselves and are not in it.
- A bare mint carries `agents:register`, `agents:data` and `agents:heartbeat`:
  enough for an agent, not for `access:*`, `costs:write`, `skills:write`, `briefing:write`, `triage:write` or `news:write`.
  `--right <r>` (repeatable) adds a right, e.g. `--right costs:write`; a name
  that is not in the table is refused at mint time.
- The agent binding: a token names an agent by the name it registers under.
  On `/api/agents/register` the `name` in the body must equal the token's
  agent id. Register assigns its own id (a UUID); on `/api/agents/{id}/...`
  the `name` of the agents row `{id}` must equal the token's agent id, and an
  unknown `{id}` is refused. Otherwise `403`. `--any-agent` adds `agents:any`, which lifts the binding
  for a token that also holds the route's right, e.g. one workstation token
  that posts notices for many repositories. It is not a right to any route.
- Reads (since warte v1.6.30, REQ-API-005): with `[auth]`, the agents'
  records need a login, or a warte token holding `agents:read`. That right
  opens `GET /api/agents` (the list) and `GET /api/agents/{id}` and
  `GET /api/agents/{id}/data` for the token's own agent only, bound as the
  write routes are; another agent's or an unknown `{id}` is `403`, unless the
  token also holds `agents:any`. A bare mint does not carry it: add
  `--right agents:read`, e.g. for a notice client that reads back what it
  posted. Without `[auth]` the list and rows are open, as before.
- Sensitive reads (since warte v1.6.44, REQ-API-007, #46): even without
  `[auth]`, `GET /api/agents/{id}/data` and `GET /api/access/list/{agent}` need a
  warte token holding `agents:read`: `401` without a valid token, `403` without the
  right. Data and access lists answer only for the token's own agent unless
  it also holds `agents:any`. With `[auth]` a login opens them as before.
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
POST /api/agents/register           # register an agent, or get its id again
GET  /api/agents/{id}               # get agent details
POST /api/agents/{id}/heartbeat     # record heartbeat
POST /api/agents/{id}/data          # capture agent data
GET  /api/agents/{id}/data          # query agent data
```

Register is idempotent per name (since v1.6.33, REQ-API-006): a name this
warte already has returns that agent's `agent_id` (the newest, if older
releases left several), with a fresh certificate and the request's `repo`,
`identity_type` and `public_key`. A retry or a rerun never makes a second
agent.

A heartbeat may carry `{"status": "ended" | "idle" | "working"}` (since
v1.6.48, REQ-GLANCE-001, seam warte-plugin/heartbeat-status): `ended` from a
session's end hook, `idle` when it waits at the prompt. A beat without a
`status` field (no body, `{}`, or what older clients send) is as before; only
an unknown `status` is `422`.

## Decisions (REQ-DECIDE-001)

```
POST /api/decisions/answer   {"notice_id": "...", "revision": 2, "option_id": "...", "note": "optional"}
```

The owner answers an agent's open `needs-decision` notice from the
Decisions view (Agents). Only a signed-in user with the owner role, from this
server's origin, with the session's CSRF token in `X-Warte-CSRF` (the view
sends both); otherwise `401` or `403`, and without `[auth]` always `401`. The
notice must exist, be at that revision, still be open and not answered, and
have offered the option; otherwise `409` with the reason, and nothing is
stored. The answer is stored on the agent that asked as an `answer.v1` row
(notice.schema.json answer, schema_version 1, `via: "warte"`), which the
notice client polls and consumes once. A note is data for the agent, never
instructions, at most 2000 characters. Each answer is logged with who and
which notice, never the note.

## Reviews (REQ-PRREVIEW-001)

```
GET /api/reviews
```

The pull requests one person has to review (#85), from `[reviews]`:

```toml
[reviews]
token_file = "/home/me/.config/warte/review-token"  # the reviewer's own read-only token, mode 0600
teams = ["acme/core"]                 # team review requests that count
highlight_authors = ["boss"]          # also listed, and highlighted
highlight_repos = ["acme/app"]
highlight_labels = ["urgent"]
# api_url = "https://api.github.com"  # a GitHub Enterprise API
# interval_s = 600
```

Every `interval_s` warte asks GitHub who the token is, then runs one search
per bucket: review requested from the reviewer, from each team, watched
authors, watched repositories. Drafts and the reviewer's own pull requests
are left out. Each pull request shows why it is listed, its size (S up to 50
changed lines, M up to 250, L up to 1000, XL above), its build (green, red
with the first failed check, running), the review decision, the reviewer's
own latest review, and how long it has waited. It is a re-review when
commits were pushed after that review, and done when approved, green and
mergeable. Order: re-reviews first, then the longest waiting, done ones last.
A failed scan keeps the last list and says why, never with the token.

PR titles and authors are private work data: with `[auth]` only a signed-in
user reads them; without it, only a warte bound to loopback (`401`
otherwise). The CI menu shows them as Reviews.

## Reports (REQ-REPORTS-001)

```
POST /api/reports/seam-check     # seam-check.v1, with reports:write:seam-check
```

A producer posts its report after its run (seam pulseengine-warte/reports,
#87). The token needs the kind's right, `reports:write:<kind>`, and nothing
else. warte checks the schema string and the fields it renders, at most
2 MiB, and keeps the latest; `422` names the broken field. The views render
only what was posted and compute nothing: Compliance, Seams shows every seam
with its status, provider, consumers, violations, warnings and open
questions, violations first, stale after 8 days. Signed-in only, because the
reports cover private repositories; without `[auth]` the view shows nothing.

## Night glance (REQ-GLANCE-001)

```
GET /api/glance
GET /glance
```

Whether anything needs the owner now, as JSON or one phone-sized page.
Signed-in only, since it reads notices; without `[auth]` both are `401`.
`verdict` is `needs-you` when `items` holds anything, one per: an open
`needs-decision` notice tagged with an always-open kind (default `security`
and `red-main`, set with `[glance] always_open` to mirror the notice client's
`always_open_kinds`), a repository red on its default branch, a release run
that failed and was not rerun green, a host with a critically full disk.
Otherwise `quiet`. Routine decisions are counted in `routine_decisions`, never
listed; agents silent for over 30 minutes and agents never heard from are
counted in `silent` and `unknown`, never listed. A decision title is cut to 80
characters. Read-only: it answers and triggers nothing.

## Access control

```
POST /api/access/grant              # grant access rights
POST /api/access/revoke             # revoke access rights
GET  /api/access/list/{agent_id}    # list access rights
```

## Dashboard

```
GET /api/dashboard/overview         # dashboard overview data
GET /api/dashboard/nav-state        # the menu: views hidden while empty, counts per group
```

The menu (v1.6.41, REQ-DASH-019) has five groups: Now (North Star, Overview,
Outages, Briefing), CI (Queue, Airport, CI history, Critical Path, Jobs,
Resources, Schedule, Ask), Machines (Machines, Health), Agents (Agents, Costs,
Skills, Friction, Artifacts) and Compliance. The open group's views form the
second row; a group opens on its first view, and the page with no view in the
address opens on Overview. Every view keeps its route and its address
(`#machines`, `#queue`, ...). `nav-state` answers `{attention, hidden, badges}`: `attention` lists the
items of the "Needs your attention" strip under the menu (`kind`, `text`,
`view`; a disk 95% full or more, counted, and each workflow red on a public
repository's default branch), empty when nothing waits; then: the views
hidden while empty (Friction with no report, Artifacts with none; a link still
opens them, and the row says what it hides), and per group the red and amber
counts the group shows (Now: outages; Machines: forecasts; Compliance: failing
temper checks red, a stale or missing sweep amber).

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

## Coming up (REQ-PREDICT-001/002)

```
GET /api/forecast
```

What is forecast to reach its limit. For each reporting machine, warte fits a
straight line through the last week's hourly means of each disk's used share
(`disk.used_pct{mount=…}`) and of memory use (`mem.used_pct`). A forecast is
listed when the line rises steadily (R² at least 0.7), rests on at least 24
hours and 12 hourly points, and reaches 95% within 14 days at its current
rate, while the latest value is still below it. Each item carries its
evidence: `now`, `per_day`, `due_at`, `days`, `span_h`, `points` and `r2`.
Series with too little data are counted in `not_enough_data`, never
forecast. Within 3 days a forecast is an amber outage finding and within 1 day
a red one, so the alerts notify. Open like `/api/machines`; the overview shows
the same as its Coming up panel.

## Dependency canaries (REQ-PREDICT-003)

```
GET /api/canaries
```

The latest round of DNS checks configured in `[canaries]` (see deployment):
`report` holds every name through every resolver, by label, with its
`outcome` (`ok`, `timed_out`, `server_failure`, `no_such_name`, `refused`,
`no_address`, `failed`), `ms`, `failing_rounds` and `failing_since`;
`findings` names the likely cause of what has failed two rounds in a row: a
resolver, the dependency's DNS, or warte's network. Resolver addresses are
never shown. 503 without `[canaries]` or before the first round.

## Expiry dates (REQ-PREDICT-004)

```
GET /api/expiry
```

What runs out, soonest first: warte's own certificates, the endpoints and
files in `[expiry]` (every certificate in the chain, by label and place:
`leaf`, `intermediate 1`, `root`), the dates in `[expiry]`, and the token each
job last used, by the job's name. Each item has `kind` (`own`, `tls`, `pem`,
`date`, `token`), `what`, `part`, `expires_at`, and `problem` (`unreachable`,
`timed_out`, `handshake_failed`, `unreadable`, `no_certificate`) when it could
not be read. `findings` are the end dates within `amber_days` (amber) or
`red_days` (red). Addresses and paths are never shown.

## CI health (REQ-PREDICT-005)

```
GET /api/ci-health
```

This week (the last seven days) against the week before, from the
finished-job ledger, with no extra GitHub requests. `all` and `repos` carry
each week's finished jobs (successes and failures; cancelled and skipped are
not counted), failures and flaky jobs (failed, then passed when the run was
re-run). `waits` has each runner pool's 90th-percentile queue wait over first
attempts, and `release_this_p50_s` / `release_last_p50_s` the median release
run (workflows named release, first start to last end, successful runs only).
`warnings` lists only large changes resting on enough data: a failure rate of
10% or more that at least doubled (20 jobs a week or more), 3 or more flaky
jobs and more than before, a queue wait of 5 minutes or more that at least
doubled, a release run 50% and 5 minutes slower (5 samples a week or more).
Recomputed at most every 10 minutes; 503 without a CI source. The overview
shows it as its CI health panel.

## Architecture (REQ-ARCHVIEW-001)

```
GET /api/architecture
```

The `[architecture] file`, checked against `architecture.v1`: `architecture`
(the document), `crossings` (connection id to whether it crosses a trust
zone) and `findings` (zone crossings without authentication or encryption).
With a login it needs a signed-in user; without one, a warte token holding
`architecture:read`; otherwise `401`, and `403` for a token without the
right. `404` without `[architecture]`, `422` with `errors` when the file does
not check out. The contract and the checks: `warte docs architecture-view`.

## Attention schedule (REQ-ATTN-001)

```
GET /api/attention
PUT /api/attention
```

One person's attention schedule: decision windows, available hours, focus
blocks, off periods, timezone, `max_asking_flows` and `always_open_kinds`, in
the shape of pulseengine.eu's `attention.schema.json` (schema_version 1),
which the notice client reads before its cached copy, its local file and its
defaults.

- `GET`: a signed-in user reads their own. A warte token holding
  `agents:read` reads the schedule of the person its agent works for, named
  at mint time with `warte token mint <agent> --right agents:read --owner
  <subject>` (the subject is shown in the app under My hours). `404` when no
  schedule is set or the token names no owner, so the client keeps its local
  file; `401` with neither a login nor a token; `403` for a token without
  `agents:read`.
- `PUT`: only a signed-in user, for their own schedule, from this server's
  origin with the session's CSRF token in `X-Warte-CSRF` (the My hours panel
  sends both); otherwise `403`. A token never writes a schedule. The body is
  validated before anything is stored: HH:MM spans, `from` different from
  `to`, `max_asking_flows` 1 to 100, a timezone the system's zone database
  knows, `off.to` after `off.from`, no unknown field. A span written as
  `"HH:MM-HH:MM"` is stored as `{from, to}`. A broken field answers `422`
  naming the field, never its value. The change is logged as who changed
  which fields, without the values.

The app edits it under My hours, next to the sign-out link, as JSON so a
span's days and an off period's note are kept.

## Runner usage (REQ-CI-009)

```
GET /api/runner-usage?repo=<name>&from=YYYY-MM-DD&until=YYYY-MM-DD
```

How much runner time one repository used over a window of ledger days, and
how long its jobs waited for a runner, from the finished-job ledger with no
extra GitHub requests. A job counts when its run was created on a day from
`from` to `until`, inclusive; the window is at most 31 days.

- `runner_minutes`, `runs` and `minutes_per_run`: completed minus started,
  over every attempt that got a runner, since a re-run uses runner time too.
- `events`: the same per event (`pull_request`, `push`, `merge_group`, ...).
- `pr_branches` and `minutes_per_pr_branch`: the ledger has no pull request
  number, so a pull request's head branch stands for it.
- `pools`: per runner pool, `jobs`, `runner_minutes`, and the runner wait,
  started minus created over first attempts, as `wait_p50_s` and
  `wait_p90_s` with `wait_samples` (`null` without samples).
- `records`: the ledger records of the repository in the window.

Only public repositories are answered, from the same `type=public` listing
the attention strip uses; a private, archived or unknown name gets one 404,
so the endpoint never confirms that a repository exists, and before the
first listing nothing is answered. No runner names. Compare two windows to
see a change, for example the two weeks before and after a CI change. 400
for anything but a repository name and a valid window; 503 without a CI
source.

## Shared incidents (REQ-CI-005)

```
GET /api/incidents
```

Failed jobs from two or more repositories whose failing step carries the same
error signature, within an hour of each other, over the last day: one cause,
shown once. A signature is one of `resolver` (the host did not resolve),
`timeout`, `refused` or `server_error` (HTTP 500, 502, 503 or 504), for one
host, built from fixed patterns in the end of the job's log; no log text is
kept or shown. `incidents` lists `kind`, `host` (only for a well-known public
dependency or a name the dependency canaries watch, otherwise `null`),
`what` (the signature in words), `repos`, `jobs`, `first`, `last` and
`ongoing` (a failure within the last hour). Ongoing incidents also raise an
alert. The signature comes from the fleet agent where it runs; in GitHub-only
mode warte reads the log of each failed job of the last day once, at most 30
per ledger pass. 503 without a CI source.

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

## North Star (REQ-NS-001)

```
GET /api/north-star
```

The North Star's objectives with their published indicators, since v1.6.36.
Public, like the North Star itself. With `[north_star]` set, warte reads
pulseengine.eu's `data/north-star.toml` and `data/north-star/<id>.json` over
https every 6 hours and keeps every distinct observation (`north-star.jsonl`
next to the database). warte renders and does not compute.

Answer: `{rows, line, board, stale_after_days, last_poll}`. Each row has `id`,
`title`, `guard`, `state` and, when measured, `latest` and `previous`
observations (`measured_at`, `value`, `of`, `verifier`, `blind_spot`,
`guard_value`, `schema_version`; 0 until the publisher sends one; since
v1.6.40 also `period`, `evidence` and `not_counted`) and `age_days`. `board`
(v1.6.40, REQ-NS-005) is the published North Star as warte read it:
`north_star`, `updated`, `objectives` (with `summary`, `indicator` and the
declared state), `pullers` (products and tool repositories with the
objectives they pull) and `commitments` (period, layers, kind, target and,
when published, `target_value` and `target_of`).

- `unmeasured`: the North Star declares it unmeasured, there is no published
  file, or the file was gone at the last poll (its history is kept). Never 0.
- `stale`: the latest observation is older than `stale_after_days` (31 since
  v1.6.40, a month as the publisher's monthly check counts it; 45 before).
- `line` is the one-line summary the Overview shows, e.g.
  `O2 1/2 (2 d) · O4 1/839 (1 d) · 3 unmeasured`.
- `404` without `[north_star]`.

The North Star view (Overview line, or the North Star tab;
`GET /api/dashboard/north-star-partial`) shows the same files as
pulseengine.eu's /north-star page: one card per objective with the value as
"N of M" and a bar (never a percentage), the change since the previous value
and its age (red when stale), the evidence rows with their links, what is not
counted, the period's commitments, the products and tool repositories that
pull it, and the indicator, guard and blind spot, always shown. `off-target`
appears only when the current period's commitment publishes `target_value`;
`of = 0` reads "no repo in the denominator". The trend lists one value per
varve layer of the current period from warte's own observations, labelled
"observed by warte"; a layer without an observation shows a dash.

```toml
[north_star]
# base_url = "https://raw.githubusercontent.com/pulseengine/pulseengine.eu/main/data"
# interval_s = 21600
# stale_after_days = 45
```

## GitHub tokens (REQ-GHTOKEN-001)

`POST /api/agents/{name}/github-token` exchanges an agent's warte token
(right `github:token`, bound to its own name; `agents:any` does not lift it)
for a one-hour GitHub App installation token limited to the repositories the
operator assigned with `warte agents repos`. The body may only narrow. The
answer carries `Cache-Control: no-store`; 404 without `[github_app]`. See
`warte docs github-tokens`.

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

Up to v1.6.2, `warte serve` also mounted the absorbed hub page and API
under `/ci`. Since v1.6.3 it is gone; since v1.6.4 every `/ci` path answers
`410 Gone` with JSON `{"error":"gone","see":…,"reason":…}`, where `see` is the
replacement relative to warte's base URL, query kept. Use the routes above
(`/ci/api/eta` → `/api/eta`, `/ci/status.md` → `/api/status.md`,
`/ci/api/report` → `/api/report`, `/ci/report.md` → `/api/report.md`,
`/ci/api/history` → `/api/history/fleet`). `/ci/api/state` has no successor: the
whole snapshot carries raw log lines; the telemetry link and the timers are on the
Health view. `warte hub`, the standalone ground station, still serves the old API
on its own port (`warte docs ground-station`).

