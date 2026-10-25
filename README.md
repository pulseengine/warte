# warte

Warte — the PulseEngine control tower. A CI dashboard with an ask feature and an API.

German: "watchtower" — and "bitte warte auf mich", "please wait for me".

## What it does

- **Overview** — all repos, all gates, green/red/pending
- **Ask** — "when will this PR run?" — ETA from measured waits
- **Queue** — runner pool, waiting jobs, hardware availability
- **Critical path** — longest chain of dependent gates
- **Resources** — who needs what (RAM, disk, runners, boards)
- **Outages** — GitHub status, fleet health
- **Friction** — tool errors, workarounds, build decisions
- **Registry** — agents register themselves (one agent per repo)
- **Schedule** — weekly cron table, who runs what when
- **Health** — factory-wide metrics from agent heartbeats

## CLI

```bash
warte serve               # start the dashboard server
warte status              # factory health
warte agents              # list registered agents
warte schedule            # weekly cron table
warte watch               # open dashboard in browser
```

## Roadmap

See [issues](https://github.com/pulseengine/warte/issues) for the full roadmap.

## License

MIT
