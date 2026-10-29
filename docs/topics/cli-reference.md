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
  `access`, `migrate`, `probe`, `hub`, `artifact`, `docs`

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

Sending to the ground station (QUIC, mutual TLS, relay-sec), enrollment
with a one-time token, and agent working/waiting state from hooks you
install are the next steps (DD-TRANSPORT-001, DD-PROBE-002, DD-PROBE-003).
