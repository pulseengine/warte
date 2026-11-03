# CLI reference

```
warte [OPTIONS] <COMMAND>

Commands:
  serve     Start the dashboard server
  status    Show factory health status
  agents    List registered agents
  schedule  Show the weekly cron schedule
  watch     Block until a PR's or branch's CI changes, then print JSON and exit
  access    Manage access rights
  migrate   Migrate data from an existing system
  probe     What this machine is and runs: hardware, disks, agent CLIs, sessions
  fleet     The fleet model: read a spar instance of the Warte_Fleet types
  artifact  Manage artifacts
  docs      Show embedded documentation

Options:
  --config <CONFIG>  Config file path [default: warte.toml]
  --format <FORMAT>  Output format [text, json] [default: text]
  -h, --help         Print help
  -V, --version      Print version (warte <semver>)
```

## Conventions

- `--version` / `-V` prints `warte <semver>` and exits 0
- `--help` exits 0
- Unknown flags exit 2
- Structured output via `--format json`
- Subcommands: `serve`, `status`, `agents`, `schedule`, `watch`,
  `access`, `migrate`, `probe`, `fleet`, `hub`, `artifact`, `docs`

## Examples

```bash
warte serve --bind 0.0.0.0:8080
warte status --format json
warte agents --health unhealthy
warte access grant <agent-id> --rights "artifacts:download"
warte migrate temper --path /etc/temper/config.yml
warte hub --udp <wireguard-address>:8092 --allow <ci-host-tunnel-ip> --data-dir /var/lib/smithy-hub
warte docs architecture
```

## warte watch

Blocks until a PR's or branch's CI runs start or finish, prints one JSON
line and exits. It is meant for agents: run it as a background command
and be woken when it exits, instead of polling `gh pr checks`.

```bash
warte watch --ci --repo spar --pr 511 --url https://<host>/warte
```

| Exit | Meaning |
|------|---------|
| 0 | the CI state changed; the JSON says `from`, `status` and the runs |
| 3 | `--timeout` (default 3600 s) passed with no change |
| 2 | usage error |
| 1 | error (server unreachable, bad answer, no ground station) |

Every answer carries a `cursor`. Pass it back with `--since` on the next
call and no change between the two calls is missed. `status` is one of
`queued`, `running`, `running+queued`, `not_queued` (the PR has nothing in
flight) or `unknown` (nothing seen, but the queue scan was incomplete).
The resolution is one ground-station snapshot, about a minute. `--url`
defaults to `WARTE_URL`, then to the configured bind address. Behind a reverse proxy with its own local CA (e.g. Caddy's `tls internal`), pass that CA with `--ca-cert <pem>` or `WARTE_CA_CERT`; it is trusted in addition to the built-in roots. There is no switch to skip certificate checks. The server
side is `GET /api/watch`; see `/api/openapi.json`.

## warte probe

`warte probe report` collects once and prints what this machine is and
runs: OS, kernel and architecture, CPU and load, memory and swap, disks,
the agent CLIs and tools on PATH (`claude`, `copilot`, `codex`, `gh`,
`varve`) with their versions, and the agent sessions running now. It is
read-only and sends nothing. For agent processes it reports the agent, pid,
start time and working-directory name, never command lines, which can
carry prompts and tokens.

```bash
warte probe report
warte --format json probe report
```

### Enrollment (DD-PROBE-002)

On the ground station, mint a one-time token for the machine:

```bash
sudo -u warte warte probe token --host wohl      # valid 15 min, used once
```

On the machine, enroll with it. The key is made here and never leaves:

```bash
warte probe enroll <token> --ground https://<host>/warte [--ca-cert ca.pem]
# key (0600), certificate and the ground station's CA in /var/lib/warte-probe
```

The certificate names the host the token was minted for, whatever the
request says, and is valid for 30 days. A token works once, for one host,
for 15 minutes; only its SHA-256 is stored.

### Sending reports (REQ-PROBE-006)

Once enrolled, the machine sends its report, signed with its key:

```bash
warte probe send --ground https://<host>/warte [--ca-cert ca.pem] \
  [--state-dir /var/lib/warte-probe]
```

Run it every minute (launchd on macOS, a systemd timer on Linux). The
ground station stores a report only if the certificate was issued by its
CA and is valid, the certificate names the report's host, the ECDSA P-256
signature over the exact report bytes verifies, and the report was taken
within 10 minutes and is newer than the last one from that host (no
replays). A signature, not mutual TLS, so it works behind a reverse proxy
that terminates TLS.

### Renewal and revocation (REQ-PROBE-007)

Certificates last 30 days. Once two thirds have passed, `warte probe send`
renews first: it signs a new CSR for the same key, with that key, and posts
it with the current certificate to `POST /api/probe/renew`. No token is
needed. The new certificate replaces the old one atomically; a failed
renewal is printed and tried again on the next run, while the current
certificate still works. An expired certificate cannot renew: that machine
needs a new token.

```bash
warte probe revoke --host <host>     # on the ground station
```

refuses every certificate the host holds now, for reports and renewals,
until it enrolls again with a new token (`warte probe token --host <host>`).

`POST /api/probe/enroll`, `POST /api/probe/report` and `POST
/api/probe/renew` are the only write endpoints usable without the
dashboard's login: the one-time token and the signatures are their
authentication. Every refusal gets the same answer;
the reason is in the server log. Agent working/waiting state from hooks
you install follows (DD-PROBE-003).

## warte fleet

The fleet a deployment intends (machines, where they live, which bench
board is attached to which machine) is an AADL model of the `Warte_Fleet`
types in `architecture/fleet/warte_fleet.aadl`, instantiated with spar:

```bash
spar instance --root My_Fleet::MyFleet.impl --format json \
  warte_fleet.aadl my_fleet.aadl > fleet.json
warte fleet show fleet.json          # exit 1 when the model has problems
```

Point `fleet_model` (or `WARTE_FLEET_MODEL`) at `fleet.json` and warte
compares it with what reports in: a modelled machine that is not
reporting, a reporting machine that is not modelled. The Machines view
shows the differences and draws modelled-but-silent machines dashed;
`GET /api/fleet` serves the same. spar does the modelling; warte draws it.

## Scheduled jobs (REQ-JOBS-001)

Hosts push one state file per job to the ground station
(`<dir>/<host>/<job>.json`, as `pe-job` writes them), and a manifest
lists the files that must exist and how long each may go without a
success:

```toml
[jobs]
dir = "/var/lib/pulseengine/jobs"
expected = "/etc/pulseengine/jobs-expected"   # "<path> <window_s>" per line
hosts = { mini = "Mac.fritz.box", "netcup-vps" = "netcup" }  # dir -> machine
```

(`WARTE_JOBS_DIR`, `WARTE_JOBS_EXPECTED` work too.) The Jobs view and
`GET /api/jobs` list every job worst first: **missing** (listed, no file),
**overdue** (no success within the window, so a stopped timer or a host
that is down shows up), **failing** (the last run failed, a success is
still within the window) and **ok**. Files that are not listed are shown
with their own `expect_within_s`. The airport puts each job on its
machine's pier and makes a late one the pier's note.
