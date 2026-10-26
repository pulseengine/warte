# Warte edge proxy deployment (Hetzner / Netcup)
#
# The edge proxy is the ONLY internet-facing component. It terminates TLS 1.3,
# enforces rate limiting, validates requests, and proxies to internal warte
# instances over a private WireGuard VPN.
#
# Deploy on: Hetzner Cloud (edge) + Netcup VPS (secondary instance)
# Internal:  WireGuard VPN between edge and internal instances

## Architecture
#
#   Internet → Edge Proxy (Hetzner, public IP)
#                  │ WireGuard VPN (10.0.0.0/24)
#                  ├──→ warte instance 1 (fourpi.local replacement, internal)
#                  └──→ warte instance 2 (Netcup, internal)
#
#   Mobile (iOS/Android) ──mTLS──→ Edge Proxy ──→ warte instances
#   Agents (worker machines) ──mTLS──→ Edge Proxy ──→ warte instances
#   Dashboard (browser) ──TLS──→ Edge Proxy ──→ warte instances

## Caddy configuration (recommended — automatic TLS, HTTP/3)

# /etc/caddy/Caddyfile
{
    email admin@pulseengine.eu
}

warte.pulseengine.eu {
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
    reverse_proxy 10.0.0.2:8080 10.0.0.3:8080 {
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
Address = 10.0.0.1/24
ListenPort = 51820
PrivateKey = <EDGE_PRIVATE_KEY>

# Internal instance 1 (fourpi.local replacement)
[Peer]
PublicKey = <INSTANCE1_PUBLIC_KEY>
AllowedIPs = 10.0.0.2/32

# Internal instance 2 (Netcup)
[Peer]
PublicKey = <INSTANCE2_PUBLIC_KEY>
AllowedIPs = 10.0.0.3/32

# /etc/wireguard/wg0.conf (on warte instance 1)
[Interface]
Address = 10.0.0.2/24
PrivateKey = <INSTANCE1_PRIVATE_KEY>

[Peer]
PublicKey = <EDGE_PUBLIC_KEY>
AllowedIPs = 10.0.0.1/32
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

# Instance 1 (fourpi.local replacement)
# bind = "10.0.0.2:8080"
# tls_enabled = false  # TLS terminated at edge proxy
# sync_peers = ["https://10.0.0.3:8080"]

# Instance 2 (Netcup)
# bind = "10.0.0.3:8080"
# tls_enabled = false  # TLS terminated at edge proxy
# sync_peers = ["https://10.0.0.2:8080"]

## Deployment steps

# 1. Provision Hetzner Cloud server (edge proxy)
#    - Ubuntu 24.04 LTS, 2 vCPU, 4 GB RAM, 40 GB SSD
#    - Public IP, SSH key access
#
# 2. Provision Netcup VPS (secondary instance)
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
#    curl https://warte.pulseengine.eu/api/health
