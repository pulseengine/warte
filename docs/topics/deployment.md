# Deployment

Warte can run behind an edge proxy with internal instances connected over
WireGuard. Real hosts and addresses belong in the deployment's private
infrastructure repository, not here.

## Edge proxy

- Caddy with TLS 1.3 termination
- Rate limiting (100 req/min per IP, 1000 req/min per agent)
- Request schema validation
- Attack pattern detection (WAF rules)
- Auto-patching (unattended-upgrades, 24h max for critical CVEs)
- Reverse proxy to internal instances over WireGuard

## Internal instances

- warte binary (from varve layer)
- SQLite local state
- WireGuard VPN to edge proxy
- No direct internet exposure

## Configuration

```toml
# warte.toml
bind = "<instance1-tunnel-ip>:8080"
db_path = ".warte/state.db"
tls_enabled = false  # TLS terminated at edge proxy
sync_peers = ["https://<instance2-tunnel-ip>:8080"]
# which repo depends on which, for the Critical Path view (top of the file)
repo_dependencies = [
  { repo = "jess", depends_on = ["relay", "gale"] },
]
```

The Critical Path view draws the longest chain of declared dependencies
on which at least one repo has CI queued or running, names its
bottleneck (the deepest such repo: every repo before it waits on it), and
gives a lower bound on release time: the measured p50 queue wait of each
repo's pools along the chain. Run times are not modelled. Only declared
dependencies are used so far; rivet externals and GitHub's dependency
graph as sources are still open (REQ-CRIT-001).

## Without a smithy agent: GitHub's API as the CI source

A team without a smithy agent (GitHub-hosted runners, or a GitHub Enterprise
tenant) points warte at GitHub's REST API. The scan fills the same model, so
every view and `/api/eta`, `/api/queue`, `/api/admission` and `/api/watch`
work unchanged.

```bash
WARTE_GITHUB_API_URL=https://api.<tenant>.ghe.com \
WARTE_GITHUB_ORG=<org> WARTE_GITHUB_TOKEN=<token> warte serve
```

| API base | for |
|---|---|
| `https://api.github.com` (default) | github.com |
| `https://api.<tenant>.ghe.com` | GitHub Enterprise Cloud with data residency (no `/api/v3`) |
| `https://<host>/api/v3` | GitHub Enterprise Server |

The base is set, never guessed from a web URL. The same settings as a
`[github]` section: `api_url`, `org`, `interval_s` (120), `active_days` (7),
`max_runs` (150), `rate_reserve` (150, env `WARTE_GITHUB_RATE_RESERVE`),
`self_hosted_labels` (env `WARTE_SELF_HOSTED_LABELS`, comma-separated),
`ca_bundle`, `status_url`, `data_dir`, `ledger_days` (2), `ledger_interval_s`
(600). Keep the token in
`WARTE_GITHUB_TOKEN`, not in the file.

- **Token scope:** read-only. A fine-grained token with "Actions: read" on the
  repositories; "Self-hosted runners: read" on the org adds the runner cards.
  Without it (403, or 404 for ordinary members) the queue still works:
  busy self-hosted runners are taken from their running jobs, so Queue,
  Overview, Ask, `/api/eta` and admission count running self-hosted work;
  idle runners are unknown, and Health says the runners are not listed.
- **Runner groups without a `self-hosted` label** (ARC scale sets, for
  example): list their labels in `self_hosted_labels`, such as
  `["arc-*"]`. `*` matches any run of characters, case-insensitive; a job
  with a matching label is counted as self-hosted. Empty keeps the default:
  only `self-hosted` does.
- **Runner pools and kinds** (REQ-RUNNERS-001..004): a pool is named from a
  job's labels. Self-hosted pools are named by their specific labels
  (`rust-cpu`); GitHub-hosted jobs go to `hosted:<runs-on label>`
  (`hosted:ubuntu-latest`, `hosted:linux-16core`), not one `hosted` pool.
  Each pool has a kind (`standard-hosted` for the images GitHub publishes as
  standard, `larger-hosted` for any other hosted label, `self-hosted`) and an
  OS. CI history charts queued and busy per pool, `/api/eta` estimates each
  pool from its own waits, and the monthly report shows runner minutes per
  pool (each job rounded up to the minute, as GitHub bills). Records from
  before v1.6.6 keep the pool `hosted`, shown as "GitHub-hosted (not split)".
- **A runner inventory where GitHub will not list runners:** `[[runners.pool]]`
  entries name, classify, size and price a pool. Where the org's runner list
  is refused, a `count` fills the pool with idle runners next to the busy ones
  seen from running jobs (marked `config`; busy runners of pools not in the
  config are marked `inferred`). Where GitHub lists runners, its list wins.
  `per_minute` prices the pool's minutes in the monthly report as a notional
  cost (list price, not a bill).
