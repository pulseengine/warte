# Deployment

Warte deploys on Hetzner (edge proxy + primary instance) and Netcup
(secondary instance), connected via WireGuard VPN.

## Edge proxy (Hetzner)

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
bind = "10.0.0.2:8080"
db_path = ".warte/state.db"
tls_enabled = false  # TLS terminated at edge proxy
sync_peers = ["https://10.0.0.3:8080"]
```

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
`ca_bundle`, `status_url`, `data_dir`. Keep the token in
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
- **Rate limits:** a scan lists recently pushed repos, their queued and
  running runs, and those runs' jobs (at most `max_runs`). Every listing is
  asked again with its ETag (`If-None-Match`); an unchanged one comes back
  304, which GitHub does not count against the primary limit. warte reads
  `x-ratelimit-remaining` from every answer and sends nothing while
  `rate_reserve` or fewer are left before the reset, so other users of the
  token keep headroom. On a primary or secondary rate limit the scan waits
  for the reset; a refused token (401,
  or 403 that is not a rate limit) is shown in Health and Outages and
  retried at the normal interval, so fixing the token needs no restart.
- **TLS-intercepting proxy:** warte uses rustls with bundled Mozilla roots
  and does not read the system store. Point `WARTE_CA_BUNDLE` (or
  `ca_bundle`) at the proxy's PEM CA; it is trusted in addition to the
  bundled roots. A file without a certificate stops startup.
- `[hub]` and `[github]` are exclusive: one CI source per instance.

## Deployment steps

See `docs/deployment.md` for the full deployment guide.
