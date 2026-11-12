# Ground station (absorbed hub)

`warte hub` is the CI fleet's ground station. It began as a separate hub and was
absorbed into warte in v1.4.0 (DD-SMITHY-001).

## Contract: fleet agent wire v1

Version **1**, since warte v1.4.0; last change in v1.6.21. A fleet agent pins
against this version. A change that an older agent or hub could misread bumps
it; adding an optional field does not.

Packets are CCSDS space packets (`relay-ccsds`) over UDP, one APID per stream,
each with its own 14-bit sequence counter. JSON payloads are at most 1200 bytes;
a long list is split over several packets.

| APID | Stream |
|---|---|
| `0x100` | host snapshot (load, PSI, disks) |
| `0x110` + id | one runner (id 1..0x6F) |
| `0x200`, `0x201` | queue scan, queue detail |
| `0x210` + n, `0x220` + n | hosted and queued jobs |
| `0x230` | ledger: finished-job records |
| `0x231` | ledger: day manifests |
| `0x300`..`0x304` | timers, dispatcher, watchdog, bazel, trace |
| `0x400` | temper feed |

A ledger record (APID `0x230`) is one finished job:

| Field | Type | |
|---|---|---|
| `d` | string | UTC day the run was created, `YYYY-MM-DD` (at most 10 bytes) |
| `id` | u64 | GitHub job id, unique per attempt |
| `run`, `att` | u64, u32 | run id and attempt |
| `repo` | string | repository name without the org (60) |
| `wf`, `job` | string | workflow and job name (60, 80) |
| `pool` | string | runner pool, `hosted:<label>` for GitHub-hosted (40) |
| `runner` | string | runner name; empty when none (40) |
| `ev` | string | triggering event (24) |
| `c`, `s`, `e` | i64, i64?, i64? | created, started, completed (unix seconds) |
| `r` | string | conclusion (16) |
| `sig` | string? | since v1.6.21, optional: a failed job's error signature `kind:host`, `kind` one of `resolver`, `timeout`, `refused`, `server_error` (270). Never log text; anything else is ignored |

A day manifest (APID `0x231`) is `{day, n, x, closed}`: the count of the day's
records, the XOR of their ids, and whether the agent will look at the day again.
A day counts as complete only when the hub's records match both.

Changes: v1.6.21 added `sig`.

## Data path

The fleet agent runs on the CI host. Once a minute it collects
a snapshot (machine load and PSI, disks, every runner with its job and labels, the
org-wide queue scan, automation timers), and it reads finished jobs back from GitHub
per UTC day. It **dials out** to the ground station with CCSDS space packets over UDP
(`relay-ccsds` from pulseengine/relay), so the CI host exposes nothing.

The ground station:

- accepts telemetry only from `--allow` addresses (the CI host's WireGuard tunnel IP)
  and counts everything else (`foreign` in `/api/state`);
- keeps the latest snapshot plus one history point per minute for 7 days in
  `hub-state.json`, written every 5th frame;
- keeps every finished job in `ledger/YYYY-MM-DD.jsonl`, deduplicated by job id.
  GitHub is the store of record, so the ledger can be rebuilt;
- answers "when will PR X's next job start" from **measured** waits in that pool,
  as a range with its basis, never from queue position.

## API

Unchanged from the original hub; the contract is `/api/openapi.json`.

| Path | |
|---|---|
| `/api/eta?repo=<name>&pr=<n>` (or `branch=`) | Next-job start estimate for a PR or branch |
| `/status.md` (`?repo=&pr=` for one PR) | Fleet status / PR status as Markdown |
| `/api/state` | Latest snapshot plus telemetry link statistics |
| `/api/history?hours=<1..168>` | Per-minute trend points |
| `/report.md`, `/api/report?month=YYYY-MM` | Monthly conveyor-belt report |
| `/healthz` | Liveness |

`repo` is the name without the org. Answers carry `recheck_after_s`; agents use it to
schedule their next look instead of polling GitHub (see the `ci-status` skill).

## Security

- **Telemetry is authenticated only by source address.** UDP source addresses can be
  spoofed, so anything that can reach `--udp` could inject a fake snapshot or ledger
  records. Bind `--udp` to the WireGuard interface address and firewall it to the
  tunnel. Packet authentication is tracked separately.
- Ledger records with impossible values (outside `[0, now + 1 day]`, or a run that ends
  before it starts) are rejected on ingest and when the ledger is read from disk, and
  the report caps how far one record is spread. Records where `started_at` precedes
  `created_at` are kept: GitHub produces them for re-run attempts.
- HTTP defaults to loopback (`127.0.0.1:8091`); publish it through Caddy. Only `GET` is
  served, as in the original hub; any other method gets a 404.
- If a panic poisons the hub's state lock, the process exits so systemd restarts it from
  disk, as the original hub's single-threaded server did, rather than serving half-dead.

## Inside `warte serve`

With a `[hub]` section in warte.toml, `warte serve` runs the ground station in-process,
answers under warte's own API (`/api/eta`, `/api/status.md`, `/api/report`,
`/api/report.md`, `/api/history/fleet`; see `warte docs api-reference`), and the
dashboard's Queue, Critical Path, Ask, CI history and Health views read from it.
Up to v1.6.2 it also mounted the old hub page and API under `/ci`; that is retired
(REQ-SMITHY-007):

```toml
[hub]
udp = "<wireguard-address>:8092"
allow = ["<ci-host-tunnel-ip>"]
data_dir = "/var/lib/warte-hub"   # default
org = "pulseengine"                # default
```

Without it, those views say the ground station is not configured.

## Running

```bash
warte hub --udp <wireguard-address>:8092 \
  --allow <ci-host-tunnel-ip> --data-dir /var/lib/warte-hub
```

The flags and the data directory are the original hub's, so `warte hub` reads an existing
`/var/lib/warte-hub` as-is and replaces the original hub in its systemd unit. Unlike the original hub,
`--udp` has no default and `--http` defaults to loopback. Keep HTTP on
loopback and publish it through a reverse proxy if you need it beyond the host.
