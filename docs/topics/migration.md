# Migration guide

## From smithy (internal dashboard)

```bash
warte migrate smithy --path /path/to/smithy/data
```

Imports: agent registrations, captured session data, CI status history.

## From temper (GitHub App)

Temper is linked, not absorbed. Warte reads temper's config and shows
its compliance status in the dashboard.

```bash
# Configure temper link in warte.toml
[temper]
url = "http://temper.internal:3000"
token = "..."
```

## From fourpi.local

The internal dashboard on fourpi.local is replaced by warte. Migrate
data via the smithy migration command, then decommission fourpi.local.
