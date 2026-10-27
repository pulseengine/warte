#!/usr/bin/env bash
# Deploy the current origin/main of warte to fourpi.local, next to smithy-hub.
#
#   scripts/deploy-fourpi.sh            # build origin/main, install, restart, check
#
# Idempotent. First run also creates the `warte` user, /var/lib/warte, the
# systemd unit, and a Caddy route https://fourpi.local/warte/. smithy-hub
# (/ci/) is not touched; warte's ground station keeps its own copy of
# the hub data until the agent feeds both hubs (smithy#19).
#
# Builds a static aarch64 binary here (cargo-zigbuild) because fourpi has no
# Rust toolchain. Each build is kept as /usr/local/lib/warte/warte-<sha>, and
# /usr/local/bin/warte points at the current one, so rollback is a symlink.
set -euo pipefail

HOST=${WARTE_DEPLOY_HOST:-fourpi.local}
TARGET=aarch64-unknown-linux-musl
CI_HOST_IP=${WARTE_CI_HOST_IP:-192.168.178.201}
cd "$(git rev-parse --show-toplevel)"

git fetch -q origin main
sha=$(git rev-parse --short=12 origin/main)
if [ "$(git rev-parse --short=12 HEAD)" != "$sha" ] || [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "refusing: check out a clean origin/main ($sha) first" >&2
  exit 2
fi

cargo zigbuild --release --locked --target "$TARGET" -q
bin=target/$TARGET/release/warte
ver=$("$bin" --version 2>/dev/null || true)
echo "built $sha (${ver:-cross binary})"

want=$(shasum -a 256 "$bin" | cut -d' ' -f1)
# Streamed over the sudo session into a root-owned directory: the binary never
# sits in a world-writable place (no /tmp) between upload and install.
ssh "$HOST" "sudo -n sh -c 'install -d -m 755 /usr/local/lib/warte && umask 022 && cat > /usr/local/lib/warte/.incoming-$sha'" < "$bin"
ssh "$HOST" "sudo -n bash -s" -- "$sha" "$CI_HOST_IP" "$want" <<'REMOTE'
set -euo pipefail
sha=$1; ci_host_ip=$2; want=$3

incoming=/usr/local/lib/warte/.incoming-$sha
got=$(sha256sum "$incoming" | cut -d' ' -f1)
if [ "$got" != "$want" ]; then
  rm -f "$incoming"
  echo "sha256 mismatch: built $want, received $got" >&2
  exit 1
fi
chmod 755 "$incoming"
mv -f "$incoming" "/usr/local/lib/warte/warte-$sha"
ln -sfn "/usr/local/lib/warte/warte-$sha" /usr/local/bin/warte

# --- first-time setup -------------------------------------------------------
id warte >/dev/null 2>&1 || useradd --system --home-dir /var/lib/warte --shell /usr/sbin/nologin warte
install -d -o warte -g warte -m 750 /var/lib/warte
if [ ! -d /var/lib/warte/hub ]; then
  # seed the ground station with smithy-hub's state; it reads it as-is
  cp -a /var/lib/smithy-hub /var/lib/warte/hub
  chown -R warte:warte /var/lib/warte/hub
fi
lan_ip=$(ip -4 -o addr show dev eth0 | awk '{print $4}' | cut -d/ -f1 | head -1)
if [ ! -f /var/lib/warte/warte.toml ]; then
  cat > /var/lib/warte/warte.toml <<EOF
bind = "127.0.0.1:8080"

[hub]
# smithy-hub keeps :8092; warte listens on :8094 for the parallel run.
# Source-IP allow-listing is spoofable (warte#41): LAN address only, not 0.0.0.0.
udp = "$lan_ip:8094"
allow = ["$ci_host_ip"]
data_dir = "/var/lib/warte/hub"
EOF
  chown warte:warte /var/lib/warte/warte.toml
fi
# migrate an earlier config that listened on every interface
if grep -q '^udp = "0.0.0.0:8094"' /var/lib/warte/warte.toml; then
  sed -i "s|^udp = \"0.0.0.0:8094\"|udp = \"$lan_ip:8094\"|" /var/lib/warte/warte.toml
fi
cat > /etc/systemd/system/warte.service <<'EOF'
[Unit]
Description=warte - PulseEngine control tower (dashboard + ground station)
After=network-online.target
Wants=network-online.target

[Service]
User=warte
Group=warte
WorkingDirectory=/var/lib/warte
ExecStart=/usr/local/bin/warte serve
Restart=always
RestartSec=5
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/lib/warte

[Install]
WantedBy=multi-user.target
EOF
# Caddy: serve warte at https://fourpi.local/warte/ like /photos and /ci.
# The dashboard uses relative URLs, so it works under the stripped prefix.
caddyfile=/opt/caddy-compose/Caddyfile
if ! grep -q 'handle_path /warte/\*' "$caddyfile" || grep -q 'fourpi.local:8443' "$caddyfile"; then
  cp "$caddyfile" "$caddyfile.bak.$(date +%Y%m%d-%H%M%S)"
  python3 - "$caddyfile" <<'PY'
import re, sys
p = sys.argv[1]
s = open(p).read()
# drop the earlier :8443 site
s = re.sub(r'\n# warte: PulseEngine control tower[^\n]*\nfourpi\.local:8443 \{.*?\n\}\n', '\n', s, flags=re.S)
route = ('\t# warte: PulseEngine control tower (latest main)\n'
         '\t# its read API is unauthenticated (agent data, access lists, downloads):\n'
         '\t# private-network clients only\n'
         '\tredir /warte /warte/\n'
         '\thandle_path /warte/* {\n'
         '\t\t@outside not remote_ip private_ranges\n'
         '\t\trespond @outside "forbidden" 403\n'
         '\t\treverse_proxy localhost:8080\n'
         '\t}\n')
if 'handle_path /warte/*' not in s:
    anchor = '\thandle {\n\t\troot * /srv'
    assert anchor in s, 'catch-all handle block not found'
    s = s.replace(anchor, route + anchor, 1)
with open(p, 'r+') as f:   # in place: single-file bind mount
    f.seek(0); f.write(s); f.truncate()
PY
  docker exec caddy caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile
fi

# link warte from fourpi's landing page (written in place: the file is a
# single-file bind mount, so an atomic replace would not reach the container)
index=/opt/caddy-compose/index.html
if grep -q 'fourpi.local:8443' "$index"; then
  python3 - "$index" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read().replace('href="https://fourpi.local:8443/"', 'href="/warte/"')
with open(p, 'r+') as f:
    f.seek(0); f.write(s); f.truncate()
PY
fi
if ! grep -q 'href="/warte/"' "$index"; then
  cp "$index" "$index.bak.$(date +%Y%m%d-%H%M%S)"
  python3 - "$index" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
anchor = '<li><a href="/ci/">'
i = s.index(anchor)
j = s.index('</li>', i) + len('</li>')
entry = ('\n  <li><a href="/warte/"><span class="name">warte</span><br>'
         '<span class="desc">PulseEngine control tower: dashboard, agents, ETA (latest main)</span></a></li>')
with open(p, 'r+') as f:
    f.seek(0); f.write(s[:j] + entry + s[j:]); f.truncate()
PY
fi

# --- (re)start ------------------------------------------------------------------
systemctl daemon-reload
systemctl enable -q warte
systemctl restart warte
for i in $(seq 1 30); do
  curl -fsS -m2 127.0.0.1:8080/api/health >/dev/null 2>&1 && break
  sleep 1
done
systemctl is-active warte
/usr/local/bin/warte --version
# keep the last 5 builds for rollback
ls -1t /usr/local/lib/warte/warte-* | tail -n +6 | xargs -r rm -f
REMOTE

echo "--- checks via Caddy"
for p in /warte /warte/ /warte/api/health /warte/ci/healthz /warte/dashboard.css /ci/healthz; do
  printf '%-22s %s\n' "$p" "$(curl -sk -o /dev/null -w '%{http_code}' "https://$HOST$p")"
done
echo "deployed $sha -> https://$HOST/warte/"
