#!/bin/sh
# Operator duties: costs and skills (nightly). Rights: costs:write, skills:write.
# warte does not ship the generators: they read your agents' own records,
# which stay on your machine. Each prints one report on stdout:
#   COSTS_GENERATOR   a costs report (docs/topics/costs-schema.md)
#   SKILLS_GENERATOR  optional: a skill-usage report
# warte rejects a report with free text or unknown fields (400) and keeps the
# last good one, so a broken generator shows as a late duty, not bad data.
. "$(dirname "$0")/common.sh"
: "${COSTS_GENERATOR:?set COSTS_GENERATOR}"
sh -c "$COSTS_GENERATOR" > "$work/costs.json"
api POST /api/costs "$work/costs.json"
if [ -n "${SKILLS_GENERATOR:-}" ]; then
	sh -c "$SKILLS_GENERATOR" > "$work/skills.json"
	api POST /api/skill-usage "$work/skills.json"
fi
