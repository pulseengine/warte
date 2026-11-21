#!/bin/sh
# Runs jig for a planted control: $JIG when set (a local build), else the
# pinned release, checked against the sha256 from its cosign-verified
# SHA256SUMS.txt (jig v0.1.0, x86_64 linux musl, 2026-11-21). Same version
# and sum as the jig workflows ci.yml pins.
set -eu
if [ -n "${JIG:-}" ]; then exec "$JIG" "$@"; fi
v=v0.1.0
sum=730cdfcd902544c945a31bfa20ae11e00d8990308b379a06bb9b6d3e5f9259b4
a="jig-$v-x86_64-unknown-linux-musl.tar.gz"
d="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/warte-controls-jig-$v"
if [ ! -x "$d/jig" ]; then
  mkdir -p "$d"
  curl -fsSL -o "$d/$a" "https://github.com/pulseengine/jig/releases/download/$v/$a"
  (cd "$d" && echo "$sum  $a" | sha256sum -c - >/dev/null && tar -xzf "$a")
fi
exec "$d/jig" "$@"
