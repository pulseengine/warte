#!/bin/sh
# Runs jig for a planted control: $JIG when set (a local build), else the
# pinned release, checked against the sha256 from its cosign-verified
# SHA256SUMS.txt (jig v0.1.1, x86_64 linux musl, 2026-11-22). Same version
# and sum as the jig workflows ci.yml pins.
set -eu
if [ -n "${JIG:-}" ]; then exec "$JIG" "$@"; fi
v=v0.1.1
sum=67ff0ebf11a7d96de90eeafad09b1a5315918bf88af47c7281a635459f5c34ac
a="jig-$v-x86_64-unknown-linux-musl.tar.gz"
d="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/warte-controls-jig-$v"
if [ ! -x "$d/jig" ]; then
  mkdir -p "$d"
  curl -fsSL -o "$d/$a" "https://github.com/pulseengine/jig/releases/download/$v/$a"
  (cd "$d" && echo "$sum  $a" | sha256sum -c - >/dev/null && tar -xzf "$a")
fi
exec "$d/jig" "$@"
