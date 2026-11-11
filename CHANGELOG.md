# Changelog

Newest first. Releases up to v1.3.0 were merged while the rivet CI gate was red (#34); their "Cuttable" claims were checked only by a local rivet.

## Unreleased

## v1.6.15 — 2026-11-11

- **The v1.6.11 and v1.6.14 changes get their own requirements**, so they can be released. Both tags were refused by the release gate because no rivet artifact was scoped to them, and neither has a GitHub release. REQ-COSTS-003: a counter that was not measured shows as not measured, never as 0 (the v1.6.11 change). REQ-COSTS-004: a report whose newest data is more than a day old is stale (the v1.6.14 change). No code changes; v1.6.15 carries both.

## v1.6.14 — 2026-11-11

- **A fresh costs report with old data is stale** (REQ-COSTS-002). On 2026-11-11 the transcript archive had failed for three days. The nightly report was generated on time from stale input and showed as "fresh". warte now compares the newest day with data against the report's generation. More than a day apart makes the report stale, and the Costs view says the input may have stopped. `/api/costs` adds `data_through` and `data_lag_days`.

## v1.6.13 — 2026-11-11

- **Login through Zitadel** (DD-AUTH-001, REQ-AUTH-001..003). With an `[auth]` section, personal data needs a login. That means costs, skills, briefing, triage, agents, artifacts, Ask, friction and health. Fleet status stays open on the LAN, and so do the APIs agents and probes use.

  warte never sees a password. Either a proxy passes the provider's JWT, or **Sign in** hands the browser to Zitadel (code flow with PKCE, state and nonce) and warte keeps a server-side session behind an HttpOnly, Secure, SameSite=Lax cookie. Either way warte verifies the signature (JWKS), issuer, audience and expiry. Roles come from Zitadel project roles (viewer, member, owner). `alg: none`, shared-secret tokens and bare identity headers are refused.

  Every read route has a data class, and a route without one needs a login and fails a test. Without `[auth]` nothing changes.

  An independent security review before release found six issues, all fixed with a test each:
  - A login is bound to the browser that started it (login CSRF).
  - The return path is a local path from a short allow-list. A tab character could turn it into another site on a root deployment.
  - Only warte's project-scoped roles count.
  - A flood of login starts pushes out the oldest instead of locking everyone out.
  - The provider's keys are fetched at most once a minute, even while it is down.
  - The local test issuer is matched by exact host.

  Tokens with a `crit` header are refused, and `ca_bundle` trusts a LAN CA for the provider.

## v1.6.12 — 2026-11-11

- **Issue triage queue** (REQ-TRIAGE-001/002). Every 10 minutes warte reads the organisation's public event feed, without a token and with its ETag, and queues each issue a person opens or reopens. Bots, configured logins, pull requests and issues carrying an agent marker are skipped. `GET /api/triage` hands the oldest out, at most 10 an hour, and counts the rest. The reply run, posting as its own bot outside warte, marks each one done with `triage:write`. warte stays read-only on GitHub. Configure with `[triage] org = ...`.
- **The briefing's plain version claims counts only** (REQ-BRIEF-002). The first briefing on fourpi said the checks "held those changes back until they passed" and that "the agents" finished the work. Neither is in the facts: a failed job may be on main or never fixed, and merged pull requests include people's and bots'. It now names the counts and the repositories with the most failed jobs. The open pull requests are counted in full, not capped at the 20 listed. Template briefings already kept are written again from their facts when shown.

## v1.6.11 — 2026-11-11

- **Costs schema 2: `loop_guard_fired` is optional** (REQ-COSTS-002). Where no loop guard is installed, the extractor leaves it out, and the Costs view says "not measured" instead of a 0 that could never be anything else. Asked for by the owner through the extractor's maintainer.

## v1.6.10 — 2026-11-11

- **The briefing** (REQ-BRIEF-001..003). Every two days, a Briefing view says what the agents did:
  - one line anyone understands;
  - three plain bullets: done; what went wrong and how it was caught; what waits for you;
  - collapsed, the numbers for an IT reader: merged pull requests, closed issues, releases, CI runs and failed jobs per repository, pull requests open for more than a day, this week's notional cost against last week's, and the setup changes of the last two weeks.

  How it is made:
  - The facts are read from GitHub without a token, so only public repositories appear.
  - warte writes a plain version from the facts at once.
  - A scheduled model run elsewhere may post a friendlier text with `briefing:write`. It is kept only if every repository, number and version it names is in the facts; otherwise the plain version stays and the view says why.
  - Every briefing is kept with its facts. Configure with `[briefing] org = ...`.
- **Crash loops of template units are told apart** (REQ-PROBE-016). The probe kept restart counts per template, so twelve `github-runner@runnerN.service` instances collapsed onto `github-runner@.service`. The list showed it twice, and the deltas were wrong. Counts are now kept per instance on the machine. What is published is one entry per template, with the worst instance's restarts, and names the instances that restarted when their name is a plain word (`runner3`); an instance named after a path or a user id is never shown. The probes need this release for it.

## v1.6.9 — 2026-11-10

- **Costs schema 2** (REQ-COSTS-002). The nightly costs report may now carry `weeks`: per ISO week, the sessions; advisor calls, compactions, cross-session messages received and SendMessage calls, main sessions and subagents apart; and per repository and active skill, tool uses and tool errors by tool (MCP tools by server), permission denials, failure streaks (3 or more in a row) and loop-guard firings. Counts and names only: error text, a raw MCP tool name, a path or more errors than uses refuse the report. The Costs view shows the newest week against the one before, the failed calls by tool and the wasted effort by repository and skill. Schema 1 is still accepted. The schema is in `warte docs costs-schema`.
- **An idle scene** (REQ-DASH-018). After two minutes without a key, click or move, a translucent seasonal layer drifts over the dashboard, which stays readable underneath; any input takes it away. The theme changes every month (frost, carnival, spring rain, blossom, seeds, fireflies, the lake, shooting stars, migration, leaves, lanterns, Advent lights; six months on in the southern hemisphere), and calendar moments take over on their day: carnival, Easter, the solstices, the Perseids, Halloween, St. Martin, the Advent Sundays, New Year's fireworks. The caption has the sun (daylight left, or when it rises), the moon's phase drawn as it is, the weather and a fact from the ground station. warte stores no location: the browser's time zone stands in. Off with `?calm` or `[idle] enabled = false`; `[idle] weather = false` keeps the browser from calling Open-Meteo.
- **Machines view** (REQ-PROBE-003) verified: Beszel is retired.

## v1.6.8 — 2026-11-10

- **The monthly report works without a smithy agent** (REQ-RUNNERS-005). The GitHub source now keeps the job ledger: every `ledger_interval_s` (600) it reads the finished jobs of the scanned repos for the last `ledger_days` (2) days and today, records each once, and marks a day complete only when the day is over, every run created on it has finished and its jobs were read. Runner minutes, costs per pool, waits and flow efficiency in `/api/report` then come from GitHub's own records. Listings go out with their ETag, finished runs are read once and complete days are not listed again. The report matches GitHub runner names to their labels in this mode too.

## v1.6.7 — 2026-11-08

What went unnoticed on a host for days now reaches Outages and the outage alerts. On netcup, temper crash-looped for two days and a newer kernel waited 22 days for a reboot; warte showed neither.

- **Red machine findings and late feeds are outages** (REQ-TEMPER-003). The outage alerts only ever looked at the CI fleet and GitHub's status. Now Outages has a "machines and feeds" panel and the alerts notify red machine findings and a temper report that is missing or older than `[temper] stale_after_s`.
- **Crash loops are found** (REQ-PROBE-016). A `Restart=` loop never reaches `failed`, so failed units missed it. The probe reports services whose restart count rose since its last report or that wait to restart; 3 or more in one interval is red.
- **A pending reboot is found from the files themselves** (REQ-PROBE-015): Debian's reboot-required file with its package count, or a newer kernel in `/boot` than the one running. Amber at once, red after 7 days, its age taken from the files, so it no longer resets.
- **ext4 errors are reported** (REQ-PROBE-017). Each mounted ext4 filesystem's error count, read without root: amber above zero, red when it rose. On gale an SD card's write failure showed only as a failed unattended-upgrades unit.
- **Kernel drift compares like with like**: machines of the same OS, architecture and kernel flavour. netcup's amd64 kernel was reported "behind" the Raspberry Pis'.

The probes need this release for the reboot reason and the crash loops.

## v1.6.6 — 2026-11-08

- **GitHub runners are told apart** (REQ-RUNNERS-001..004). GitHub-hosted jobs no longer share one `hosted` pool: each runs-on label is its own pool (`hosted:ubuntu-latest`, `hosted:macos-15`, `hosted:linux-16core`), with a kind (standard hosted, larger hosted, self-hosted) and an OS. CI history charts queued and busy per pool; start estimates use each pool's own waits; the monthly report shows runner minutes per pool and, where `[runners]` gives a per-minute rate, a notional cost. **API change:** `pool` in `/api/eta`, `/api/report` and the queue now reads `hosted:<label>` for GitHub-hosted jobs (old records keep `hosted`).
- **A runner inventory in config** (REQ-RUNNERS-002). Where GitHub refuses to list the org's runners (GitHub Enterprise without admin rights), `[[runners.pool]]` gives each pool's labels, count, kind, OS and size; idle capacity then shows. Where GitHub lists runners, its list wins. Each runner says where it is known from (`config`, `inferred`).
- Not yet: without a smithy agent the monthly report has no job ledger (REQ-RUNNERS-005, next release); at home the smithy agent still reports GitHub-hosted finished jobs as one `hosted` pool until it sends the label.

## v1.6.5 — 2026-11-08

- **Disk pressure** (REQ-PRESSURE-001, REQ-PRESSURE-002). `GET /api/pressure?host=` answers whether a host is low on disk: a verdict per writable volume (`ok`, `low` under 20 GB free, `critical` under 10 GB, both set in `[pressure]`), the host's being the worst, and the GB that agent scratchpads, `target/` directories and git worktrees hold there, as totals only. Who holds it is shown only on the machine: `warte probe pressure` lists each holder with its path, session id, live sessions and size, largest first. The probe sizes holders only while a volume is low, at most every 10 minutes, within 60 seconds, on the low volumes only; it follows no symlink and deletes nothing.

## v1.6.4 — 2026-11-08

- **The retired `/ci` paths answer `410 Gone` with their replacement** (REQ-SMITHY-008). Since v1.6.3 they answered a bare `404`, and an agent calling `/ci/api/eta` could read the failed call as "nothing queued". Now each answers `410` with JSON naming the path under warte's own API (`/ci/api/eta?repo=spar&pr=1` → `/api/eta?repo=spar&pr=1`, `/ci/api/state` → `/api/queue`).

## v1.6.3 — 2026-11-08

`/ci` is retired: everything it offered is in warte's own API and dashboard since v1.6.2.

- **`/ci` is retired** (REQ-SMITHY-007). `warte serve` no longer mounts the absorbed smithy-hub page and API under `/ci`; it answers `404`. Everything it offered is under warte's own API and dashboard: `/ci/api/eta` → `/api/eta`, `/ci/status.md` → `/api/status.md`, `/ci/api/report` and `/ci/report.md` → `/api/report` and `/api/report.md`, `/ci/api/history` → `/api/history/fleet`, the page → the CI history and Health views. `/ci/api/state` has no successor (it carried raw log lines). The standalone `warte hub` still serves the old API on its own port.

## v1.6.2 — 2026-11-07

Everything the ground station's `/ci` page offered, in warte's own API and dashboard: the start estimate, status as Markdown, the monthly conveyor-belt report, fleet trends and the CI host's timers. `/ci` itself still answers in this release; it is retired in the next.

- **The `/ci` page's history and reports, in warte** (REQ-SMITHY-006). A new **CI history** view: fleet trends (runners busy and online, jobs queued on our runners and on GitHub's, the longest wait, PRs waiting, the CI host's load) over 6 h, 24 h or 7 d, and the monthly conveyor-belt report (work against waiting per pool, whether the runner pool is the bottleneck by hour, the busiest workflows, the runners) for this and the two previous months. **Health** shows the CI host's timers. Under warte's API: `/api/status.md`, `/api/report`, `/api/report.md` and `/api/history/fleet`, the same answers as their `/ci` counterparts. `/ci` stays until its readers have moved.
- **`/api/eta` under warte's own API** (REQ-SMITHY-005). The start estimate that was only at `/ci/api/eta`: per running job where and for how long, per queued part its pool, how long it waited and when it probably starts (`start_in_low_s`, `start_in_high_s`, `overdue`, `basis`). Same answer as `/ci/api/eta`, which stays for now; agents and the ci-status skill can move to `/api/eta` before the `/ci` view is retired.

## v1.6.1 — 2026-11-07

Agents as they are: "waiting for you" only when an agent asks you, done and idle otherwise; Copilot CLI and Mistral Vibe recognised; `warte probe install-hooks`; each probe's version and certificate end on its card. The probes need this release for the new hook states and Vibe; probes older than 1.5.0 cannot renew their certificate.

- **`warte probe install-hooks` offers the activity hook to every agent here** (DD-PROBE-003). Claude Code (`~/.claude/settings.json`), Copilot CLI (its own `~/.copilot/hooks/warte.json`) and Mistral Vibe (two entries appended to `~/.vibe/hooks.toml`). It shows the exact change, writes only after a yes (or `--yes`), backs each file up next to itself, keeps every other hook and setting (untouched keys in `settings.json` keep their exact text) and adds nothing where the hook is already there. `warte docs cli-reference` lists the events per agent.

- **"Waiting for you" means the agent is asking you** (REQ-PROBE-014). Every session that finished its turn showed as waiting, orange with a beacon, for as long as it sat there: on the main Mac most had been "waiting" for days. Now only a permission prompt, a question or a form counts as waiting. A finished turn is "done", and after 30 minutes "idle", parked without a beacon. The probe reports since when, and the airport shows "waiting for you · 5 min". Activity files from older hooks are read as done. A probe older than this sends "waiting for you" without a time, and the airport shows that as done too, so the map is right before the probes are updated.
- **The airport shows every session**. A machine draws at most 6 planes; its label now says "12 agents (+6 not drawn) · 1 asking you", and selecting it lists every session with its state and how long. The perimeter fence's caption no longer swallows clicks on the machine label under it.
- **Copilot CLI and Mistral Vibe sessions are recognised, once each**. Vibe (`vibe-rs` and its `vibe-app-server`) was not recognised at all; a Copilot CLI installed with npm counted twice (its loader and its native binary). The probe keeps the innermost process of such a pair. `warte probe hook` understands Copilot's and Vibe's hook events and tool names, and takes `--event` for hook configs that name the event themselves.
- **macOS: smartctl from Homebrew is found under launchd** (REQ-PROBE-012). A launchd agent's PATH has no `/opt/homebrew/sbin`, so the probe never found smartctl there; it now also looks in `/opt/homebrew/sbin` and `/usr/local/sbin`. Only on macOS: on Linux the root collector runs only the `smartctl` on its PATH.

- **Each machine shows its probe version and when its certificate ends** (REQ-PROBE-013). The card adds the version after "via probe", marked when it is older than the ground station's, and the date the certificate ends; `/api/machines` publishes both (`report.probe_version`, `cert_until`). Probes older than 1.5.0 cannot renew their certificate: a finding says so 14 days before it ends (red at 3 days) and names the version needed. A probe that can renew but still has under 7 days left is a finding too (red at 2 days), because renewal starts at 10. An older version alone is not a finding.

## v1.6.0 — 2026-11-06

Machine history and hardware. warte now keeps 90 days of history for every number a probe reports and charts it, and probes report health, network, temperatures, processes and hardware. Items in v1.6.0's rivet scope that were not built (40, among them the Devices view, the airport map and the AADL fleet model) moved to v1.7.0.

- **No "collector not running" where there is nothing to collect** (REQ-PROBE-012). A machine whose disks have no SMART (SD card, virtio, a USB-attached SSD) reports the privileged collector as `none`, whether or not a collector is installed. Before, the ground station's own card said "SMART: collector not running" because its SSD is on USB.

- **Probes report hardware: disk health, debug probes and boards, GPUs** (REQ-PROBE-012, #52). Disks get SMART status, wear, media errors, power-on hours and temperature; USB devices are listed with the driver holding them, and debug probes (J-Link, ST-LINK, CMSIS-DAP, Raspberry Pi Debug Probe, Black Magic, FTDI, LPC-Link2, XDS110, Espressif, EDBG, …) are recognised by id; GPUs with model and, with nvidia-smi, load, memory and temperature. Findings: SMART failing (red), wear at 80 % (amber) / 95 % (red), media errors, and a debug probe seen this week that is gone. Cards get a chip per attached probe kind; the history gets Disk health and Hardware groups and an "Attached now" list. No serial numbers are published. On Linux SMART needs root, and the probe gets none: `warte probe privileged`, a separate root oneshot with no network, writes SMART to a file that the probe reads as untrusted data (strict schema, size cap, names and values re-checked) and reports "privileged collector not running" when it is missing or stale (`warte docs probe-privileged` has both systemd units). On macOS the probe reads SMART itself, unprivileged.

- **An open machine history stays open** (REQ-MHIST-003). The 30-second refresh folded it again after a few seconds. The page now tells the server which histories are open, and it renders them open and filled in. The address holds the open machine and the range (`#machines/<host>?range=7d`), so a reload or a shared link opens the same history.

- **Probes report health, network, temperatures and processes** (REQ-PROBE-011, #52). Each report now carries CPU and memory pressure and OOM kills (Linux), per-interface traffic and errors (loopback and virtual interfaces left out), temperatures per sensor family (macOS too, without root: IOKit as an ordinary user), battery charge and cycles, the top 5 processes by CPU and by memory (names only, summed per name), failed systemd units, reboot required, pending (security) updates (apt every 6 h, macOS `softwareupdate` daily) and NTP state and clock offset (Linux). All of it except the process list becomes machine history, charted under Network, Temperature, Health and Power; the process list shows under "Processes now" in a machine's history; cards get chips for failed units, reboot, updates, battery and the hottest sensor. New findings: failed units, reboot required over 3 days, security updates pending over 7 days, clock off by over 500 ms (red over 5 s), NTP not synchronised, an OOM kill in the last hour, a sensor near or at its critical temperature. Names pass a strict whitelist or are dropped; the read API publishes the block without error text. On its first run on the main Mac it showed one process holding a full core and 33 `node` processes using about 7 cores, which is the load of 10.6 on 10 cores.

- **Terraform for one hosting setup moved out of the public repo**: `terraform/main.tf` and REQ-DEPLOY-001/TEST-DEPLOY-001 are gone from warte; deployment lives in the private infrastructure repository.

- **Machine history: numbers over time, not just now** (REQ-MHIST-001..003). Beszel kept a few series and too little of them. warte now keeps every machine's numbers itself, in its own file next to the machines state (`history.redb`, pure-Rust redb): per minute for 7 days and hourly min/avg/max for 90 days (`[history] keep_days`). Every probe report goes in (load and load per core, memory, swap, disk used per mount, disk IO per device and IO pressure on Linux, agent sessions, uptime), and so does every ground-station snapshot (CPU and memory pressure, runners online/busy/offline, per-runner CPU and memory, queue depth and longest wait), the ground station's existing 7 days imported once. Only names and numbers are stored. Writes are buffered for the SD card (durable every 5 minutes and at shutdown) and a store error never touches a report: it is one amber finding. The Machines view gets trend lines on each card with a 1 h · 24 h · 7 d · 90 d switch that the browser remembers, and a collapsible full history per machine grouped by CPU, memory, disks, IO, runners, queue and agents, with hover values; gaps break the line. Disk, load, memory and IO findings link to their chart. `GET /api/history/machines[/<host>/metrics|/<host>/series]` serves the series. Works the same without smithy, so GitHub-only setups get charts too.

- **Probe inventory: what agents actually run** (REQ-PROBE-010, #52). The session-start hook checks, in the agent's own PATH, every tool varve has a shim for, and records the ones another binary shadows; the probe adds their versions. The probe also reports installed Claude Code plugins and the machine's scheduled jobs with their last result (macOS LaunchAgents, systemd timers, crontab schedule and program name, never arguments). Machines shows them folded on each card and raises drift findings: tools run unpinned, a scheduled job whose last run failed, a plugin behind the newest version installed anywhere. On the first run on the main Mac it found varve's shims missing from agents' PATH (8 tools unpinned, rivet 0.38.0 among them) and a monthly job killed with signal 9. The read API and the views show where a binary comes from ("cargo install", "Homebrew") instead of its path, and only a count of what could not be read.

- **Outages and alerts without smithy** (REQ-ALERT-002). In GitHub-only mode the findings are worded for GitHub ("No GitHub scan for …", "GitHub scan: …") instead of a smithy agent and a telemetry link, and a rate limit is its own finding instead of only stale data. A secondary rate limit (`retry-after`) is now remembered, so no request goes out before it lifts. Documented under "Without a smithy agent".

- **Skills view: which agent skills are actually used** (REQ-SKILLS-001, #125). Same design as Costs: a workstation job counts skill invocations in its local agent transcripts and pushes one `skill-usage.v1` report, whole, by `POST /api/skill-usage` (new right `skills:write`: `warte token mint <agent> --right skills:write`) or as a file (`[hub] skill_usage_file`, re-read when it changes). warte keeps the latest report per `source` and shows, per skill, invocations (model / slash / subagent), sessions and SKILL.md reads, summed and per source; the installed skills never invoked; the share of sessions using any skill; and each report's window and age. `GET /api/skill-usage` serves the reports. Unknown fields, free text (a strict character class on every string) and negative counts are rejected with a 400, nothing is stored, and the reason never quotes the input.

- **Outage alerts no longer flood** (REQ-ALERT-001). In their first night live they recorded 100 notices in 4.5 hours, 82 of them packet loss: when the link lost ~1.5%, each of 46 telemetry streams opened its own finding in the same minute and cleared one by one, and CPU pressure hovering at 10% opened and cleared six times. Packet loss is now one finding for the link ("loses packets on 46 streams, worst …"); amber findings wait five evaluations to open and fifteen to clear (red keeps two and three); and one evaluation records at most five notices, the rest folded into one summary.

- **GitHub's status in Outages, and hosted jobs stuck in the queue detected** (REQ-STATUS-001, REQ-DASH-008/009, #120). On 2026-11-04 GitHub had two Actions incidents and up to 61 GitHub-hosted jobs queued against ~17 running, while Outages showed only a disk warning and "GitHub's own status is not monitored yet": the status client was built but never called, although its requirements were accepted. Outages now reads githubstatus.com's `summary.json` (one request, 5 min cache, 5 s timeout, a failure retried after a minute, no GitHub API calls): overall status, components not operational, unresolved incidents, https links only. GitHub-hosted jobs queued 15 min or more are a *degraded* finding per runner type. `status_url = "off"` now really turns it off.

- **Outage alerts reach the owner** (REQ-ALERT-001). Every minute the server evaluates the findings Outages shows (the fleet's and GitHub's) and records a factory notice (`notice.v1`, schema version 2) under its agent `warte-alerts` when one opens, escalates or clears, so `notice.py digest` / `review` show it under the owner's attention settings. A finding opens after two consecutive evaluations and clears after three without it; while open it does not repeat unless it gets worse.

- **The Overview shows Outages at a glance** (REQ-DASH-017): the number of findings, the worst one, and GitHub's status, in a strip that opens Outages.

- **Hosted queue history per runner type** (REQ-HIST-001). Each history point carries GitHub-hosted queued and running jobs per `runs-on` label (the 12 busiest), so a backlog can be attributed to macOS or Linux afterwards. Older points without it still load.

- Removed the unmounted placeholder `src/server/auth.rs` (#60); the auth middleware in use is `crate::auth`.

- **Agent bearer tokens can be minted** (REQ-API-003). The server signed and verified tokens but nothing issued one, so every agent write got 401. `warte token mint <agent-id> [--valid-hours N]` prints a token on the ground station host. The signing key was a fresh random key per start; it is now a seed file (`token-seed`, mode 0600, created on first start), so tokens survive restarts. Deleting that file invalidates every token.

- **Tokens carry rights, and write only as their own agent** (REQ-API-004, #117). Each write route now needs its right in the token (`agents:register`, `agents:heartbeat`, `agents:data`, `access:grant`, `access:revoke`, `sync:update`, `costs:write`); without it the answer is 403, and a missing or invalid token stays 401. On `agents/register` the `name`, and on `agents/{id}/...` the name of the agents row `{id}`, must equal the token's agent id unless the token holds `agents:any`. `POST /agents/{id}/data` for an unknown agent is 404, not 500. A write route with no row in the table is refused. `warte token mint` keeps the three agent rights by default; `--any-agent` adds `agents:any`, `--right <r>` (repeatable) adds a named right, and an unknown right is an error. **Tokens minted before this change keep working for agent writes only as their own agent, and no longer reach `access/*`, `sync/update` or `costs`: re-mint with `--right costs:write` for the cost job, and with `--any-agent` for a token that posts as many agents.** The contract lists each route's right.

## v1.5.1 — 2026-11-04

- **temper's compliance report, via its host's probe** (REQ-TEMPER-002, DD-TEMPER-002). temper drops its HTML dashboard; its results reach warte instead. temper writes `temper-report.v1` (enums, identifiers, times and hex only) on its own host, and that host's probe sends it with `warte probe send --feed temper=<path>`: one CCSDS segmented sequence on APID 0x400, the relay-ccsds codec of the CI agent's telemetry, signed with the probe key. warte accepts it only from the `[temper] host` probe, fresh, not replayed, from an unrevoked certificate, and valid against the schema (rejected whole otherwise, the input never quoted). The new **Compliance** view and `GET /api/compliance` show failed checks worst first, temper's failed or stuck tasks, its reviews and its own GitHub API use, and say when the last full sweep is older than 26 h. warte makes no GitHub API calls for it.
- **Fixed:** the auth middleware kept its own list of the open probe routes instead of the documented one; it now uses that list, and a test posts to each route without a token.
- **Release assets the pulseengine varve layer can carry.**
  - The signed checksums are now `SHA256SUMS.txt` and `SHA256SUMS.txt.cosign.bundle`, the PulseEngine release standard. varve's layer deposit reads only those names, so under the old ones the pulseengine realm could not carry warte. **Anyone who fetches `SHA256SUMS` or `SHA256SUMS.cosign.bundle` by name must switch to the new names.**
  - `sbom.json` is in the checksums, so the cosign signature now covers the SBOM.
  - Each binary has SLSA build provenance: `gh attestation verify warte-<version>-<platform> --repo pulseengine/warte`.

## v1.5.0 — 2026-11-04

Probes on every machine: they enroll, send signed reports, renew their own certificates and report disk IO. warte also runs on GitHub Enterprise without runner-admin rights and without burning the token's rate limit. Not started items moved to v1.6.0 (owner's decision 2026-11-04): QUIC/relay-sec transport, sigil signing, the varve layer, live push, Devices, Topology, mDNS, advisories.

- **Probes enroll, send and renew.**
  - **Enrollment** (DD-PROBE-002): `warte probe token --host <h>` on the ground station mints a one-time token (15 min, single use, bound to the host, stored as its SHA-256). `warte probe enroll <token> --ground <url>` on the machine makes its key locally (0600) and sends only a certificate request; warte's CA signs a 30-day certificate for the token's host.
  - **Signed reports** (REQ-PROBE-006): `warte probe send` signs its report with that key and posts it to `POST /api/probe/report`. It is stored only if the certificate is warte's and valid, names the report's host, the signature verifies, and the report is fresh and newer than the last one (no replays). A signature, not mutual TLS, so it works behind a reverse proxy.
  - **Renewal** (REQ-PROBE-007): at two thirds of its life, `probe send` renews first: a CSR for the same key, signed with it, to `POST /api/probe/renew`, no token. An expired certificate needs a new token. `warte probe revoke --host <h>` refuses every certificate the host holds now (migration 007).
  - The release build fails unless `warte probe --help` lists `enroll`, `send` and `hook`: v1.4.0 had only `report` and broke a probe host it was installed on.
  - Releases ship `linux-arm64` too, for probes on Raspberry Pis (REQ-RELEASE-006).
- **Probe IO** (REQ-PROBE-008): on Linux, reports carry each block device behind a reported disk, whether it spins, its busy share, time per request and rates since the previous report, and IO pressure (PSI). Findings: a spinning disk at 60 % busy or 100 ms per request is amber, 90 % red; IO pressure at 20 % amber, 50 % red. Optional fields: old probes and ground stations keep working. `/api/machines` publishes `io`; a test fails when a report field is neither published nor listed as hidden on purpose.
- **Walking agents that say what they do** (REQ-PROBE-009): `warte probe hook`, run by Claude Code's hooks, records only a verb ("editing", "running tests", "waiting for you") per agent; the airport shows agents walking with their repo and verb.
- **Airport map** (REQ-DASH-015, part 1): the fleet as an airport, from live data: the ground station as the tower, machines as piers, boards as stands, agent sessions as planes, busy runners as planes on runways with their PR, job and time, the queue as a holding pattern. Full screen with `f`. Every string is drawn as text, never markup.
- **Jobs view** (REQ-JOBS-001): scheduled jobs across hosts from the state files `pe-job` writes, against a manifest: ok, failing, overdue (no success within its window) or missing. A stopped timer or a dead host shows as overdue, not as silence.
- **GitHub as the CI source, at enterprise scale.**
  - **Rate limits** (REQ-GHSRC-003): every listing is asked again with its ETag; an unchanged one is a 304, which GitHub does not count. warte sends nothing while `rate_reserve` (150) or fewer requests are left, so other users of the token keep headroom. The default interval is now 120 s.
  - **Without runner-admin rights** (REQ-GHSRC-004): when the org runner list is unreadable (403, or 404 for ordinary members), busy self-hosted runners are taken from their running jobs, so Queue, Ask, `/api/eta` and admission count running self-hosted work. The Overview shows "self-hosted jobs running" instead of runways 0/0.
  - **Runner groups without a `self-hosted` label** (REQ-GHSRC-005): `self_hosted_labels` (`WARTE_SELF_HOSTED_LABELS`), `*` wildcards, case-insensitive.
- **Critical Path draws the chain** (REQ-DASH-006, #44): `repo_dependencies` in warte.toml declares which repo depends on which. The view shows the longest chain with a repo queued or running, its bottleneck, and a lower bound on release time from the measured pool waits. The path search now walks every simple path and needs a blocking repo, as the requirement says. rivet externals and GitHub's dependency graph as sources follow (REQ-CRIT-001, v1.6.0).
- **Ask refreshes itself** (REQ-DASH-016): its answer and in-flight list re-ask every 30 s while visible; the form stays as typed.
- **smithy-hub retired** (REQ-MIGRATE-001, REQ-SMITHY-004): warte's ground station replaced it in place, reading its data directory as-is, after a parallel run in which one smithy agent fed both and a full UTC day of ledgers matched (6765 records, 0 differences).
- **A config file that does not parse stops warte from starting** (REQ-DEPLOY-002). Before, warte fell back to its defaults and ran for minutes without its hub, fleet model and jobs while looking healthy.
- **Docs:** enterprise deployment notes (REQ-DOCS-010): GHE data residency behind a TLS-intercepting proxy, a static musl build, HTTP basic auth in front with the three probe routes left open (the documented Caddy matcher is checked against the server's exempt paths).
- **CI:** all GitHub Actions pinned to commit SHAs.

## v1.4.0 — 2026-10-29

One release for everything since v1.3.2: the ground station absorbed into warte, warte without a smithy agent (GitHub API, including `.ghe.com`), machine supervision, the agent APIs, and the control-tower dashboard. Machine supervision covers the ground station's own machine and the CI host; other machines join once probes can enroll and send (v1.5.0). Retiring smithy-hub follows its parity run (moved to v1.5.0).


- **Fleet model** (REQ-FLEET-001). The fleet a deployment intends (machines, where they live, which bench board is attached to which machine) is an AADL model of the new `Warte_Fleet` types (`architecture/fleet/`), instantiated with spar. `warte fleet show` reads the instance, and exits 1 when the model has problems. With `fleet_model` set, warte compares the model with what reports in: `GET /api/fleet`, plus a "model vs reality" panel in Machines that draws modelled-but-silent machines dashed. spar models; warte draws.
- **Costs:** a `costs_file` that exists but cannot be read (a denied directory, a unit sandbox hiding the path) is now shown as rejected, with the error kind (`cannot read the report: permission denied`), instead of looking like "no report received yet" (#80). Only a missing file stays the quiet empty state.
- **GitHub's API as a CI source** (#81, REQ-GHSRC-001/002). warte now works without a smithy agent: set `WARTE_GITHUB_API_URL` (default `https://api.github.com`; `https://api.<tenant>.ghe.com` for Enterprise Cloud with data residency; `…/api/v3` for GHES), `WARTE_GITHUB_ORG` and `WARTE_GITHUB_TOKEN`. The scan fills the ground station's model, so every view and the agent APIs work unchanged. It follows paging, waits out rate limits, and shows a refused token in Health and Outages. `WARTE_CA_BUNDLE` trusts a TLS-intercepting proxy's CA. The status-page URL is configurable or `off`. The `WARTE_GITHUB_ORG`/`WARTE_GITHUB_TOKEN` settings, parsed but unused since v1.3, now drive this source.
- **Costs view** and `GET /api/costs` (REQ-COSTS-001). They show the notional cost of agent chats and subagents, labelled **notional list price, not billed**: list prices applied to the token counts in agent transcripts.
  - **What it shows:** $ per day for the last 30 days, the top sessions, repositories and models, and subagent spend by task category. It also shows the share of subagent work that ran on a cheaper tier than its caller.
  - **How the report arrives:** a workstation job copies it to the ground station (`[hub] costs_file`, re-read when it changes). `POST /api/costs` also takes it, behind bearer-token auth.
  - **What gets rejected:** reports with unknown fields, free text in a name, or bad amounts. Nothing from them is stored, and the reason shown never quotes the input. The read API is public (#46), so a task description must not reach it.
- The dashboard view lives in the URL (`#machines`, `#queue`, ...). A reload stays on the view, a link opens it, and back/forward move between views. Before, a reload always went back to the Overview.
- **Security:** removed the permissive CORS layer. It let any web page a LAN user opened read warte's whole unauthenticated read API cross-origin. Nothing needed it: the dashboard is same-origin, and agents use the CLI. `/api/machines` publishes a reduced record, without tool paths, tool error output, process ids or working directories.
- **Machines view** and `GET /api/machines` (REQ-PROBE-001/003). The ground station probes its own machine every minute (`[hub] probe_self`, on by default) and shows a card per machine: OS, kernel, load, memory, disks, agent CLIs with versions, and agent sessions. The CI host gets a card from the smithy agent's data. Findings across machines, worst first: kernel or tool versions behind the newest seen, disks at 85% or more, low memory, load above the core count, and machines silent for 3 minutes. More machines appear as they are enrolled.
- **`warte probe report`** (REQ-PROBE-001, first part) prints what a machine is and runs: OS, kernel and architecture, CPU and load, memory and swap, disks, the agent CLIs and tools on PATH with their versions (`claude`, `copilot`, `codex`, `gh`, `varve`), and the agent sessions running now. It is read-only and sends nothing yet. For agent processes it reports the agent, pid, start time and working-directory name, never command lines. Sending (QUIC + mTLS + relay-sec), one-time-token enrollment (DD-PROBE-002) and hook-based agent state (DD-PROBE-003) build on it.

- **Every view now reads the ground station, not just the Overview.**
  - **Ask** lists everything queued or running right now; click a row to ask, no typing. The repo field suggests the repos in flight.
  - **Queue** shows each pool with a meter and its measured waits, plus every queued run on the departures board.
  - **Critical Path** shows which repos hold the queue, and each pool's p50/p90/max wait on one scale.
  - **Health** shows the CI host's load, memory, swap, CPU/memory/IO pressure and disks, the telemetry link's loss per stream, and collection errors.
  - **Outages** lists the problems detected now, worst first: stale telemetry, offline runners, OOM kills, full disks, pressure, overdue runs, link loss. It replaces a hardcoded "GitHub: operational" badge that claimed something warte never checked.
  - **Agents and Artifacts** get titles and empty states that say how to fill them.
- **Removed** the unauthenticated `/api/ws` and `/api/events`. They broadcast agent heartbeats and access-right grants and revocations to anyone who could reach warte, and nothing used them: the dashboard refreshes every 30 s, and agents use `/api/watch`. REQ-REALTIME-001 moves to v1.5.0 and returns with authentication.
- `warte watch --ca-cert <pem>` (or `WARTE_CA_CERT`) trusts a reverse proxy's local CA, e.g. Caddy's `tls internal`. Without it, watching `https://<host>/warte` failed with `UnknownIssuer`. A file holding no certificate is an error up front.
- **`warte watch --ci`** (REQ-WATCH-001/002, DD-WATCH-001) blocks until a PR's or branch's CI runs start or finish, then prints JSON and exits: 0 on a change, 3 on timeout, 2 on usage errors, 1 on errors. Agents run it as a background command instead of polling. The server side is the long-poll `GET /api/watch`. Its cursor is the CI state itself, so a caller resuming with `--since` cannot miss a change and the server keeps no event log. Holds are capped at 900 s, 8 per client and 1024 in total.
- **The Overview is a control tower** (REQ-DASH-014). It shows a runway gauge with one cell per runner, KPIs with two-hour trends and six hours of traffic, all drawn from the ground station's own history. Each runner is a flight strip showing what it is running, for how long, its cgroup CPU and memory, and whether GitHub's busy flag disagrees with the host. A departures board lists queued runs, longest waiting first. It estimates start from the pool's measured median wait and marks runs past p90 as overdue. Every view gets warte's own look: night sky, tower cyan for what is moving, amber for what is waiting, and a UTC tower clock. No new requests and no external assets: the page is still offline-capable, and motion stops under `prefers-reduced-motion`.
- `GET /api/queue` and `GET /api/admission` give agents a machine-readable queue and an `open | batch | hold` verdict before they open a PR (#55). The verdicts come from the ground station's snapshot, the thresholds live in warte, and stale data never gates. Documented in the new `/api/openapi.json`.

- **Ground station** (absorbed smithy hub, DD-SMITHY-001): `warte hub` takes smithy hub's flags and reads its data directory as-is; it serves the hub API unchanged. Behaviour on the ground-station host's real data was checked against smithy's own binary: 0 differences in 26 requests (`scripts/hub-parity.sh`).
- `warte serve` runs the ground station in-process when warte.toml has a `[hub]` section, and mounts its API under `/ci`. The Queue, Critical Path, Ask and Health views now read the hub's live data, escape repo/branch/job names, and show the snapshot's real age.
- **Removed duplicates of the hub:**
  - the poller (never started), the GitHub and GitLab webhooks, and the `eta` module;
  - the `ci_jobs`, `wait_history` and `ci_status` tables (migration 005);
  - `warte migrate smithy`, which imported a format smithy never had;
  - the stub `/api/dashboard/ask`.
  REQ-POLLER-*, REQ-CI-DATA-001, REQ-WEBHOOK-001/002 and REQ-ASK-* are deprecated in rivet and traced to DD-SMITHY-001.
- **Fixed:**
  - Routing: since at least v1.0.0 every API route was mounted at `/api/api/...`, so `/api/health` returned 404 and the dashboard page was unreachable. The API is now at `/api` and the dashboard at `/`.
  - The dashboard Ask form posted to a route that requires auth (401). Ask is now a read-only GET.
  - Views no longer show a hardcoded "fresh" badge, and the footer shows the real version.
  - Every dashboard view now HTML-escapes agent- and author-controlled text: agent names and repos, artifact fields, hardware boards, friction and schedule entries, and Ask input. Before, an agent could put script into the page for every dashboard viewer.
  - The leftover authentication exemption for `/api/webhooks/*` is removed together with the webhooks.
  - The dashboard uses relative URLs, so it works behind a path prefix (e.g. `https://<host>/warte/`). Unknown paths return a text 404; before, browsers saved the empty 404 response as `document.txt`.
- **Dashboard in a browser** (found on the ground-station host; HTTP-level tests had passed):
  - Overview was always empty: its response was an out-of-band swap into an element that didn't exist.
  - Ask fetched the whole page into the content area. Ask is now a view with its own form.
  - The footer claimed an SSE connection that never existed (no extension loaded, nothing publishing). It is replaced by a 30-second refresh of the current view until `warte watch` brings live updates.
  - htmx is vendored and served by warte (2.0.4, Zero-Clause BSD) instead of loaded from unpkg.com.
  - Overview now shows the fleet (runners, queue, freshness).
  - New Playwright suite (`tests/playwright`, modelled on rivet's) with a CI job; on the old dashboard 8 of its 18 tests fail.
- Deployment is environment-specific and no longer lives in this repository: it moved to the operators' private Ansible (a role for warte: hash-checked binary, systemd unit, Caddy route for private-network clients including the LAN's IPv6 prefix, landing-page link). This public repository keeps only generic documentation.
- **Traceability corrected:** REQ-CRIT-001 and REQ-DASH-006 had been `accepted`, but the Critical Path view never showed the dependency chain they describe (`CriticalPathComputer` is not called by any view). Both are back to `approved` for v1.5.0 (#44).

## v1.3.2

- Release pipeline: the SBOM job installed `cargo-cyclonedx` unpinned and called a flag (`--output-file`) the current release doesn't have, so v1.3.1's release run failed and no GitHub release was created. The tool is now pinned (0.5.9) and called with `--override-filename sbom` (CycloneDX 1.5). A new oracle requires every `cargo install` in the workflows to pin `--version` or `--rev`.
- The v1.3.1 release run otherwise confirmed the pipeline end to end: the rivet gate passed for the tag, both builds passed the `warte --version` check, and checksums and cosign signing succeeded.

## v1.3.1 — tagged 2026-10-28, not released

Tagged at `b5ce41c`; the release run failed at SBOM generation (fixed in v1.3.2), so there are no v1.3.1 binaries.

- CI installs rivet from `pulseengine/rivet` git at a pinned rev (v0.41.1, matching the varve layer). It used to install crates.io `rivet-cli`, which is a different project (#34).
- The release gate runs `rivet release status` for the tag being cut and fails on rivet's exit code (#34).
- CI jobs (build, test, clippy, rivet) run on the smithy `rust-cpu` pool. Pull requests from forks are routed to GitHub-hosted runners. That is routing, not isolation: a fork PR can edit the workflow, so the smithy runner-group policy and fork-PR approval are the actual boundary. All release jobs run on GitHub-hosted runners and restore no caches (#35).
- rivet is installed into a job-scoped `$RUNNER_TEMP` directory and never reused from `$HOME`; third-party actions are pinned to commit SHAs; workflows default to `bash`.
- Release assets: binaries, `SHA256SUMS` and the cosign bundle are now laid out flat, so signing and the GitHub release find them. Before this, every Release workflow run (v1.0.0–v1.3.0) failed, and the published releases have no assets. A missing asset now fails the release (`fail_on_unmatched_files`).
- License changed from MIT to **Apache-2.0**, the PulseEngine default; `LICENSE` added (there was none before). This also lets smithy hub's code move into warte unchanged (v1.4.0).
- `tests/ci_workflows.rs` checks all of the above from the parsed workflow YAML; each check was confirmed to fail on an injected violation.
- Release notes moved from 13 root `RELEASE_NOTES_*` files into this changelog; root `SHA256SUMS*` files removed (they ship as release assets). `Cargo.toml` version now follows the release, and release CI checks that `warte --version` reports the tag (#39).

## v1.3.0

2 artifact(s) in scope: 2 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-CRIT-003` (accepted) — GitHub dependency graph API integration — auto-discover dependencies from GitHub

#### verification

- `TEST-CRIT-003` (accepted) — test_spdx_sbom_deserialization + test_integration_creation + test_integration_ghe — GitHub dependency graph API

### Limitations in relation to the committed scope

None — every artifact committed to v1.3.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 2 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-CRIT-003`

1 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-CRIT-003` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-CRIT-003` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 2 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v1.2.0

2 artifact(s) in scope: 2 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-CRIT-002` (accepted) — Rivet externals integration — auto-discover cross-repo dependencies for critical path

#### verification

- `TEST-CRIT-002` (accepted) — test_rivet_externals_deserialization + test_integration_creation + test_build_computer_with_manual_deps — rivet externals integration

### Limitations in relation to the committed scope

None — every artifact committed to v1.2.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 2 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-CRIT-002`

1 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-CRIT-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-CRIT-002` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 2 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v1.1.0

4 artifact(s) in scope: 4 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-CRIT-001` (accepted) — Critical path dependency graph — real repo dependencies from rivet/GitHub
- `REQ-DASH-012` (accepted) — FREE board listing in hardware availability — show all boards, not just claimed

#### verification

- `TEST-CRIT-001` (accepted) — test_critical_path_simple_chain + test_critical_path_with_blocking + test_critical_path_no_dependencies + test_repo_node_is_blocking — dependency graph critical path
- `TEST-DASH-009` (accepted) — FREE board listing in hardware availability — all boards from catalog + claims, FREE/CLAIMED status

### Limitations in relation to the committed scope

None — every artifact committed to v1.1.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

2 of 4 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-CRIT-001`
- `TEST-DASH-009`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-CRIT-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-012` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-CRIT-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DASH-009` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 4 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v1.0.0

4 artifact(s) in scope: 4 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-DASH-011` (accepted) — Hardware availability in queue view — boards free/claimed, by whom, since when
- `REQ-POLLER-002` (accepted) — Poller measured wait recording — record queued → started transitions from polling

#### verification

- `TEST-DASH-008` (accepted) — Hardware availability in queue view — boards CLAIMED/FREE, agent, duration
- `TEST-POLLER-002` (accepted) — Poller measured wait recording — queued → started transitions tracked, waits recorded in wait_history

### Limitations in relation to the committed scope

None — every artifact committed to v1.0.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

2 of 4 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DASH-008`
- `TEST-POLLER-002`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-DASH-011` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-POLLER-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-DASH-008` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-POLLER-002` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 4 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.9.0

3 artifact(s) in scope: 3 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-DASH-009` (accepted) — Wire StatusClient into outages view — real GitHub status in dashboard
- `REQ-DASH-010` (accepted) — Dashboard navigation — add buttons for queue, critical path, resources, outages, friction, schedule, health views

#### verification

- `TEST-DASH-007` (accepted) — StatusClient wired into outages view + nav buttons for all 11 dashboard views

### Limitations in relation to the committed scope

None — every artifact committed to v0.9.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 3 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DASH-007`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-DASH-009` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-010` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-DASH-007` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 3 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.8.0

6 artifact(s) in scope: 6 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-DEPLOY-001` (accepted) — Edge proxy deployment automation — Terraform for Hetzner/Netcup
- `REQ-STATUS-001` (accepted) — GitHub status API integration — real status from status.github.com
- `REQ-WEBHOOK-002` (accepted) — GitLab webhook endpoint — receive pipeline events

#### verification

- `TEST-DEPLOY-001` (accepted) — Terraform configuration — Hetzner edge + Netcup secondary + WireGuard mesh + DNS
- `TEST-STATUS-001` (accepted) — test_status_client_creation + test_status_client_custom + test_status_serialization + test_incident_serialization — GitHub status API
- `TEST-WEBHOOK-002` (accepted) — GitLab webhook endpoint — X-Gitlab-Token verification + pipeline event processing

### Limitations in relation to the committed scope

None — every artifact committed to v0.8.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

3 of 6 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DEPLOY-001`
- `TEST-STATUS-001`
- `TEST-WEBHOOK-002`

3 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-DEPLOY-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-STATUS-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-WEBHOOK-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-DEPLOY-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-STATUS-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-WEBHOOK-002` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 6 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.7.0

3 artifact(s) in scope: 3 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-DASH-007` (accepted) — Dashboard resources view — who needs what (RAM, disk, runners, boards)
- `REQ-DASH-008` (accepted) — Dashboard outages + friction views — GitHub status, fleet health, tool errors

#### verification

- `TEST-DASH-006` (accepted) — Dashboard resources + outages + friction + schedule + health views — complete dashboard

### Limitations in relation to the committed scope

None — every artifact committed to v0.7.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 3 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DASH-006`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-DASH-007` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-008` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-DASH-006` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 3 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.6.0

3 artifact(s) in scope: 3 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-DASH-005` (accepted) — Dashboard queue view — runner pool, waiting jobs, hardware availability
- `REQ-DASH-006` (accepted) — Dashboard critical path view — longest chain of dependent gates

#### verification

- `TEST-DASH-005` (accepted) — Dashboard queue + critical path views — runner pools, waiting jobs, bottlenecks, ETA pool stats

### Limitations in relation to the committed scope

None — every artifact committed to v0.6.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

1 of 3 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DASH-005`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-DASH-005` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-006` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-DASH-005` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 3 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.5.0

4 artifact(s) in scope: 4 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-POLLER-001` (accepted) — Periodic CI status poller — fallback when webhooks are not configured
- `REQ-WEBHOOK-001` (accepted) — GitHub webhook HTTP endpoint — receive workflow_run events

#### verification

- `TEST-POLLER-001` (accepted) — test_poller_config_default + test_poller_config_custom + test_poller_creation — periodic CI status poller
- `TEST-WEBHOOK-001` (accepted) — GitHub webhook endpoint — HMAC-SHA256 signature verification + workflow_run event processing

### Limitations in relation to the committed scope

None — every artifact committed to v0.5.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

2 of 4 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-POLLER-001`
- `TEST-WEBHOOK-001`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-POLLER-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-WEBHOOK-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-POLLER-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-WEBHOOK-001` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 4 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.4.0

4 artifact(s) in scope: 4 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-CI-DATA-001` (accepted) — GitHub CI data collection — webhook receiver + status poller for ETA estimation
- `REQ-DASH-004` (accepted) — Dashboard HTML frontend — HTMX-based overview, ask, queue views

#### verification

- `TEST-CI-DATA-001` (accepted) — test_parse_datetime + test_infer_runner_pool + test_webhook_deserialization + test_process_webhook — GitHub CI data collection
- `TEST-DASH-004` (accepted) — Dashboard HTML frontend — HTMX-based overview, ask, agents, artifacts views with SSE

### Limitations in relation to the committed scope

None — every artifact committed to v0.4.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

2 of 4 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-CI-DATA-001`
- `TEST-DASH-004`

2 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-CI-DATA-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-004` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-CI-DATA-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DASH-004` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 4 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.3.0

10 artifact(s) in scope: 10 delivered, 0 withheld.

### Functionalities provided

#### requirement

- `REQ-ASK-001` (accepted) — ETA estimation from measured waits (the ask feature)
- `REQ-ASK-002` (accepted) — ETA estimation API endpoint
- `REQ-ASK-003` (accepted) — Measured wait history storage
- `REQ-DISCOVERY-003` (accepted) — Full mDNS wire protocol via mdns-sd ServiceDaemon
- `REQ-REALTIME-001` (accepted) — WebSocket for real-time dashboard and mobile updates
- `REQ-SIGIL-001` (accepted) — Sigil/wsc signature verification for artifacts

#### verification

- `TEST-ASK-001` (accepted) — test_eta_estimator_no_samples + test_eta_estimator_with_samples + test_percentile — ETA estimation from measured waits
- `TEST-DISCOVERY-002` (accepted) — Full mDNS wire protocol via mdns-sd ServiceDaemon — register, browse, goodbye, TTL expiry
- `TEST-REALTIME-001` (accepted) — test_event_broadcaster + test_event_serialization + test_all_event_types — WebSocket/SSE real-time updates
- `TEST-SIGIL-001` (accepted) — test_sigil_verifier_creation + test_sha256_fallback_sign + test_sha256_fallback_verify + test_wsc_signature_serialization — sigil/wsc signatures

### Limitations in relation to the committed scope

None — every artifact committed to v0.3.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

4 of 10 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-ASK-001`
- `TEST-DISCOVERY-002`
- `TEST-REALTIME-001`
- `TEST-SIGIL-001`

6 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `REQ-ASK-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-ASK-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-ASK-003` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DISCOVERY-003` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-REALTIME-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-SIGIL-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `TEST-ASK-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DISCOVERY-002` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-REALTIME-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-SIGIL-001` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 10 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.2.0

29 artifact(s) in scope: 29 delivered, 0 withheld.

### Functionalities provided

#### design-decision

- `DD-DISCOVERY-001` (accepted) — mDNS via zeroconf crate, optional with static fallback
- `DD-DOCS-001` (accepted) — Embedded docs via clap + include_str! (like varve docs)
- `DD-PLATFORM-001` (accepted) — CiPlatform trait with GitHub + GitLab implementations
- `DD-RELEASE-001` (accepted) — Release pipeline via GitHub Actions + varve sbom + cosign
- `DD-RESEARCH-DB-001` (accepted) — BUILD: stay on SQLite + CRDT, evolve with per-table CRDT mapping
- `DD-RESEARCH-MDNS-001` (accepted) — ADOPT mdns-sd crate, BUILD discovery semantics in src/discovery/
- `DD-RESEARCH-TOOLS-001` (accepted) — BUILD: no adoption of similar tools — warte is purpose-built
- `DD-REUSE-001` (accepted) — Relay contract patterns: thin interfaces, seam traits, consumption contracts
- `DD-TEMPER-001` (accepted) — Temper as HTTP link, not code absorption

#### requirement

- `REQ-DISCOVERY-001` (accepted) — mDNS/Bonjour service discovery for multi-instance
- `REQ-DISCOVERY-002` (accepted) — Discovery security — mDNS over authenticated channel
- `REQ-DOCS-001` (accepted) — Embedded queryable documentation (warte docs)
- `REQ-DOCS-002` (accepted) — Documentation coverage check (warte docs check)
- `REQ-LAYER-001` (accepted) — Warte binary included in the varve toolchain layer
- `REQ-PLATFORM-001` (accepted) — Support enterprise GitHub (GHE) in addition to github.com
- `REQ-PLATFORM-002` (accepted) — Support GitLab CI as a platform
- `REQ-PLATFORM-003` (accepted) — Platform abstraction layer for CI providers
- `REQ-RELEASE-001` (accepted) — Release artifact pipeline (SHA256SUMS, SBOM, SLSA, cosign)
- `REQ-RELEASE-002` (accepted) — Release execution with V-model traceability gate
- `REQ-REUSE-001` (accepted) — Reuse relay's contract patterns for agent orchestration
- `REQ-TEMPER-001` (accepted) — Temper integration as optional link, not full absorption

#### verification

- `TEST-DISCOVERY-001` (accepted) — test_discovery_service_create + test_discovery_add_static_peer + test_discovery_update_peer — mDNS discovery service
- `TEST-DOCS-001` (accepted) — test_all_topics_load + test_coverage_passes + test_unknown_topic — embedded docs load and coverage check
- `TEST-DOCS-002` (accepted) — warte docs --check — CLI docs coverage command
- `TEST-LAYER-001` (accepted) — varve layer integration — warte.toml pin, varve-realms.toml, CLI conventions, embedded docs
- `TEST-PLATFORM-001` (accepted) — test_github_cloud/enterprise + test_gitlab_cloud/self_hosted + test_fleet_aggregator — CiPlatform trait
- `TEST-RELEASE-001` (accepted) — release-pipeline.sh + release.yml — SHA256SUMS, SBOM, SLSA, cosign, V-model gate
- `TEST-REUSE-001` (accepted) — test_agent_contract + test_consumption_contract + test_contract_serialization — relay contract patterns
- `TEST-TEMPER-001` (accepted) — test_temper_client_creation + test_temper_client_trailing_slash + test_temper_link_creation + test_dashboard_summary_serialization — temper HTTP link

### Limitations in relation to the committed scope

None — every artifact committed to v0.2.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

8 of 29 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `TEST-DISCOVERY-001`
- `TEST-DOCS-001`
- `TEST-DOCS-002`
- `TEST-LAYER-001`
- `TEST-PLATFORM-001`
- `TEST-RELEASE-001`
- `TEST-REUSE-001`
- `TEST-TEMPER-001`

1 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `DD-DISCOVERY-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-DOCS-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-PLATFORM-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-RELEASE-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-RESEARCH-DB-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-RESEARCH-MDNS-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-RESEARCH-TOOLS-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-REUSE-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-TEMPER-001` — 0 link(s), 0 source marker(s); decision-justification met
- `REQ-DISCOVERY-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-DISCOVERY-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-DOCS-001` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-DOCS-002` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-LAYER-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-PLATFORM-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-PLATFORM-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-PLATFORM-003` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-RELEASE-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-RELEASE-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-REUSE-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-TEMPER-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `TEST-DISCOVERY-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DOCS-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DOCS-002` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-LAYER-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-PLATFORM-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-RELEASE-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-REUSE-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-TEMPER-001` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 29 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled

## v0.1.0

61 artifact(s) in scope: 61 delivered, 0 withheld.

### Functionalities provided

#### aadl-analysis-result

- `AADL-ANALYSIS-001` (accepted) — SPAR EMV2 fault tree — single points of failure
- `AADL-ANALYSIS-002` (accepted) — SPAR connectivity analysis — data flow paths

#### aadl-component

- `AADL-COMP-001` (accepted) — EdgeProxy — sole internet-facing component (CTRL-004)
- `AADL-COMP-002` (accepted) — WarteInstance — the control tower (CTRL-001)
- `AADL-COMP-003` (accepted) — MobileClient — iOS/Android control (CTRL-003)
- `AADL-COMP-004` (accepted) — AgentRuntime — worker machine agent (CTRL-005)

#### aadl-flow

- `AADL-FLOW-001` (accepted) — Agent registration flow (mTLS authenticated)
- `AADL-FLOW-002` (accepted) — Artifact download flow (integrity-verified)
- `AADL-FLOW-003` (accepted) — Multi-instance sync flow (CRDT, mTLS)
- `AADL-FLOW-004` (accepted) — Mobile control flow (mTLS + biometric)

#### design-decision

- `DD-ARCH-001` (accepted) — Rust + Axum + Tokio for the server stack
- `DD-ARCH-002` (accepted) — SQLite for local state, with CRDT-based sync for multi-instance
- `DD-ARCH-003` (accepted) — mTLS with per-agent certificates for agent authentication
- `DD-ARCH-004` (accepted) — Sigil/wsc signatures for artifact integrity
- `DD-ARCH-005` (accepted) — Push-based revocation with active disconnection
- `DD-ARCH-006` (accepted) — WebSocket/SSE for real-time dashboard and mobile push
- `DD-ARCH-007` (accepted) — Edge proxy as the sole internet-facing component (Hetzner/Netcup)
- `DD-ARCH-008` (accepted) — Mobile app uses native iOS (Swift) and Android (Kotlin)
- `DD-ARCH-009` (accepted) — Migration from smithy — agent fleet data import
- `DD-ARCH-010` (accepted) — Migration from temper — repo hardening config import
- `DD-ARCH-011` (accepted) — CLI follows PulseEngine CLI conventions
- `DD-ARCH-012` (accepted) — RBAC with per-resource, per-action scoping

#### requirement

- `REQ-ACCESS-001` (accepted) — Access rights are managed via a central policy service
- `REQ-ACCESS-002` (accepted) — Access revocation propagates within 5 seconds
- `REQ-ACCESS-003` (accepted) — Access rights are scoped per resource and action
- `REQ-AGENT-REG-001` (accepted) — Agent registration requires cryptographic identity proof
- `REQ-AGENT-REG-002` (accepted) — Agent identity is bound to a unique agent ID
- `REQ-API-001` (accepted) — REST API with structured JSON responses
- `REQ-API-002` (accepted) — API requires authentication for all mutating operations
- `REQ-ARTIFACT-001` (accepted) — Artifact downloads require agent authentication
- `REQ-ARTIFACT-002` (accepted) — Artifacts carry verifiable integrity proofs
- `REQ-ARTIFACT-003` (accepted) — Artifact download is scoped to agent access rights
- `REQ-CAPTURE-001` (accepted) — Agents capture and report structured data
- `REQ-CAPTURE-002` (accepted) — Captured data is queryable via the API
- `REQ-CLI-001` (accepted) — CLI follows PulseEngine CLI conventions
- `REQ-DASH-001` (accepted) — Dashboard displays status with freshness indicators
- `REQ-DASH-002` (accepted) — Dashboard provides agent registry overview
- `REQ-DASH-003` (accepted) — Dashboard provides CI status overview
- `REQ-EDGE-001` (accepted) — Edge proxy terminates TLS 1.3 and enforces rate limiting
- `REQ-EDGE-002` (accepted) — Edge proxy validates all requests against schemas
- `REQ-EDGE-003` (accepted) — Edge proxy auto-patches within 24 hours
- `REQ-MIGRATE-001` (accepted) — Migrate agent fleet data from smithy
- `REQ-MIGRATE-002` (accepted) — Migrate repo hardening config from temper
- `REQ-SYNC-001` (accepted) — Instances synchronize state via an authenticated sync protocol
- `REQ-SYNC-002` (accepted) — Sync conflicts are detected and resolved deterministically
- `REQ-SYNC-003` (accepted) — Instance divergence is detected and alerted

#### verification

- `TEST-AADL-001` (accepted) — spar parse + spar analyze — AADL model validates with 0 errors
- `TEST-ACCESS-001` (accepted) — test_access_control_grant_revoke — grant, check, revoke lifecycle
- `TEST-ACCESS-002` (accepted) — test_access_control_orset_fail_safe — OR-Set concurrent grant+revoke resolves to REVOKED
- `TEST-AGENT-HB-001` (accepted) — test_agent_heartbeat — heartbeat updates agent health status
- `TEST-AGENT-REG-001` (accepted) — test_agent_registration — agent registration creates valid agent record
- `TEST-API-AUTH-001` (accepted) — test_token_issue_and_verify + test_token_expired + test_token_invalid_format + test_token_tampered + test_base64url_roundtrip — bearer token auth
- `TEST-ARTIFACT-001` (accepted) — test_artifact_store_and_integrity — store, download, verify integrity
- `TEST-CA-001` (accepted) — test_ca_create_and_load + test_issue_agent_cert — CA creates, loads, and issues agent certificates
- `TEST-CAPTURE-001` (accepted) — agent data capture and query endpoints
- `TEST-CLI-001` (accepted) — test_cli_version_format — CLI version follows PulseEngine conventions
- `TEST-DASH-001` (accepted) — dashboard overview returns agent and artifact counts
- `TEST-EDGE-003` (accepted) — Edge proxy auto-patching configuration — unattended-upgrades with 24h CVE window
- `TEST-HEALTH-001` (accepted) — health endpoint returns structured status with freshness
- `TEST-MIGRATE-001` (accepted) — test_migrate_smithy + test_migrate_temper — migration commands import data from smithy and temper
- `TEST-SYNC-001` (accepted) — test_sync_version_vectors — version vector increment, merge, divergence

### Limitations in relation to the committed scope

None — every artifact committed to v0.1.0 is release-ready.

### Result of verification measures

Each delivered artifact is judged by the traceability rules its schema declares for its type. A `verifies` link or source marker is evidence for any type.

25 of 61 delivered artifact(s) carry NO evidence (no `verifies` link, no source marker, and no traceability rule of their type met):

- `AADL-ANALYSIS-001`
- `AADL-ANALYSIS-002`
- `AADL-COMP-001`
- `AADL-COMP-002`
- `AADL-COMP-003`
- `AADL-COMP-004`
- `AADL-FLOW-001`
- `AADL-FLOW-002`
- `AADL-FLOW-003`
- `AADL-FLOW-004`
- `TEST-AADL-001`
- `TEST-ACCESS-001`
- `TEST-ACCESS-002`
- `TEST-AGENT-HB-001`
- `TEST-AGENT-REG-001`
- `TEST-API-AUTH-001`
- `TEST-ARTIFACT-001`
- `TEST-CA-001`
- `TEST-CAPTURE-001`
- `TEST-CLI-001`
- `TEST-DASH-001`
- `TEST-EDGE-003`
- `TEST-HEALTH-001`
- `TEST-MIGRATE-001`
- `TEST-SYNC-001`

11 delivered artifact(s) miss a warning- or info-severity rule (advisory; listed per artifact below).

- `AADL-ANALYSIS-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `AADL-ANALYSIS-002` — 0 link(s), 0 source marker(s); no rule governs its type
- `AADL-COMP-001` — 0 link(s), 0 source marker(s); aadl-component-has-allocation missing (info)
- `AADL-COMP-002` — 0 link(s), 0 source marker(s); aadl-component-has-allocation missing (info)
- `AADL-COMP-003` — 0 link(s), 0 source marker(s); aadl-component-has-allocation missing (info)
- `AADL-COMP-004` — 0 link(s), 0 source marker(s); aadl-component-has-allocation missing (info)
- `AADL-FLOW-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `AADL-FLOW-002` — 0 link(s), 0 source marker(s); no rule governs its type
- `AADL-FLOW-003` — 0 link(s), 0 source marker(s); no rule governs its type
- `AADL-FLOW-004` — 0 link(s), 0 source marker(s); no rule governs its type
- `DD-ARCH-001` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-002` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-003` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-004` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-005` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-006` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-007` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-008` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-009` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-010` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-011` — 0 link(s), 0 source marker(s); decision-justification met
- `DD-ARCH-012` — 0 link(s), 0 source marker(s); decision-justification met
- `REQ-ACCESS-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-ACCESS-002` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-ACCESS-003` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-AGENT-REG-001` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-AGENT-REG-002` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-API-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-API-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-ARTIFACT-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-ARTIFACT-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-ARTIFACT-003` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-CAPTURE-001` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-CAPTURE-002` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-CLI-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-DASH-001` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-DASH-002` — 2 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-DASH-003` — 1 link(s), 0 source marker(s); requirement-coverage missing (warning), requirement-verification met
- `REQ-EDGE-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-EDGE-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-EDGE-003` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-MIGRATE-001` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-MIGRATE-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-SYNC-001` — 2 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-SYNC-002` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `REQ-SYNC-003` — 1 link(s), 0 source marker(s); requirement-coverage met, requirement-verification met
- `TEST-AADL-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-ACCESS-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-ACCESS-002` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-AGENT-HB-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-AGENT-REG-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-API-AUTH-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-ARTIFACT-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-CA-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-CAPTURE-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-CLI-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-DASH-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-EDGE-003` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-HEALTH-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-MIGRATE-001` — 0 link(s), 0 source marker(s); no rule governs its type
- `TEST-SYNC-001` — 0 link(s), 0 source marker(s); no rule governs its type

### Approval

0 of 61 delivered artifact(s) carry a `reviewed-by` record.

### Not covered by this note

Automotive SPICE 11-03 names these release-note elements. rivet does not yet compute them, so they are declared rather than omitted:

- **application parameters** — not modelled
- **configurations and variants** — the variant subsystem exists but is not wired in here
- **intended area and environment of usage** — not modelled
- **impact to other components linked to the release** — needs the cross-repo externals graph
- **upstream or downstream compatibility** — not modelled
- **copyright and license information** — the cargo-deny licenses gate produces this but it is not read here
- **approval for delivery by responsible roles** — per-artifact reviewers are listed under Approval; a release-level sign-off by a responsible role is not recorded (REQ-333)
- **dependencies to other linked products** — not modelled
