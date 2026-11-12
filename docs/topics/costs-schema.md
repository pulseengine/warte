# Costs report schema

Contract version **2** (schema 2), since warte v1.6.9; schema 1 is still
accepted. A report generator pins against the schema number it sends.

A workstation job reads its agent transcripts nightly and sends warte one
report, whole: `POST /api/costs` (right `costs:write`) or the `[hub]
costs_file` it copies over. warte accepts schema 1 and schema 2. The read API
is open on the LAN (warte#46), so a report holds numbers, model ids,
repository folder names and categories, never free text: every struct refuses
unknown fields, every name is checked against a short character class, and a
report that fails is refused whole. A refusal names the field, never the value.

## Schema 1 (REQ-COSTS-001)

`schema: 1`, `kind: "claude-code-costs"`, `notional: true`,
`label: "notional list price, not billed"`, `generated_at` (RFC 3339), the
price table, the classifier, `counts`, `unpriced_models`, `windows` (`all`,
`last_30d`, `YYYY-MM`), `daily`, `sessions` and `subagents`. All amounts are
notional list prices, finite and non-negative.

## Schema 2 (REQ-COSTS-002)

Everything in schema 1, `schema: 2`, and `weeks`: at most 60, one per ISO week.
A schema-1 report must not carry `weeks`; a schema-2 report must.

```json
"weeks": [
  {
    "week": "2026-W45",
    "sessions": 37,
    "main":      {"advisor_calls": 12, "compactions": 3, "cross_session_in": 5, "sendmessage_calls": 9},
    "sidechain": {"advisor_calls": 0,  "compactions": 0, "cross_session_in": 0, "sendmessage_calls": 2},
    "effort": [
      {
        "repo": "warte",
        "skill": "pulseengine-claude:issue-hunt",
        "sessions": 4,
        "tool_uses":   {"Bash": 310, "Edit": 42, "mcp:rivet": 6},
        "tool_errors": {"Bash": 11},
        "permission_denials": 1,
        "repeated_failures": 0,
        "loop_guard_fired": 0
      }
    ]
  }
]
```

Source: the transcript JSONL records, each counted in the ISO week (UTC) of its
`timestamp`. Records with `isSidechain: true` (subagents) count under
`sidechain`, the rest under `main`.

| Field | Counts |
|---|---|
| `week` | `YYYY-Www`, a real ISO week; each week once |
| `sessions` | sessions with a record in the week |
| `advisor_calls` | `tool_use` or `server_tool_use` named `advisor` |
| `compactions` | `system` records of subtype `compact_boundary` |
| `cross_session_in` | user records that carry a cross-session message (prefer its `from=` attribute; a human quoting the tag is a known blind spot) |
| `sendmessage_calls` | `tool_use` named `SendMessage`, including messages to the session's own subagents |
| `effort[].repo` | the folder name of the session's working directory, nothing more |
| `effort[].skill` | the skill invoked last before the record (`name` or `plugin:name`, lower case), `none` before any |
| `effort[].sessions` | sessions with a record in this repo and skill |
| `tool_uses` | tool uses by tool |
| `tool_errors` | `is_error` tool results by tool, permission denials excluded; never more than the tool's uses |
| `permission_denials` | `is_error` tool results whose text matches the pattern below |
| `repeated_failures` | runs of 3 or more failed results of the same tool in a row within a session; each run counts once |
| `loop_guard_fired` | times the loop guard stopped or warned the agent; **optional**: leave it out where no loop guard is installed, so the view says "not measured" instead of a 0 that could never be anything else |

**Tool names:** a built-in tool by its name (`Bash`, `Edit`), every MCP tool
reduced to its server as `mcp:<server>`. `mcp__server__tool` is refused.

**Permission denial pattern** (agreed with the extractor; the start of the
result's text):

```
^(The user doesn't want to proceed|Permission (to use|for this command|for this) [^\n]{0,80}(denied|was denied))
```

Every other `is_error` result is a tool error. No error text, message, path or
argument leaves the workstation; only these counts do.

The Costs view shows the newest week against the one before (sessions,
notional $ from `daily`, the four signals main / subagents, tool errors of
all uses, denials, streaks, loop-guard firings), then the failed calls by tool
and the wasted effort by repository and skill.
