# Migration guide

## From the original hub

the original hub is now warte's ground station (`warte hub`, DD-SMITHY-001). There is
**nothing to import**: `warte hub` uses the original hub's flags and reads its data
directory (`hub-state.json` + `ledger/`) as-is.

1. **Run side by side.** Start `warte hub` on another port pair with its own copy of
   the data directory, and have the fleet agent send to both hubs (needs a
   repeatable `--hub` in the agent: the agent).
2. **Compare.** `scripts/hub-parity.sh https://<host>/ci/ http://<warte-hub>`
   checks every endpoint, ETA for every queued PR, and the monthly reports.
   Require zero differences, re-running once to rule out a frame landing between
   the two requests.
3. **Cut over.** In `the hub's systemd unit`, replace `ExecStart=/usr/local/bin/<old hub binary> …`
   with `ExecStart=/usr/local/bin/warte hub …` (same flags), restart, and keep
   Caddy's `/ci/` route. Or run the ground station inside `warte serve` with a
   `[hub]` section (see `warte docs ground-station`).

Day manifests are kept in memory by both hubs, so a freshly started one reports
"preliminary" until the agent's next ledger pass re-sends them.

`warte migrate` for the original hub existed before v1.4.0. It imported a JSON format the original hub never
produced, and has been removed.

## From temper (GitHub App)

Temper is linked, not absorbed. Warte reads temper's config and shows
its compliance status in the dashboard.

```bash
warte migrate temper --path /path/to/temper/config.yml
```

```toml
# Configure the temper link in warte.toml
[temper]
url = "http://temper.internal:3000"
token = "..."
```
