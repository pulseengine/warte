# Contract: lifecycle v1

Version **1**, since warte v1.6.31. How a machine starts and keeps the agent
sessions its [sessions v1](sessions.md) file lists (REQ-SESS-002). warte
publishes the commands; the operator's private configuration installs the
binary, renders the file and runs `start` at login.

## Commands

All take `--config <sessions.json>`, and check it as `warte sessions check`
does before anything else: a file that check refuses stops every command.

```bash
warte sessions status --config sessions.json
warte sessions start  --config sessions.json --reason login [--id warte] [--dry-run]
warte sessions pause  --config sessions.json warte --reason "owner away"
warte sessions resume --config sessions.json warte --reason back
warte sessions rm     --config sessions.json warte --reason retired
```

- `--state <dir>` (default `~/.local/state/warte/sessions`) holds warte's
  state per session, `<id>.json`, and the event log `events.jsonl`.
- `--claude-dir <dir>` (default `~/.claude`) is where Claude Code keeps its
  projects; warte reads only file names and times there, never content.
- `--format json` prints JSON instead of text.

## States

| State | Meaning |
|---|---|
| `running` | a process's arguments hold `--remote-control <name>` (or `--remote-control=<name>`) for the session's name |
| `missing` | enabled, not paused, and no such process: `start` starts it |
| `paused` | `pause` was run; `start` leaves it until `resume` |
| `disabled` | `"enabled": false` in the file |

## Start

`start` starts every `missing` session, or the one `--id` names (an `--id`
that is not `missing` exits 1 and says which state it is in). For each it
runs, with the session's `dir` as the working directory:

```
zellij attach --create-background warte-<id> -- \
  claude --remote-control <name> --model <model> --effort <effort> \
         --permission-mode <mode> [--resume <session id>]
```

`<session id>` is the newest `<uuid>.jsonl` in Claude Code's project
directory for `dir` (its path with every character other than an ASCII
letter or digit replaced by `-`); without one the session starts fresh.
Every argument is passed as one argument, never through a shell.
`--dry-run` prints each start as a JSON line, `{"id", "dir", "argv"}`, and
starts nothing.

## Rules

- **warte never kills a process.** `start` skips a running session;
  `pause` only stops later starts; there is no stop command. To restart,
  stop the session yourself and run `start --reason "<why>"`.
- **`rm` refuses while the session runs** (exit 1), checked by the name in
  the file and by the name it was last started with. It removes only
  warte's `<id>.json`; **transcripts are never deleted.**
- **Every start, pause, resume and rm has a reason**: 1 to 200 characters,
  no control characters, kept in `events.jsonl` with the time.
- **No hostnames.** The commands read the sessions file, the process list
  and Claude Code's project file names. A machine's role is in the file, its
  name is not.

## Kill criteria

lifecycle v1 is broken if:
- `start` starts a session that is running, paused or disabled;
- any command ends or signals a process;
- `rm` succeeds while the session runs, or deletes a file outside `--state`;
- a start runs anything but the argv above, or splits the name;
- a command proceeds with a file `warte sessions check` refuses.
