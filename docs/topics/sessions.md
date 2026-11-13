# Contract: sessions v1

Version **1**, since warte v1.7.0. The agent sessions a machine runs are
listed in one JSON file (REQ-SESS-001). The operator's private configuration
renders the file per machine; warte publishes the format and the check, so a
bad list stops before it is deployed.

## The file

Written by the operator's configuration, mode 0644. It holds no secrets and
no hostnames: `role` names the kind of machine, not the machine. Example:

```json
{
  "version": 1,
  "sessions": [
    {"id": "warte", "name": "warte", "dir": "/Users/agent/git/warte",
     "model": "opus", "effort": "medium", "mode": "bypassPermissions",
     "role": "workstation", "enabled": true},
    {"id": "docs", "name": "Docs site", "dir": "/Users/agent/git/site",
     "model": "sonnet", "effort": "low", "mode": "acceptEdits",
     "role": "workstation", "enabled": false}
  ]
}
```

| Field | Type | Rule |
|---|---|---|
| `version` | number | exactly 1 |
| `sessions` | array | 1 to 64 entries |
| `id` | string | `[a-z0-9-]`, 1 to 32 characters, unique; the key local state is kept under |
| `name` | string | 1 to 64 characters after trimming, no control characters, unique ignoring case; the remote-control name |
| `dir` | string | an absolute path, no `..` segment, no control characters, at most 1024 bytes |
| `model` | string | 1 to 64 characters of `[A-Za-z0-9._-[]]`, passed to `claude --model` |
| `effort` | string | one of `low`, `medium`, `high`, `xhigh`, `max` |
| `mode` | string | one of `acceptEdits`, `auto`, `bypassPermissions`, `manual`, `dontAsk`, `plan`; passed to `claude --permission-mode` |
| `role` | string | `[a-z0-9-]`, 1 to 32 characters: a machine role such as `workstation` or `build`, never a hostname |
| `enabled` | boolean | required; `false` keeps the entry listed but not started |

- **Every field is required** except where the table says otherwise, so each
  choice (model, effort, permission mode) is visible in the configuration's
  review.
- **Unknown keys** at any level are refused. A new field means version 2.
- **Exact JSON shapes:** the file and each session are objects (an array in
  their place is refused); `version` is the integer `1`; a key given twice is
  refused; `enabled` is `true` or `false`, never a string.
- **Lengths count characters** as Unicode code points (`name`, after
  trimming); `dir` counts bytes.

## The check

```bash
warte sessions check sessions.json
```

Exit 0 prints `ok: sessions v1, N sessions, M enabled` (`--format json`
prints the same as an object). Any rule that fails exits 1 with
`[sessions] <file>: <field> …`, naming the field, for example
`sessions[2].mode must be one of …`. The configuration runs the check on the
rendered file before deploying it, with the warte release it pins.

## Kill criteria

sessions v1 is broken if:
- a file the table forbids passes the check;
- a file the table allows fails it;
- a failure does not name the field.
