# CLI reference

```
warte [OPTIONS] <COMMAND>

Commands:
  serve     Start the dashboard server
  status    Show factory health status
  agents    List registered agents
  schedule  Show the weekly cron schedule
  watch     Block until a PR's or branch's CI changes, then print JSON and exit
  token     Mint bearer tokens for agents (operator, on the ground station)
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
warte token mint <agent-id> --valid-hours 720   # see api-reference.md
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

On Linux the report also carries IO (REQ-PROBE-008): each block device
backing a reported disk, whether it spins, and since the previous report
its busy share, time per request and read/write rates (from
`/proc/diskstats`; `probe send` keeps the last sample as `io-prev.json` in
its state directory, so the first report after a start has no rates), and
IO pressure (`/proc/pressure/io`, some/full avg60). The Machines view
shows them; findings: a spinning disk at least 60 % busy or 100 ms per
request is amber, 90 % busy red; IO pressure at least 20 % amber, 50 % red.
The fields are optional, so older probes and ground stations keep working.

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

### Feeds: forwarding temper's report (REQ-TEMPER-002)

A probe can also forward a file another service on its machine writes.
Today that is temper's compliance report on temper's host:

```bash
warte probe send --ground https://<host>/warte \
  --feed temper=/var/lib/temper/report.json
```

After the machine report, the probe sends the file as one CCSDS segmented
sequence on APID 0x400 (the relay-ccsds codec the smithy agent's telemetry
uses), signed with its key, to `POST /api/probe/feed/temper`. A missing or
refused feed fails the run but does not hold back the machine report. Only
known feeds are accepted (`temper`); an unknown name is a usage error.

On the ground station, name the one host whose temper feed counts:

```toml
[temper]
host = "<the enrolled probe host on temper's machine>"
stale_after_s = 93600   # the last full sweep may be this old (26 h)
```

warte accepts the feed only from that host's probe, fresh, newer than the
last, from an unrevoked certificate, and only a valid `temper-report.v1`:
enums, identifiers, RFC 3339 times and hex, at most 500 repos with 40
checks each and 1 MiB. Anything else is rejected whole, nothing is stored,
and the input is never quoted back. The Compliance view and `GET
/api/compliance` show failed checks worst first, undecided checks, temper's
failed or stuck tasks, its review counts and its own GitHub API use. warte
itself makes no GitHub calls for this.

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

`POST /api/probe/enroll`, `/api/probe/report`, `/api/probe/renew` and
`/api/probe/feed/temper` are the only write endpoints usable without the
dashboard's login: the one-time token and the signatures are their
authentication. Every refusal gets the same answer;
the reason is in the server log.

### Agent activity hooks (DD-PROBE-003, REQ-PROBE-014)

```bash
warte probe install-hooks          # shows each change, asks before writing
warte probe install-hooks --yes
```

offers `warte probe hook` to every agent whose config folder exists, so the
probe can say what each session is doing:

| Agent | File | Events |
|---|---|---|
| Claude Code | `~/.claude/settings.json` | SessionStart, UserPromptSubmit, PreToolUse, Stop, Notification |
| Copilot CLI | `~/.copilot/hooks/warte.json` (its own file) | sessionStart, userPromptSubmitted, preToolUse, agentStop, notification (named with `--event`) |
| Mistral Vibe | `~/.vibe/hooks.toml` (two `[[hooks]]` appended) | pre_tool, post_agent |

It keeps every other hook and setting (in `settings.json` untouched keys
keep their exact text), backs each file up next to itself
(`*.warte-backup-<time>`), and adds nothing where the hook is already there.
The probe never installs it by itself.

The hook writes only a verb and a time per agent process: "waiting for you"
when the agent asks you (a permission prompt, a question, a form), "done"
when it finished its turn (shown as "idle" after 30 minutes), otherwise what
it does ("editing", "running tests", ...). No prompt, command, file name or
path leaves the machine. Vibe has no session-start or prompt event, so a
Vibe session shows its tools and "done" only.

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
hosts = { mini = "mini.local", "build-vps" = "build" }  # dir -> machine
```

(`WARTE_JOBS_DIR`, `WARTE_JOBS_EXPECTED` work too.) The Jobs view and
`GET /api/jobs` list every job worst first: **missing** (listed, no file),
**overdue** (no success within the window, so a stopped timer or a host
that is down shows up), **failing** (the last run failed, a success is
still within the window) and **ok**. Files that are not listed are shown
with their own `expect_within_s`. The airport puts each job on its
machine's pier and makes a late one the pier's note.