- **Job ledger and monthly report** (REQ-RUNNERS-005): every
  `ledger_interval_s` the scan reads the finished jobs of the scanned repos
  into the ground station's ledger, for the days from `ledger_days` back to
  today (`0` turns it off). A day is the UTC day a run was created. It counts
  as complete once it is over (plus an hour), every run created on it has
  finished and its jobs were read; until a day is complete the monthly report
  says preliminary. A pass costs one listing per repo and open day, sent with
  its ETag, and one job listing per finished run, read once; complete days are
  not listed again. What was read is kept in `ghledger.json` in `data_dir`.
  Repos not pushed within `active_days` are not read.

- **Rate limits:** a scan lists recently pushed repos, their queued and
  running runs, and those runs' jobs (at most `max_runs`). Every listing is
  asked again with its ETag (`If-None-Match`); an unchanged one comes back
  304, which GitHub does not count against the primary limit. warte reads
  `x-ratelimit-remaining` from every answer and sends nothing while
  `rate_reserve` or fewer are left before the reset, so other users of the
  token keep headroom. On a primary or secondary rate limit no request goes
  out until the reset, and the limit is shown in Outages as its own finding; a refused token (401,
  or 403 that is not a rate limit) is shown in Health and Outages and
  retried at the normal interval, so fixing the token needs no restart.
- **TLS-intercepting proxy:** warte uses rustls with bundled Mozilla roots
  and does not read the system store. Point `WARTE_CA_BUNDLE` (or
  `ca_bundle`) at the proxy's PEM CA; it is trusted in addition to the
  bundled roots. A file without a certificate stops startup.
- **Outages and alerts:** the same findings and notices as with smithy, from
  what GitHub shows: GitHub-hosted jobs queued 15 min or more per runner
  type, queued runs overdue for their pool, self-hosted runners offline (with
  the runner scope), GitHub's status, and the scan itself failing or
  rate-limited. Host findings (disk, pressure, OOM kills, link loss) need
  host telemetry and do not appear. Alerts are recorded as `notice.v1` under
  the agent `warte-alerts` on this instance; point `PULSEENGINE_WARTE_URL`
  at it where `notice.py digest` runs. GitHub Enterprise Server has no
  githubstatus.com: set `status_url = "off"`.
- `[hub]` and `[github]` are exclusive: one CI source per instance.

## Enterprise deployment notes

### GitHub Enterprise Cloud with data residency, behind a proxy

The API base of a data-residency tenant is `https://api.<tenant>.ghe.com`
(no `/api/v3`). Where a TLS-intercepting proxy re-signs traffic, give warte
the proxy's CA as PEM; it is trusted in addition to the bundled roots:

```toml
# warte.toml
[github]
api_url = "https://api.<tenant>.ghe.com"
org = "<org>"
ca_bundle = "/etc/ssl/certs/corp-proxy-ca.pem"
# runner groups whose jobs carry no `self-hosted` label
self_hosted_labels = ["<group-prefix>-*"]

[runners]
currency = "USD"

# a self-hosted pool the token cannot list: its size, so idle capacity shows
[[runners.pool]]
labels = ["<group-prefix>-build"]
name = "Build runners"
count = 6
cores = 16
memory_gb = 64

# a GitHub-hosted larger runner: name it and price its minutes
[[runners.pool]]
labels = ["<larger-runner-label>"]
name = "Linux, 16 cores"
kind = "larger-hosted"   # standard-hosted | larger-hosted | self-hosted
os = "linux"             # linux | macos | windows | unknown
per_minute = 0.064
```

```bash
WARTE_GITHUB_TOKEN=<read-only token> warte serve
```

A token without "Self-hosted runners: read" still works: running
self-hosted work is taken from the jobs (see above).

### One Linux binary for older and newer distributions

The release's Linux binaries link glibc and need a glibc at least as new as
the build image's for the symbols they use. For older distributions, or
one binary across many, build a static musl binary:

```bash
cargo install cargo-zigbuild        # needs zig on PATH
cargo zigbuild --release --target x86_64-unknown-linux-musl
cargo zigbuild --release --target aarch64-unknown-linux-musl
file target/x86_64-unknown-linux-musl/release/warte   # "statically linked"
```

A static binary does not use the system's name service switch, so `.local`
names (mDNS) do not resolve: give the probe the ground station by an
address or a DNS name, or pin the name in `/etc/hosts`.

### Authentication in front, probe routes left open

warte's dashboard and read API have no login of their own yet (#46). Put
one in front, for example HTTP basic auth in Caddy, and leave the four
probe routes open: they carry their own authentication (a one-time token
for enrollment, a signature by the enrolled key for reports, renewals and
feeds)
and probes cannot answer a password prompt.

```caddyfile
warte.example.internal {
	@guarded not {
		method POST
		path /api/probe/enroll /api/probe/report /api/probe/renew /api/probe/feed/temper
	}
	# caddy hash-password --plaintext '<password>'
	basic_auth @guarded {
		admin <bcrypt hash>
	}
	reverse_proxy 127.0.0.1:8080
}
```

Only `POST` to exactly those paths passes without the password; everything
else, including `GET` on the same paths, asks for it.

## Deployment steps

See `docs/deployment.md` for the full deployment guide.
