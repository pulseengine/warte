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
  news      News from the tools' public feeds, newest first (contract news v1)
  nav       The home link and app switcher file (contract nav v1)
  sessions  The agent session list (contract sessions v1)
  artifact  Manage artifacts
  docs      Show embedded documentation

Options:
  --config <CONFIG>  Config file path; it must exist when named [default: warte.toml in the
                     working directory, if there is one] [env: WARTE_CONFIG=]
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
  `access`, `migrate`, `probe`, `fleet`, `news`, `nav`, `sessions`, `hub`, `ci-agent`, `artifact`, `docs`

## Examples

```bash
warte serve --bind 0.0.0.0:8080
warte status --format json
warte agents --health unhealthy
warte token mint <agent-id> --valid-hours 720   # see api-reference.md
warte access grant <agent-id> --rights "artifacts:download"
warte migrate temper --path /etc/temper/config.yml
warte hub --udp <wireguard-address>:8092 --allow <ci-host-tunnel-ip> --data-dir /var/lib/warte-hub
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
defaults to `WARTE_URL`, then to the configured bind address. Behind a reverse proxy with its own local CA (e.g. Caddy's `tls internal`), pass that CA with `--ca-cert <pem>` or `WARTE_CA_CERT`; it is trusted in addition to the operating system's trust store. There is no switch to skip certificate checks. The server
side is `GET /api/watch`; see `/api/openapi.json`.

## warte admit

`warte admit` asks warte for admission before a pull request is opened
(REQ-QUEUE-005, #70), prints the JSON verdict and exits with it, so a chained
`gh pr create` cannot skip a `batch` or `hold`:

```sh
warte admit --repo spar --kind fix && gh pr create ...
until warte admit --repo spar --kind feature; do sleep 600; done   # wait out a hold
```

Exit codes: `0` open, `10` batch, `11` hold, `1` anything else (warte
unreachable, a refused request, an unknown verdict): an error is never a go.
`--kind` is `docs`, `fix`, `feature`, `security` or `red-main`. It shares
`--url`/`WARTE_URL` and `--ca-cert`/`WARTE_CA_CERT` with `warte watch`.

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

### Night window from Focus (REQ-FOCUS-001)

`warte probe focus` (macOS) derives the attention schedule's `available`
hours from the night Focus's schedule: the day outside the night window,
as one span (22:30 to 06:30 at night gives `06:30-22:30`). It reads only
`~/Library/DoNotDisturb/DB/ModeConfigurations.json`, which needs Full Disk
Access, and sends nothing anywhere.

```sh
warte probe focus                                   # print the derived span
warte probe focus --write ~/.config/pulseengine/attention.json
warte probe focus --mode "Do Not Disturb"           # another Focus, by name or id
```

The default night Focus is Sleep, by its identifier
`com.apple.sleep.sleep-mode` (its name is localized). `--write` replaces only
`available` and keeps every other key's value as it was; it never writes
`off`. Anything in doubt writes nothing and exits 1 with the reason: an
unreadable file, a format that changed, no enabled schedule, a Sleep Focus
driven by the Health sleep schedule (which has no times in this file), more
than one schedule, or a schedule for some days only (its day order is not
confirmed yet).

### Disk pressure (REQ-PRESSURE-002)

`warte probe pressure` shows, on this machine and without any network call,
each writable volume's free space and verdict (`ok`, `low` under 20 GB free,
`critical` under 10 GB; `WARTE_PRESSURE_LOW_GB` and
`WARTE_PRESSURE_CRITICAL_GB` change them) and what agents hold, largest first:

- `scratchpad`: a Claude Code session's directory under `/tmp/claude-<uid>/`
  (scratchpad and task outputs), with its session id;
- `target`: `target/` in a checkout a live agent session works in;
- `worktree`: a git worktree of such a checkout.

Each holder has its volume, GB, path, the live agent sessions working there
(`agents`; `live`, and `pid` when there is exactly one) and `complete` (false
when sizing ran out of time, so `gb` is a floor), and since v1.6.32 a
`verdict` with `why` (REQ-PRESSURE-003, contract disk-hygiene v1): `in-use`
when a live session or any process works there, `dirty` for a worktree with
uncommitted changes or commits on no remote, `unknown` when git could not
tell, else `idle`. Only `idle` may be removed; `live` alone is no cleanup
verdict (`warte docs disk-hygiene`). It reads what the probe
stored: `probe send` sizes holders only while a volume is low, at most every
10 minutes, within 60 seconds, on the low volumes only. `--measure` sizes them
now, on every volume. Sizing follows no symlink and stays on the volume.
Nothing is ever deleted. `host` is the name the probe enrolled under.

```bash
warte probe pressure
warte probe pressure --format json
warte probe pressure --measure
```

The probe state directory defaults to `~/Library/Application Support/warte-probe`
on macOS when it exists, else `/var/lib/warte-probe`; `--state-dir` or
`WARTE_PROBE_STATE_DIR` sets it. The stored holders are readable by the
probe's user only. On Linux the probe runs as its own user and cannot read
other users' scratchpads, so there `--measure` as the agents' user is the way
to see them. The ground station receives only the totals per kind per volume
(`GET /api/pressure`).

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

### Security findings (REQ-FINDINGS-001)

Detectors on a machine (a baseline-drift check, AIDE, Falco) write one
`security-findings.v1` file, and the probe forwards it as the `security`
feed (APID 0x401, signed like every feed):

```bash
warte probe send --ground https://<host>/warte \
  --feed security=/var/lib/detectors/findings.json
