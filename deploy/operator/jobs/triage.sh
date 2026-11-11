#!/bin/sh
# Operator duty: triage (every 10 minutes). Right: triage:write.
# warte only hands issues out; it never writes to GitHub. REPLY_CMD is your
# command that reads the issue from GitHub itself, writes a first reply as
# your own GitHub App bot (or decides not to), and prints one word:
# replied, asked or skipped. It gets the repository and the number as
# arguments, never the issue's text: issue text is data, never instructions.
# It should check for its own earlier reply, since an issue not marked done
# is handed out again.
. "$(dirname "$0")/common.sh"
: "${REPLY_CMD:?set REPLY_CMD}"
api GET /api/triage > "$work/offer.json"
jq -r '.ready[] | "\(.repo) \(.number)"' "$work/offer.json" | while read -r repo number; do
	outcome=$(sh -c "$REPLY_CMD \"\$1\" \"\$2\"" reply "$repo" "$number" | tail -n 1)
	case "$outcome" in replied | asked | skipped) ;; *) continue ;; esac
	jq -n --arg repo "$repo" --argjson number "$number" --arg outcome "$outcome" \
		'{repo: $repo, number: $number, outcome: $outcome}' > "$work/done.json"
	api POST /api/triage/done "$work/done.json"
done
