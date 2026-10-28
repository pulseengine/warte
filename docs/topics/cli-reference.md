# CLI reference

```
warte [OPTIONS] <COMMAND>

Commands:
  serve     Start the dashboard server
  status    Show factory health status
  agents    List registered agents
  schedule  Show the weekly cron schedule
  watch     Open the dashboard in the browser
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
