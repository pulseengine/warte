# warte ci-agent: the fleet agent on a CI host

`warte ci-agent` runs on a self-hosted GitHub Actions host. It reads the
host's runners and load, scans the organisation's queue, keeps a ledger of
finished jobs, and sends all of it as CCSDS telemetry over UDP to one or
more ground stations (`warte hub`; the wire is described in
`warte docs ground-station`). It only reads: it changes nothing on the host
and issues only GET calls to GitHub.

It was imported from the owners' private fleet tool. Its flags, wire format,
ledger files and output are the same as before, except for the changes listed
at the end.

## Commands

```text
warte ci-agent snapshot [collect flags] [--format json|text|markdown]
warte ci-agent agent    [collect flags] --hub <addr> [--hub <addr> ...]
                        [--interval 60] [--once]
                        [--state-dir <dir>] [--ledger-interval 3600]
                        [--ledger-budget 600] [--backfill-days 30]
                        [--ledger-per-frame 150]
warte ci-agent arch     --instance <spar.json> --repo <inventory checkout>
                        [--commit <sha>] [--spar-version <v>] [--generated-at <ts>]
```

Collect flags (for `snapshot` and `agent`):

| Flag | Default | |
|---|---|---|
| `--org` | `pulseengine` | GitHub organisation |
| `--token-cmd` | `[ci_agent] token_cmd` | command printing a GitHub token |
| `--token-args` | `--read-only` | its arguments (split on whitespace) |
| `--runner-prefix` | the hostname | GitHub runner names are `<prefix>-<id>` |
| `--no-github` | off | local view only, no API calls |
| `--queue-interval` | `300` | seconds between queue scans |
| `--active-days` | `30` | only repos pushed within this many days |
| `--max-runs` | `120` | workflow runs whose jobs are listed per scan |

- `snapshot` collects once and prints it.
- `agent` collects every `--interval` seconds and sends each snapshot to
  every `--hub` (the same bytes to each). With `--state-dir` it also keeps the
  job ledger in `<state-dir>/ledger/` and sends it, paced per frame.
- `arch` prints an `architecture.v1` document (`warte docs architecture-view`)
  from a `spar instance --format json` of the model plus the agent sessions
  of each machine in `[ci_agent] arch_hosts`, read from the inventory under
  `--repo` (`ansible/group_vars/all.yml`, `ansible/host_vars/<host>/main.yml`,
  `ansible/roles/agent_sessions/files/hosts/<host>/agents.kdl`). With no
  `arch_hosts`, the document holds the model only and no inventory is read.

Errors print `warte ci-agent: <error>` and exit 1. The agent reads
`/proc`, the journal and every runner's cgroup, so it runs as root on Linux.

## Configuration: `[ci_agent]`

Read from the config `--config` / `WARTE_CONFIG` names, or `warte.toml` in
the working directory. Every key is optional; unknown keys are an error.

```toml
[ci_agent]
runners_dir       = "/var/lib/runners"
runner_name       = "runner{id}"
runner_unit       = "github-runner@{runner}.service"
df_mounts         = ["/", "/var/lib/runners"]
token_cmd         = "/usr/local/sbin/ci-gh-token"
timers            = ["ci-downtime.timer", "ci-runner-watchdog.timer",
                     "ci-bazel-sweep.timer", "ci-trace-rollout.timer"]
dispatcher_log    = "/var/log/ci-downtime.log"
watchdog_unit     = "ci-runner-watchdog.service"
bazel_sweep_unit  = "ci-bazel-sweep.service"
trace_rollout_log = "/var/log/ci-trace-rollout.log"
arch_hosts        = []
arch_model        = "architecture/security/security.aadl"
```

| Key | Default | Used for |
|---|---|---|
| `runners_dir` | `/var/lib/runners` | each runner's directory is `<runners_dir>/<runner>` (its `_diag/Worker_*.log` and `.rustup` mount) |
| `runner_name` | `runner{id}` | a runner's user and directory name; `{id}` is its number and must appear once |
| `runner_unit` | `github-runner@{runner}.service` | a runner's systemd unit; `{runner}` must appear once |
| `df_mounts` | `["/", "/var/lib/runners"]` | mount points passed to `df -P -B1` |
| `token_cmd` | `/usr/local/sbin/ci-gh-token` | token helper when `--token-cmd` is not given |
| `timers` | the four `ci-*.timer` units above | timers whose state is reported |
| `dispatcher_log` | `/var/log/ci-downtime.log` | last 8 lines reported |
| `watchdog_unit` | `ci-runner-watchdog.service` | journal lines with `[runner-watchdog]`, last 4 |
| `bazel_sweep_unit` | `ci-bazel-sweep.service` | journal lines with `swept` or `dead`, last 2 |
| `trace_rollout_log` | `/var/log/ci-trace-rollout.log` | last 6 lines reported |
| `arch_hosts` | `[]` | `arch`: machines whose agent sessions are read |
| `arch_model` | `architecture/security/security.aadl` | `arch`: written to `source.model` |

A unit or file that does not exist is reported as inactive or empty; it does
not fail the snapshot.

## Changes from the original tool

- The automation units and logs, the token helper path and the `arch`
  machines and model path were fixed in the code; they are now the keys
  above, with neutral defaults. A host that relied on the old values sets
  them in `[ci_agent]`.
- The markdown report's title is `CI fleet status`; GitHub requests send
  `User-Agent: warte-ci-agent`; `agent_version` in the telemetry is warte's
  version.
- `arch` with no `arch_hosts` reads no inventory files.
