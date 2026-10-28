# Ground station (absorbed smithy hub)

`warte hub` is the CI fleet's ground station. It was smithy hub (`pulseengine/smithy`
`tools/smithy`) and was absorbed into warte in v1.4.0 (DD-SMITHY-001).

## Data path

The smithy agent runs on the CI host (`pulseengine-ci-01`). Once a minute it collects
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

Unchanged from smithy hub; the contract is `/api/openapi.json`.

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
  served, as in smithy hub; any other method gets a 404.
- If a panic poisons the hub's state lock, the process exits so systemd restarts it from
  disk, as smithy hub's single-threaded server did, rather than serving half-dead.

## Inside `warte serve`

With a `[hub]` section in warte.toml, `warte serve` runs the ground station in-process,
mounts the hub API under `/ci` (read-only, outside warte's auth, as on fourpi), and the
dashboard's Queue, Critical Path, Ask and Health views read from it:

```toml
[hub]
udp = "<wireguard-address>:8092"
allow = ["<ci-host-tunnel-ip>"]
data_dir = "/var/lib/smithy-hub"   # default
org = "pulseengine"                # default
```

Without it, those views say the ground station is not configured.

## Running

```bash
warte hub --udp <wireguard-address>:8092 \
  --allow <ci-host-tunnel-ip> --data-dir /var/lib/smithy-hub
```

The flags and the data directory are smithy hub's, so `warte hub` reads an existing
`/var/lib/smithy-hub` as-is and replaces `smithy hub` in its systemd unit. Unlike smithy,
`--udp` has no default and `--http` defaults to loopback. Keep HTTP on
loopback and publish it through Caddy (`https://fourpi.local/ci/`).
