# Next probe metrics: network, temperatures, processes, system health

Status: decided by the owner 2026-11-06 (option dialogue); built as REQ-PROBE-011. Builds on machine history (REQ-MHIST-001..003):
every new number below becomes a series and gets a chart with no extra work.

## Why

The owner found Beszel showed too little over time. History now exists; this fills it. The
smithy session's list of what incidents needed and Beszel lacked: memory and CPU pressure next
to IO pressure, swap and OOM kills, runner state, long history. Runner state and history are
done; pressure for CPU and memory exists only for the CI host today.

## Goals: what each probe adds (one report a minute, as now)

| Group | Series (names as stored) | Linux source | macOS source |
|---|---|---|---|
| Pressure | `psi.cpu.some`, `psi.mem.some`, `psi.mem.full` | `/proc/pressure/{cpu,memory}` avg60 | none (not drawn) |
| Memory events | `oom.kills` (per minute, from the counter) | `/proc/vmstat` `oom_kill` | none |
| Network | `net.rx_kbs{if=}`, `net.tx_kbs{if=}`, `net.errors{if=}` per minute | `/proc/net/dev` deltas (state file like IO) | `netstat -ibn` deltas |
| Temperature | `temp.c{sensor=}` (hottest per chip) | `/sys/class/hwmon/*/temp*_input` and `thermal_zone*` | none without root; left out |
| Battery | `battery.pct`, `battery.cycles` | `/sys/class/power_supply/BAT*` | `pmset -g batt`, `ioreg` cycle count |
| Processes | top 5 by CPU and by memory, **name only** (basename, ≤32 chars), not stored as series | `/proc/*/stat` + `statm` deltas | `ps -Ao comm=,%cpu=,rss=` (comm basename) |
| System health | `health.failed_units`, `health.reboot_required` (0/1), `health.updates` (count), `health.clock_offset_ms`, `health.ntp_synced` (0/1) | `systemctl --failed --output=json`, `/var/run/reboot-required`, apt's update-notifier stamp, `timedatectl show` / `chronyc tracking` | `softwareupdate` is too slow per minute: daily at most; `sntp -t 1` offset hourly |

Findings (amber unless noted): a failed systemd unit (named); reboot required for more than 3
days; security updates pending over 7 days; clock offset over 500 ms (red over 5 s, since
signed reports and certificates depend on it); an OOM kill in the last hour; a network
interface with errors climbing; a sensor over its own `crit` threshold (red) or `max` (amber).

The Machines card gains no new bars by default: network and temperatures appear as sparklines
in the full history; health items appear as findings and as chips (`2 failed units`,
`reboot required`).

## Non-goals

- SMART, wear, GPU, USB debug probes and boards, drivers: the hardware step after this one.
- Per-process history: process names are a current snapshot only, never stored over time.
- Anything needing root on macOS (powermetrics, SMC temperatures).

## Anti-goals

- Command lines, arguments, environment, full executable paths or users of processes. Names
  only, basename, length-capped; a name that looks like a path or carries `=` is dropped.
- A probe run longer than the minute: every source has a timeout (as `RUN_TIMEOUT` today) and
  the slow ones (updates) run at most daily from a state file.
- Interface names or sensor labels as free text in findings beyond a whitelist of characters.

## Owner's decisions (2026-11-06)

1. **Process list**: top 5 by CPU and by memory, names only, in the full history panel only
   (not on the card), as a snapshot of the latest report, never stored as history.
2. **macOS updates and temperatures**: both wanted; the owner chose "with a root helper".
   Built **without** one: on the main Mac (Apple Silicon, uid 501) temperatures read through
   IOKit as an ordinary user (sysinfo components: CPU die, SSD), and `softwareupdate --list`
   needs no root either. The owner's intent (both on macOS) holds with less privilege; a root
   helper stays available if a Mac turns out to need it.
3. **Terraform** (`terraform/main.tf`, REQ-DEPLOY-001): moved out of public warte; the smithy
   session takes it into private smithy if still wanted.

## Built differently from the draft

- macOS update checks run daily, Linux apt simulation every 6 h (no Apple call more than daily).
- Network errors are charted, not a finding (a single error per minute would flap).
- Clock offset and NTP state are Linux only (`timedatectl`, `chronyc`); macOS has no unprivileged
  equivalent worth a network call each minute.

## Left to the builder

State files for deltas (as `io-prev.json`), parsers with fixture tests per platform, the
whitelist character class, chart grouping (`Network`, `Temperature`, `Health`).