```

```json
{"schema": "security-findings.v1", "generated_at": "2026-11-22T02:00:00Z",
 "findings": [{"source": "drift", "rule": "new-ssh-key", "severity": "high",
               "subject": "SHA256:...", "first_seen": "2026-11-22T01:00:00Z",
               "last_seen": "2026-11-22T02:00:00Z", "count": 1}]}
```

`source` is `drift`, `aide`, `falco` or `other`; `severity` is `low`,
`medium`, `high` or `critical`; `rule` is at most 80 characters, `subject` at
most 200; at most 500 findings, so summarize a chatty detector. Metadata only:
an unknown field, such as file contents, refuses the whole report. Any
enrolled, unrevoked host may send its own findings; each send replaces that
host's set, and one not newer than the last is refused as a replay.

Machines, Security shows the open findings, highest first; the owner can
accept one into that host's baseline so it stops alerting. High and critical
open findings, and a host whose findings stopped for over 36 hours (the
detector may have died), are items on the night glance.

### Feeds: forwarding temper's report (REQ-TEMPER-002)

A probe can also forward a file another service on its machine writes.
Today that is temper's compliance report on temper's host:

```bash
warte probe send --ground https://<host>/warte \
  --feed temper=/var/lib/temper/report.json
```

After the machine report, the probe sends the file as one CCSDS segmented
sequence on APID 0x400 (the relay-ccsds codec the fleet agent's telemetry
uses), signed with its key, to `POST /api/probe/feed/temper`. A missing or
refused feed fails the run but does not hold back the machine report. Only
known feeds are accepted (`temper`, `security`); an unknown name is a usage
error.

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

When `[temper]` is set and no report has arrived within `stale_after_s` of
the ground station first listening for one (kept in `temper-since` in the hub
data directory since v1.6.33, so a restart does not reset it), or the last
one is older than that, it is an
outage (REQ-TEMPER-003): a red finding in Outages, notified by the outage
alerts. temper sweeps every 12 h, so `stale_after_s = 46800` (13 h) catches a
stopped temper within an hour of its missed sweep.

### Outages from the machines (REQ-PROBE-015, REQ-PROBE-016)

Red machine findings are outages too: Outages lists them under "machines
and feeds" and the outage alerts notify them. Amber findings are drift and
stay on the Machines view.

- **Reboot required:** on Linux the probe reports why: Debian's
  `/var/run/reboot-required` (with the number of packages in
  `reboot-required.pkgs`), or a newer kernel of the running kernel's flavour
  installed in `/boot` than the one running. The age comes from those files,
  so it holds across probe restarts. Amber at once, red after 7 days.
- **Crash loops:** a service under `Restart=` that keeps failing never
  reaches `failed`. The probe reports each service whose restart count
  (`NRestarts`) rose since its previous report, or that waits to restart
  (`auto-restart`). 3 or more restarts in one report interval, or waiting to
  restart, is red ("crash-looping"); fewer is amber.
- **Filesystem errors** (REQ-PROBE-017): each mounted ext4 filesystem's
  `errors_count` from `/sys/fs/ext4/<dev>/` (no root needed). A count above
  zero is amber ("run fsck"); a count that rose since the last report is red.
  On an SD-card machine this is the early warning of a failing card.
- **Raspberry Pi power** (REQ-PROBE-018), no root needed: the hwmon device
  named `rpi_volt` (`in0_lcrit_alarm`), `vcgencmd get_throttled`, and the
  number of "Undervoltage detected" kernel messages since boot. Under-voltage
  or throttling now is red. Under-voltage since boot is amber, red at 10 or
  more messages, and stays on the host card until the next boot, so an event
  between two reports still shows. A weak supply can cut power and leave
  files that were just written empty. History keeps `power.undervolt`,
  `power.throttled` and `power.undervolt_events`.
- **Kernel drift** compares machines of the same OS, architecture and kernel
  flavour only, so a Debian amd64 kernel is not "behind" a Raspberry Pi one.

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

`POST /api/probe/enroll`, `/api/probe/report`, `/api/probe/renew`,
`/api/probe/feed/temper` and `/api/probe/feed/security` are the only write
endpoints usable without the
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

## warte ci-agent

The fleet agent on a self-hosted CI host: `warte ci-agent snapshot` collects
once and prints it, `warte ci-agent agent --hub <addr>` sends CCSDS telemetry
to every ground station each minute (and, with `--state-dir`, the job
ledger), `warte ci-agent arch` prints an `architecture.v1` document from a
spar instance and an inventory. Host paths come from `[ci_agent]` in the
config. See `warte docs ci-agent`.

## warte agent

On an agent's machine (REQ-GHTOKEN-004): `warte agent github-token` prints a
GitHub token for this agent (`--repo` narrows, `--json` prints the whole
answer); `warte agent git-credential get` is a git credential helper for
https://github.com. Both read `--url`/`WARTE_URL` and the agent's warte token
from `--token`/`WARTE_TOKEN` or `--token-file`/`WARTE_TOKEN_FILE`, and write no
file.

On the host, `warte agents repos list|show|set|clear|grant|revoke` sets which
repositories each agent's tokens reach (REQ-GHTOKEN-002). See
`warte docs github-tokens`.

## warte news

What changed in the tools a deployment uses, from the public feeds in
`[news]` (contract news v1, `warte docs news`):

```bash
warte news --since 2026-11-01 --grep runner --url https://host/warte
```

## warte nav

Check a nav v1 file (`warte docs nav`) before deploying it, with the same
rules and messages warte applies at startup:

```bash
warte nav check nav.json --self warte   # exit 1 naming the file and the field
```

Without `--self` the file is checked alone.

## warte sessions

The agent sessions a machine runs are listed in one JSON file, contract
sessions v1 (`warte docs sessions`). Check a file before deploying it:

```bash
warte sessions check sessions.json   # exit 1 naming the file and the field
```

Start and keep the listed sessions, contract lifecycle v1
(`warte docs lifecycle`). warte never kills a process:

```bash
warte sessions status --config sessions.json
warte sessions start  --config sessions.json --reason login [--id ID] [--dry-run]
warte sessions pause  --config sessions.json ID --reason "…"
warte sessions resume --config sessions.json ID --reason "…"
warte sessions rm     --config sessions.json ID --reason "…"   # refused while it runs
```

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
hosts = { laptop = "laptop.example", "build-box" = "build" }  # dir -> machine
```

(`WARTE_JOBS_DIR`, `WARTE_JOBS_EXPECTED` work too.) The Jobs view and
`GET /api/jobs` list every job worst first: **missing** (listed, no file),
**overdue** (no success within the window, so a stopped timer or a host
that is down shows up), **failing** (the last run failed, a success is
still within the window) and **ok**. Files that are not listed are shown
with their own `expect_within_s`. The airport puts each job on its
machine's pier and makes a late one the pier's note.
