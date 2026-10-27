#!/usr/bin/env bash
# Compare two ground stations endpoint by endpoint: body, status code, content type.
#
#   scripts/hub-parity.sh <base-url-A> <base-url-B> [month ...]
#
# Used for the smithy-hub -> warte cut-over (REQ-MIGRATE-001): run both against the
# same data (or fed by the same agent during the parallel run) and require zero
# differences. Requests to A and B are fired together so time-dependent answers
# (ETA, "Ns old") see the same clock second; on live hubs a frame may still land
# between them, so re-run once before treating a diff in /api/state or /api/eta as real.
set -euo pipefail
A=${1:?base url A}; B=${2:?base url B}; shift 2
months=("$@"); [ ${#months[@]} -eq 0 ] && months=("$(date -u +%Y-%m)")
out=$(mktemp -d); trap 'rm -rf "$out"' EXIT
paths=(api/state 'api/history?hours=168' status.md api/openapi.json '' healthz
       'api/eta?repo=x/y&pr=1' 'api/report?month=bad' not-in-contract)
for m in "${months[@]}"; do paths+=("api/report?month=$m" "report.md?month=$m"); done
# ETA for every PR/branch the fleet currently shows, plus one that is not queued
paths+=('api/eta?repo=warte&pr=999999')
for q in $(curl -fsS "$A/api/state" | python3 -c '
import json,sys,urllib.parse
s=json.load(sys.stdin).get("snapshot") or {}
seen=set()
for e in (s.get("queue") or {}).get("queued", []):
    r=e.get("repo","")
    k=("pr=%d"%e["pr"]) if e.get("pr") else ("branch="+urllib.parse.quote(e["branch"],safe="")) if e.get("branch") else None
    if r and k and (r,k) not in seen:
        seen.add((r,k)); print("api/eta?repo=%s&%s"%(r,k))
' | head -20); do paths+=("$q"); done
n=0; d=0
for p in "${paths[@]}"; do
  f=$(printf '%s' "x$p" | tr '/?=&%' '_____')
  curl -s -m15 -o "$out/A$f" -w '%{http_code} %{content_type}' "$A/$p" > "$out/A$f.meta" &
  curl -s -m15 -o "$out/B$f" -w '%{http_code} %{content_type}' "$B/$p" > "$out/B$f.meta" &
  wait
  n=$((n+1))
  if ! cmp -s "$out/A$f" "$out/B$f" || ! cmp -s "$out/A$f.meta" "$out/B$f.meta"; then
    d=$((d+1)); echo "DIFF /$p  [$(cat "$out/A$f.meta")] vs [$(cat "$out/B$f.meta")]"
  fi
done
echo "compared $n requests, $d differ"
[ "$d" -eq 0 ]
