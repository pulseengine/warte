# Contract: job state v1

Version **1**, since warte v1.5.0. Scheduled jobs on any host report their last
run to warte through one small JSON file per job, written by a wrapper such as
`pe-job`, into `<jobs dir>/<host>/<job>.json` on the ground station
(REQ-JOBS-001). A deployment pins against this version; adding an optional field
does not bump it.

## State file

| Field | Type | |
|---|---|---|
| `name` | string | the job's name; default: the file name |
| `host` | string | the host; default: the directory name |
| `finished` | i64? | when the last run finished (unix seconds) |
| `exit` | i32? | its exit code; 0 is a success |
| `duration_s` | i64? | how long it ran |
| `expect_within_s` | i64? | how long the job may go without a success |
| `last_success` | i64? | when it last succeeded |

Every field is optional. Unknown fields are ignored. Everything read is
author-controlled text and is escaped where warte draws it.

## Expected-jobs manifest

One line per job that must exist: `<relative path> <window seconds>`, with `#`
starting a comment. Paths must end in `.json` and may not be absolute or contain
`.` or `..` parts; other lines are skipped. A listed file that is missing shows
as **missing**; a job with no success within its window shows as **overdue**.

Changes: none since v1.
