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

## Deployment steps

See `docs/deployment.md` for the full deployment guide.
