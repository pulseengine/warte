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
| `0x401` | security findings feed |

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
| `br` | string? | since v1.6.34, optional: the run's head branch (100). Absent from older agents |

A day manifest (APID `0x231`) is `{day, n, x, closed}`: the count of the day's
records, the XOR of their ids, and whether the agent will look at the day again.
A day counts as complete only when the hub's records match both.

Changes: v1.6.21 added `sig`; v1.6.34 added `br`.

## Contract: fleet agent transport v2 (QUIC, mutual TLS, relay-sec)

Transport **2**, decided 2026-11-22 (REQ-SEC-001, DD-TRANSPORT-001, #41). It
carries the same CCSDS packets as wire v1, byte for byte; only how they travel
changes. The hub accepts v1 over UDP and v2 over QUIC side by side until the
operator sets `require_quic`.

### Identity

- Each fleet agent enrolls once with `warte probe enroll` and a one-time token,
  as probes do today, and receives a client certificate from warte's CA. The
  certificate's common name is the agent's name. The enrollment answer also
  carries, once, the agent's **SPI** (a 16-bit number unique to that agent)
  and its **SA key** `k_id` (16 random bytes, as 32 lowercase hex
  characters), relay-sec's long-term per-agent key. The agent keeps `k_id` in
  a file only its user can read and never logs it; the hub keeps it in its
  database beside the SPI.
- `warte probe revoke <name>` revokes the certificate, the SPI and `k_id`; a
  revoked agent's connections are refused from then on.

### Connection

- QUIC (RFC 9000) on its own UDP port, TLS 1.3 only, mutual authentication.
- ALPN `warte-telemetry/1`; any other is refused.
- The server certificate comes from warte's CA, the same one the agent already
  pins as `ca.pem`.
- A client certificate that does not chain to warte's CA, or is revoked, is
  refused and counted `tls_identity`.

### relay-sec session

- The agent opens the first bidirectional stream and runs relay-sec's session
  handshake on it as the initiator (SPI, channel 0, `k_id`); the hub answers
  as the responder. Each message is a u16 big-endian length, then:
  - Hello (agent to hub), 56 bytes: ephemeral x25519 public key (32) ‖ epoch
    (u64, big-endian, 8) ‖ tag (16);
  - Response (hub to agent), 56 bytes: the same layout with the hub's key
    and epoch;
  - Confirm (agent to hub), 16 bytes: the tag.
- Epochs: each side keeps a counter on disk that never resets and goes up by
  at least one before every handshake, as relay-sec requires. The hub refuses
  a Hello whose epoch is not above the last one it accepted for that SPI
  (counted `replayed`).
- The handshake's `SessionKeys.mac` keys two relay-sec channels in MAC-only
  mode: channel 1 for DATAGRAM frames and channel 2 for the ledger stream,
  each with its own send counter and anti-replay window. The payloads are
  already encrypted by TLS.
- A new connection is a new session with new keys, so an agent restart never
  reuses a counter.
- The agent floors its epoch at the current time in milliseconds, as the hub
  does: each new epoch is max(stored + 1, unix ms). A lost or rolled-back
  counter then still yields an epoch above the hub's last one for that SPI, as
  long as the agent's clock is right (run NTP).
- An agent that lost its `k_id` (a reinstall, a restore without its state)
  re-enrols with a new one-time token and gets a new SPI and `k_id`; enrolling
  again under the same name revokes the old SPI. The hub's last epoch is never
  reset by hand, so no rollback of the agent's state can replay an old session.- An SPI the hub does not know for this certificate is refused and counted
  `unknown_spi`.

### Framing

Each sealed frame is relay-sec's `SecurityHeader ‖ CCSDS packet ‖ tag`: an
11-byte header and a 16-byte Ascon tag around exactly one wire v1 packet.

| What | How |
|---|---|
| Housekeeping: host snapshot, runners, queue, timers, temper feed | one QUIC DATAGRAM frame (RFC 9221) per sealed packet; unreliable, no head-of-line blocking |
| Ledger records and day manifests | one unidirectional stream per pass, carrying u16 big-endian length-prefixed sealed frames; reliable |

A DATAGRAM frame is at most 1200 bytes, so the CCSDS packet inside it is at
most **1173 bytes** (1200 − 11 − 16). Senders pack housekeeping to fit.
Ledger frames on a stream may be up to the v1 limit.

### What the hub checks

1. The TLS handshake: chain to warte's CA, not revoked, ALPN. Otherwise the
   connection is refused, counted `tls_identity`.
2. The SPI is the one enrolled for this certificate (otherwise `unknown_spi`),
   the handshake verifies under its `k_id`, and the epoch went up (otherwise
   `replayed`). Otherwise the connection is closed.
3. Each frame's tag under the session keys. Otherwise the frame is dropped,
   counted `bad_tag`.
4. relay-sec's anti-replay window. A frame seen before, or too far behind,
   is dropped and counted `replayed`.
5. The CCSDS packet goes to the v1 assembler unchanged.

The counts `tls_identity`, `unknown_spi`, `bad_tag` and `replayed`, the
number of agents connected over QUIC, and the v1 packets still arriving over
UDP show on Health.

### Coexistence

- `[hub] quic = "<address:port>"` turns the QUIC listener on beside the v1
  UDP listener.
- `[hub] require_quic = true` drops v1 UDP packets and counts them. The
  operator sets it, with the owner's go, once the v1 count stays at 0.

### Running transport v2

```toml
[hub]
udp = "192.0.2.1:8094"             # wire v1 stays on while agents move
quic = "192.0.2.1:8095"            # transport v2 listener
quic_names = ["ground.example"]   # the names agents dial; in the server certificate
require_quic = false              # true once wire v1 stays at 0
```

- The server certificate comes from warte's CA at start-up, for the names in
  `quic_names`, and agents verify it against the `ca.pem` they got at
  enrolment.
- `warte probe enroll` writes `telemetry-sa.json` (the SPI and `sa_key`, mode
  0600) next to the probe key. An agent that enrols another way reads `spi`
  and `sa_key` from the enrolment answer.
- The hub keeps its own epoch in `quic-epoch` in its data directory, and each
  new epoch is at least the current time in milliseconds. A restore of an older
  `quic-epoch` therefore never repeats an epoch, as long as the hub's clock is
  right (run NTP); agents need not re-enrol after a hub restore.
- The counts are under Health, Ground station internals, Transport, and in
  `GET /api/state` (`udp_v1`, `udp_dropped`, `require_quic`, `quic`).

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

Unchanged from the original hub; the contract is `/api/openapi.json` (0.1.0). Since
v1.6.44 its description names no runner host; paths and schemas are the same.

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
