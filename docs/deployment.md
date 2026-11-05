# Warte edge proxy deployment (example)
#
# The edge proxy is the ONLY internet-facing component. It terminates TLS 1.3,
# enforces rate limiting, validates requests, and proxies to internal warte
# instances over a private WireGuard VPN.
#
# Example layout: one edge host + internal warte instances. Hosts, providers,
# addresses and names here are placeholders; a real deployment keeps its own
# values in its private infrastructure repository.
# Internal:  WireGuard VPN between edge and internal instances

## Architecture
#
#   Internet → Edge Proxy (edge host)
#                  │ WireGuard VPN (<tunnel-range>)
#                  ├──→ warte instance 1 (internal)
#                  └──→ warte instance 2 (internal)
#
#   Mobile (iOS/Android) ──mTLS──→ Edge Proxy ──→ warte instances
#   Agents (worker machines) ──mTLS──→ Edge Proxy ──→ warte instances
#   Dashboard (browser) ──TLS──→ Edge Proxy ──→ warte instances

## Caddy configuration (recommended — automatic TLS, HTTP/3)

# /etc/caddy/Caddyfile
{
    email admin@example.org
}

warte.example.org {
    # TLS 1.3 only
    tls {
        protocols tls1.3
    }

    # Rate limiting (per-IP)
    rate_limit {
        zone static {
            key    {remote_host}
            events 100
            window 1m
        }
    }

    # Security headers
    header {
        Strict-Transport-Security "max-age=31536000; includeSubDomains; preload"
        X-Content-Type-Options    "nosniff"
        X-Frame-Options           "DENY"
        Referrer-Policy           "strict-origin-when-cross-origin"
        Content-Security-Policy   "default-src 'self'"
    }

    # Reverse proxy to internal warte instance (over WireGuard)
    reverse_proxy <instance1-tunnel-ip>:8080 <instance2-tunnel-ip>:8080 {
        # Health check
        health_uri /api/health
        health_interval 30s
        health_timeout 5s

        # Load balancing (failover)
        fail_duration 10s
        max_fails 3
        unhealthy_latency 5s
    }

    # Block common attack patterns
    @blocked {
        path /wp-* /xmlrpc.php /.git/* /.env* /admin*
    }
    respond @blocked 403
}

## WireGuard configuration

# /etc/wireguard/wg0.conf (on edge proxy)
[Interface]
Address = <edge-tunnel-ip>/<prefix>
ListenPort = 51820
PrivateKey = <EDGE_PRIVATE_KEY>

# Internal instance 1
[Peer]
PublicKey = <INSTANCE1_PUBLIC_KEY>
AllowedIPs = <instance1-tunnel-ip>/32

# Internal instance 2
[Peer]
PublicKey = <INSTANCE2_PUBLIC_KEY>
AllowedIPs = <instance2-tunnel-ip>/32

# /etc/wireguard/wg0.conf (on warte instance 1)
[Interface]
Address = <instance1-tunnel-ip>/<prefix>
PrivateKey = <INSTANCE1_PRIVATE_KEY>

[Peer]
PublicKey = <EDGE_PUBLIC_KEY>
AllowedIPs = <edge-tunnel-ip>/32
Endpoint = <EDGE_PUBLIC_IP>:51820
PersistentKeepalive = 25

## Auto-patching (unattended-upgrades)

# /etc/apt/apt.conf.d/50unattended-upgrades
Unattended-Upgrade::Allowed-Origins {
    "${distro_id}:${distro_codename}-security";
};
Unattended-Upgrade::AutoFixInterruptedDpkg "true";
Unattended-Upgrade::MinimalSteps "true";
Unattended-Upgrade::InstallOnShutdown "false";
Unattended-Upgrade::Remove-Unused-Kernel-Packages "true";
Unattended-Upgrade::Remove-Unused-Dependencies "true";
Unattended-Upgrade::Automatic-Reboot "false";

# Critical CVEs patched within 24h
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
APT::Periodic::AutocleanInterval "7";

## Firewall (ufw)

# Only allow SSH, WireGuard, and HTTPS
ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp    # SSH
ufw allow 51820/udp # WireGuard
ufw allow 443/tcp   # HTTPS
ufw enable

## warte instance configuration (warte.toml on internal instances)

# Instance 1
# bind = "<instance1-tunnel-ip>:8080"
# tls_enabled = false  # TLS terminated at edge proxy
# sync_peers = ["https://<instance2-tunnel-ip>:8080"]

# Instance 2
# bind = "<instance2-tunnel-ip>:8080"
# tls_enabled = false  # TLS terminated at edge proxy
# sync_peers = ["https://<instance1-tunnel-ip>:8080"]

## Deployment steps

# 1. Provision the edge host
#    - Ubuntu 24.04 LTS, 2 vCPU, 4 GB RAM, 40 GB SSD
#    - Public IP, SSH key access
#
# 2. Provision the second instance
#    - Ubuntu 24.04 LTS, 2 vCPU, 4 GB RAM
#    - Private network with edge proxy
#
# 3. Set up WireGuard VPN between edge and instances
#    wg-quick up wg0
#
# 4. Install Caddy on edge proxy
#    apt install caddy
#    systemctl enable caddy
#    systemctl start caddy
#
# 5. Deploy warte on internal instances
#    cargo install --path .
#    systemctl enable warte
#    systemctl start warte
#
# 6. Configure auto-patching
#    apt install unattended-upgrades
#    dpkg-reconfigure unattended-upgrades
#
# 7. Verify
#    curl https://warte.example.org/api/health
