# Contract: disk-hygiene v1

Version **1**, since warte v1.6.32. How agents on one machine keep its disks
from filling, and what a cleanup may remove (REQ-PRESSURE-003). warte measures
and judges; it never deletes anything. Whoever cleans up (an agent, a script,
the operator) reads warte's verdict first and removes only what this contract
allows.

## Where things go

- Build output, caches and temporary files go on a data volume, never on the
  system volume: point `CARGO_TARGET_DIR`, `TMPDIR` and tool caches there.
- A full system volume logs sessions out and loses unsaved work; a full data
  volume stops builds. Both are avoidable with the thresholds below.

## Thresholds

`GET /api/pressure?host=<host>` (see api-reference, "Disk pressure") gives each
volume a verdict from warte's `[pressure]` thresholds:

| Verdict | Meaning | What an agent does |
|---|---|---|
| `ok` | more than `low_gb` free | nothing |
| `low` | at most `low_gb` (default 20) free | finish what runs; start nothing large on that volume; remove its own `idle` holders |
| `critical` | at most `critical_gb` (default 10) free | start no build, test run or download on that volume until it is `low` again |
| `unknown` | no fresh report | treat as `low` |

## Holders and their verdict

`warte probe pressure` (on the machine, never over the network) lists what
agents hold: Claude Code scratchpads, `target/` directories of live sessions'
checkouts, and those checkouts' git worktrees. Each holder carries a `verdict`:

| `verdict` | When | May a cleanup remove it? |
|---|---|---|
| `in-use` | a live session works there, or any process's working directory is inside it | no |
| `dirty` | a worktree with uncommitted changes, or commits no remote has (`why` says which) | no |
| `unknown` | git failed or took longer than 20 s, or an older probe stored it | no |
| `idle` | none of the above | yes, by its owner or the operator |

- `live` says only whether a live agent session works in that directory. A
  session in the main checkout that runs a build in a sibling worktree leaves
  the worktree `live: false` but `in-use`. **A cleanup reads `verdict`, never
  `live`.**
- A worktree in a repository without a remote has all its commits on no
  remote, so it is `dirty` until they are pushed.
- `target/` directories and scratchpads hold nothing that cannot be rebuilt or
  is not kept elsewhere; nobody using them makes them `idle`.
- The probe judges only while a volume is low (at most every 10 minutes);
  `warte probe pressure --measure` judges now. A judgement is as old as
  "sized N min ago" says: check again right before removing.

## Rules

- **warte never deletes.** The probe and `warte probe pressure` read only.
- **Remove only `idle`.** Never `in-use`, `dirty` or `unknown`; when unsure,
  measure again.
- **Remove only your own, or as the operator.** A session removes holders it
  created; the operator may remove any `idle` holder.
- **Kill nothing to free space.** A process's holder is `in-use` until the
  process ends on its own or its owner stops it.
- **No paths leave the machine.** `/api/pressure` carries GB totals per kind
  per volume only; the read API is unauthenticated.

## Kill criteria

disk-hygiene v1 is broken if:
- a holder in which any process works, or a worktree with uncommitted changes
  or commits on no remote, is judged `idle`;
- a check that failed or timed out is judged anything but `unknown`;
- warte deletes, moves or truncates anything;
- `/api/pressure` publishes a path, session id, pid or repository name.
