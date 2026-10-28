# warte edge deployment — Terraform for Hetzner (edge) + Netcup (secondary)
#
# REQ-DEPLOY-001: Automated edge proxy deployment on Hetzner (edge) and
# Netcup (secondary instance), with WireGuard VPN mesh and DNS records.
#
# Architecture:
#   Internet → Edge Proxy (Hetzner, public IP, Caddy + WireGuard + ufw)
#                  │ WireGuard VPN (10.0.0.0/24)
#                  ├──→ warte instance 1 (home-LAN instance, internal)
#                  └──→ warte instance 2 (Netcup, internal)
#
# Usage:
#   terraform init
#   terraform plan -var="hcloud_token=..." -var="netcup_credentials=..."
#   terraform apply -var="hcloud_token=..." -var="netcup_credentials=..."

terraform {
  required_version = ">= 1.5"

  required_providers {
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "~> 1.45"
    }
    netcup = {
      source  = "nbering/netcup"
      version = "~> 1.0"
    }
  }
}

# ── Variables ───────────────────────────────────────────────────────

variable "hcloud_token" {
  description = "Hetzner Cloud API token"
  type        = string
  sensitive   = true
}

variable "netcup_customer" {
  description = "Netcup customer number"
  type        = string
}

variable "netcup_api_key" {
  description = "Netcup API key"
  type        = string
  sensitive   = true
}

variable "netcup_api_secret" {
  description = "Netcup API secret"
  type        = string
  sensitive   = true
}

variable "domain" {
  description = "Domain for warte (e.g., warte.pulseengine.eu)"
  type        = string
  default     = "warte.pulseengine.eu"
}

variable "ssh_public_key" {
  description = "SSH public key for server access"
  type        = string
}

variable "wireguard_private_key_edge" {
  description = "WireGuard private key for edge proxy (generate with: wg genkey)"
  type        = string
  sensitive   = true
}

variable "wireguard_private_key_instance1" {
  description = "WireGuard private key for instance 1"
  type        = string
  sensitive   = true
}

variable "wireguard_private_key_instance2" {
  description = "WireGuard private key for instance 2 (Netcup)"
  type        = string
  sensitive   = true
}

# ── Providers ───────────────────────────────────────────────────────

provider "hcloud" {
  token = var.hcloud_token
}

provider "netcup" {
  customer           = var.netcup_customer
  api_key            = var.netcup_api_key
  api_secret         = var.netcup_api_secret
  rate_limit_timeout = 60
}

# ── Hetzner: Edge Proxy ─────────────────────────────────────────────

resource "hcloud_server" "edge" {
  name        = "warte-edge"
  server_type = "cx22"  # 2 vCPU, 4 GB RAM
  image       = "ubuntu-24.04"
  location    = "fsn1"  # Falkenstein
  ssh_keys    = [var.ssh_public_key]

  public_net {
    ipv4_enabled = true
    ipv6_enabled = true
  }

  user_data = <<-EOF
    #!/bin/bash
    set -e

    # Update and install packages
    apt-get update
    apt-get install -y caddy wireguard ufw unattended-upgrades

    # Configure unattended-upgrades (auto-patching, 24h CVE window)
    cat > /etc/apt/apt.conf.d/50unattended-upgrades << 'APT'
    Unattended-Upgrade::Allowed-Origins {
      "$${distro_id}:$${distro_codename}-security";
    };
    Unattended-Upgrade::AutoFixInterruptedDpkg "true";
    Unattended-Upgrade::MinimalSteps "true";
    APT::Periodic::Update-Package-Lists "1";
    APT::Periodic::Unattended-Upgrade "1";
    APT::Periodic::AutocleanInterval "7";
    APT

    # Configure WireGuard
    cat > /etc/wireguard/wg0.conf << 'WG'
    [Interface]
    Address = 10.0.0.1/24
    ListenPort = 51820
    PrivateKey = ${var.wireguard_private_key_edge}

    # Instance 1 (home-LAN instance)
    [Peer]
    PublicKey = $(echo "${var.wireguard_private_key_instance1}" | wg pubkey)
    AllowedIPs = 10.0.0.2/32

    # Instance 2 (Netcup)
    [Peer]
    PublicKey = $(echo "${var.wireguard_private_key_instance2}" | wg pubkey)
    AllowedIPs = 10.0.0.3/32
    WG

    wg-quick up wg0
    systemctl enable wg-quick@wg0

    # Configure firewall (ufw)
    ufw default deny incoming
    ufw default allow outgoing
    ufw allow 22/tcp    # SSH
    ufw allow 51820/udp # WireGuard
    ufw allow 443/tcp   # HTTPS
    ufw --force enable

    # Caddy is configured via cloud-init or manually after deployment
    # See docs/deployment.md for the Caddyfile
  EOF

  labels = {
    role = "edge-proxy"
    project = "warte"
  }
}

# ── Netcup: Secondary Instance ──────────────────────────────────────

# Note: Netcup provider requires a server product ID. This is a template —
# adjust the product ID for your Netcup plan (e.g., "vServer G11").
# Netcup servers are provisioned via the Netcup API, not Terraform directly.
# Use the Netcup SCP or API to provision, then configure via cloud-init.

# For Netcup, we provide a cloud-init template instead:
locals {
  instance2_cloud_init = <<-EOF
    #!/bin/bash
    set -e

    apt-get update
    apt-get install -y wireguard

    # Configure WireGuard
    cat > /etc/wireguard/wg0.conf << 'WG'
    [Interface]
    Address = 10.0.0.3/24
    PrivateKey = ${var.wireguard_private_key_instance2}

    [Peer]
    PublicKey = $(echo "${var.wireguard_private_key_edge}" | wg pubkey)
    AllowedIPs = 10.0.0.1/32
    Endpoint = ${hcloud_server.edge.ipv4_address}:51820
    PersistentKeepalive = 25
    WG

    wg-quick up wg0
    systemctl enable wg-quick@wg0

    # Install warte (from varve layer or cargo install)
    # cargo install --git https://github.com/pulseengine/warte --locked
  EOF
}

# ── Outputs ─────────────────────────────────────────────────────────

output "edge_ip" {
  description = "Edge proxy public IP"
  value       = hcloud_server.edge.ipv4_address
}

output "edge_ipv6" {
  description = "Edge proxy IPv6"
  value       = hcloud_server.edge.ipv6_address
}

output "wireguard_network" {
  description = "WireGuard VPN network"
  value       = "10.0.0.0/24"
}

output "instance1_wg_ip" {
  description = "Instance 1 WireGuard IP (home-LAN instance)"
  value       = "10.0.0.2"
}

output "instance2_wg_ip" {
  description = "Instance 2 WireGuard IP (Netcup)"
  value       = "10.0.0.3"
}

output "deployment_notes" {
  description = "Post-deployment steps"
  value       = <<-EOT
    1. SSH to edge: ssh root@${hcloud_server.edge.ipv4_address}
    2. Install Caddy config (see docs/deployment.md)
    3. Provision Netcup instance with cloud-init:
       ${local.instance2_cloud_init}
    4. Configure DNS: ${var.domain} → ${hcloud_server.edge.ipv4_address}
    5. Deploy warte on internal instances:
       cargo install --git https://github.com/pulseengine/warte --locked
    6. Verify: curl https://${var.domain}/api/health
  EOT
}
