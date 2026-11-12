# Machine history: numbers over time, not just the current state

Status: approved by the owner 2026-11-05, with the owner's change: warte owns probe, store and
charts; the private configuration repo only deploys it (install, pin, configure, back up).

## Why

The Machines view shows each machine as it is right now. warte keeps only the latest report
per machine, so an amber or red finding says *that* something is wrong but not *when* it began,
how fast it grew, or what climbed before it. Beszel, which this replaces, kept a few series
over time and the owner found it too thin. Incidents needed more than Beszel had: memory
pressure next to IO pressure to tell exhaustion from a wedged runner, swap, per-runner
busy/idle state, and weeks of data for scheduling analysis (a capacity analysis needed weeks;
the ground station keeps 7 days).

## Goals

- Every number a probe reports can be scrolled back through: last hour, day, week, 90 days.
- Each machine card on the Machines view gets small trend lines with one shared time-range
  switch (1 h · 24 h · 7 d · 90 d); a machine detail page shows larger charts.
- A finding links to the chart of the number behind it.
- Outages show as gaps, never as a line drawn across them.
- Works the same with or without a fleet agent, so the owner's work site gets charts too.

## Non-goals

- Alerting on trends (forecasting "disk full in 3 days"). Possible later.
- New probe metrics. Network, temperatures, top processes and system health come next, once
  history exists (owner's order).
- A general metrics database: the store holds warte's own series only.

## Anti-goals (a failure even if it works)

- Paths, process ids, directories, command lines or raw error text in the store, the charts or
  any unauthenticated API (warte#46 rules apply to series too).
- Wearing out the hub host's SD card with per-sample writes.
- History slowing report ingestion: a store problem must never reject or delay a probe report.

## Owner's decisions

1. warte collects, stores and draws the history; the private configuration repo only deploys warte (2026-11-05,
   reversing an earlier "the deployment stores" choice the same day).
2. Retention 90 days or more.
3. First step: history and charts for the numbers probes already send.
4. Store engine: redb (pure Rust), not SQLite.
5. Probe rollout waits for one release that carries the inventory and history together.

## Design

### Flow

```
probe ──signed report──▶ warte ingest (verify, latest-state as today)
                           │ project to numbers only
ground-station snapshot ──▶├──▶ history store (redb, own file in the data dir)
                           ▼
              warte dashboard ◀── series read (in-process)
```

- On every accepted report, warte projects it to a sample: host, time, and a set of named
  numeric series. The projection is a whitelist; nothing else is kept.
- Accepted ground-station snapshots are projected the same way (host = the snapshot's host).
- The existing 7 days of ground-station points are imported once on first start.

### Store

- redb, one file `history.redb` next to warte's other state, separate from the SQLite db.
- Tables: `minute` keyed `(host, t)` → one encoded sample (all series of that host at that
  minute); `hour` keyed `(host, t)` → min/avg/max per series; a small `names` table mapping
  series names to ids.
- Writes are buffered: samples commit with `Durability::None`, and an `Immediate` commit runs
  every 5 minutes and on shutdown. A crash loses at most 5 minutes of history, never a report.
- Retention: minutes kept 7 days, hours 90 days or more (configurable); hourly rollup and
  pruning in one job every hour.
- A store error is logged and counted, shown as one amber finding, and never fails ingestion.

### Read API

warte serves, inside its existing API:

- `GET /api/history/machines` → hosts with series.
- `GET /api/history/machines/<host>/metrics` → series names and units.
- `GET /api/history/machines/<host>/series?metric=a,b&from=&to=` → per metric
  `{metric, unit, t0, step, values:[…], min?:[…], max?:[…]}`; unix seconds UTC; a gap is
  `null`; the server picks `step` (60 s within 7 days, 3600 s beyond) and says which.

Same exposure as `/api/machines`: numeric series only, so nothing new is published.

### Series in step 1

From every probe: load (1/5/15) and load per core (`cpu.ncpu`), memory used %, swap used,
disk used % per mount, disk busy % / await / read and write KB/s per device (Linux), IO
pressure some/full, agent session count, uptime (reboots show as resets).

From the CI host snapshot: CPU and memory pressure, runner busy/idle/offline counts, per-runner
CPU and memory, queued jobs (self-hosted and hosted), oldest wait.

Probes send IO pressure only; CPU and memory pressure exist only for the CI host until the probe
adds them (next step).

### Metric names

Dotted names with optional labels: `load.1`, `load.per_core`, `mem.used_pct`, `swap.used_mb`,
`disk.used_pct{mount=/var}`, `io.busy_pct{dev=sda}`, `psi.io.some`, `sessions`,
`runner.cpu{id=3}`. The read API takes the full name; `/metrics` lists them. A series a machine
does not send is simply not drawn, never a flat zero.

### Point time

The probe's `taken_at`, bucketed to the minute; if it differs from warte's receive time by more
than 5 minutes, the receive time is used. Host is the report's host, as on the Machines view.

### warte dashboard

- Machine card: three sparklines (load per core, memory, fullest disk) for the selected range.
  "Fullest disk" is the mount with the highest latest `disk.used_pct`.
- Machine detail: a chart per group (CPU, memory and swap, disks, IO, sessions; for the CI host
  also runners and queue), shared time axis, hover shows value and time.
- Charts rendered server-side as inline SVG through HTMX partials, consistent with the existing
  views; no JS charting library.
- Range switch remembered per viewer (browser storage, best effort).
- Disk, load, memory and IO findings carry a metric name; the link opens 24 h, or from the
  alert's opened time when the alerts agent has one.

### Privacy

Series carry host, series name, unit and numbers only. Mount points and device names are
already shown on the Machines view and stay. Sessions contribute only their count. A leak test
feeds a report holding a secret path and error text and asserts neither reaches the store or
the API.

## The deployment's part (deployment only)

- Nothing new to build. The existing warte role keeps deploying warte; it backs up
  `history.redb` with the rest of warte's data dir and may set the retention.
- Mac probes managed by the private configuration repo: release binaries for macOS arm64 exist.

## Left to the builder

- Encoding of a sample value (e.g. postcard or a compact hand-rolled format), table layout
  details, and the series-name id scheme.
- Sparkline sizes and colours (dashboard tokens).
- rivet artifacts: requirements for the store, the read API and the charts, linked to
  REQ-PROBE-001.
