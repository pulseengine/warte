# Machine History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** warte stores every machine's numbers over time (7 d per minute, 90 d hourly) and draws them on the Machines view.

**Architecture:** A new `history` module owns a redb file in the data dir. `Machines::record` and the ground station's snapshot ingest project each accepted report/snapshot to numeric series and hand them to the store, which buffers commits. Read API and HTMX partials draw server-side SVG.

**Tech Stack:** Rust, redb (pure Rust KV), axum, HTMX, inline SVG, Playwright.

**Spec:** `docs/superpowers/specs/2026-11-05-machine-history-design.md`

## Global Constraints

- Store: redb, file `history.redb` in the same data dir as `machines.json`.
- Minute rows kept 7 days (`MINUTE_KEEP_S = 7*86400`); hourly rows kept 90 days by default (`HOUR_KEEP_S = 90*86400`, `[history] keep_days` overrides, minimum 8).
- Commits: `Durability::None` per sample; one `Immediate` commit every 300 s and on shutdown.
- Step: 60 s when `to - from <= 7 d`, else 3600 s; responses state it.
- A gap is JSON `null`, never 0 and never omitted.
- Series carry host, name, unit and numbers only (warte#46 rules).
- A store error never rejects or delays a report.
- Works with `[hub]` or `[github]` alike (Machines exists in both).

## Review Focus

- A host name with `/` or `..` in the API path → 404, no store access by path (hosts are keys, not files).
- `from > to`, or a range over 400 days → 400 with a plain message.
- A report whose `taken_at` is far in the future (bad clock) → stored at receive time, not in the future.
- Two reports in the same minute → the later replaces the earlier (no double count in rollups).
- Store file unwritable (read-only data dir) → warte serves, Machines view works, history panel says "history unavailable", one amber finding.

---

### Task 1: History store

**Files:**
- Create: `src/history/mod.rs`, Modify: `src/lib.rs` (`pub mod history;`), `Cargo.toml` (`redb`, `postcard` or `serde_json` for values)
- Test: `tests/history.rs`

**Interfaces:**
- Produces:
  - `pub struct History` (Clone via `Arc` inside), `History::open(data_dir: &str) -> Result<History, String>`
  - `History::record(&self, host: &str, t: i64, series: &[(String, f64)])` — `t` is bucketed to the minute inside; never panics, errors counted
  - `History::flush(&self)` — Immediate commit
  - `History::maintain(&self, now: i64)` — roll finished hours into `hour` (min/avg/max), prune minutes older than 7 d and hours older than keep
  - `History::hosts(&self) -> Vec<String>`, `History::metrics(&self, host) -> Vec<String>`
  - `History::series(&self, host, names: &[String], from: i64, to: i64) -> Vec<Series>`
  - `pub struct Series { metric: String, unit: &'static str, t0: i64, step: i64, values: Vec<Option<f64>>, min: Option<Vec<Option<f64>>>, max: Option<Vec<Option<f64>>> }` (Serialize)
  - `pub fn unit(metric: &str) -> &'static str` (by prefix: `_pct`→"%", `_mb`→"MB", `load.`→"", `_kbs`→"KB/s", `_ms`→"ms", `psi.`→"%", `sessions`→"", `uptime`→"s")
  - `History::errors(&self) -> u64`
- [ ] Tests (`tests/history.rs`), each with a tempdir:
  - `points_read_back_with_gaps_as_null`: record t=0 and t=180 for `load.1`; `series(.., 0, 240)` → step 60, values `[Some, None, None, Some, None]`.
  - `same_minute_replaces`: record 1.0 then 2.0 at t=10 and t=50 → one value 2.0.
  - `long_ranges_use_hourly_rollups`: record 120 minutes 0..119 values = minute index; `maintain(now = 3h)`; `series(.., 0, 8 d)` → step 3600, first value avg 29.5, min 0, max 59.
  - `pruning_drops_old_minutes_keeps_hours`: minutes at day 0, `maintain(now = 8 d)` → 60-s series over day 0 empty (all null), hourly still present.
  - `flushed_data_survives_reopen`: record, `flush`, drop, reopen → value present.
  - `hosts_and_metrics_list_what_was_recorded`.
- [ ] Run `cargo test --test history` → FAIL (module missing), implement, → PASS.
- [ ] Layout: tables `minute: (&str host, i64 t) -> &[u8]` (encoded `Vec<(u32 id, f64)>`), `hour: (&str, i64) -> &[u8]` (`Vec<(u32, f64 min, f64 avg, f64 max)>`), `names: &str -> u32`, `meta: &str -> i64` (e.g. `hub_imported`). Writes go through one `Mutex`-held write path; reads use read transactions.

### Task 2: Projection

**Files:**
- Create: `src/history/project.rs`
- Test: `tests/history.rs`

**Interfaces:**
- Produces:
  - `pub fn from_report(r: &crate::probe::Report) -> Vec<(String, f64)>`: `load.1/.5/.15`, `load.per_core` (load[1]/ncpu), `mem.used_pct`, `swap.used_mb`, `disk.used_pct{mount=<m>}`, `io.busy_pct{dev=}`, `io.await_ms{dev=}`, `io.read_kbs{dev=}`, `io.write_kbs{dev=}`, `psi.io.some`, `psi.io.full`, `sessions`, `uptime`
  - `pub fn from_point(p: &crate::hub::model::Point) -> Vec<(String, f64)>`: `load.1`, `psi.cpu`, `psi.mem`, `runners.online`, `runners.busy` (local_busy+gh_busy), `runners.offline` (runners-online), `runner.cpu{id=}`, `runner.mem_mb{id=}`, `queue.self_hosted`, `queue.hosted`, `queue.oldest_s`, `disk.used_pct{mount=runners}`
  - `pub fn minute(taken_at: i64, received_at: i64) -> i64` — `taken_at` unless `|taken_at - received_at| > 300`, then `received_at`; floored to 60.
- [ ] Tests: `projection_is_numbers_only` (report with `/Users/secret` session dir, tool path and inventory error "SECRET-ERROR": serialise output → contains neither); `missing_io_is_not_zero` (report with `io: None` → no `io.` names); `bad_clock_uses_receive_time` (taken_at = now + 3600 → minute(now)).

### Task 3: Wiring

**Files:**
- Modify: `src/probe/mod.rs` (`Machines` gains `history: OnceLock<History>`; `pub fn attach_history(&self, h: History)`; `record` calls `h.record(&host, minute(..), &from_report(..))` after storing)
- Modify: `src/hub/mod.rs` (same `attach_history`; both ingest paths call `h.record(snap.host, p.t, &from_point(&p))`; on attach, if meta `hub_imported` absent, record every existing history point then set it)
- Modify: `src/server/mod.rs` (open `History` in the Machines data dir; attach to Machines and hub; `AppState.history: Option<History>`; spawn loop: `flush` every 300 s, `maintain` every 3600 s; flush on shutdown signal), `src/config/mod.rs` (`[history] keep_days`)
- Modify: `src/probe/mod.rs::findings` caller in `src/dashboard/fleet.rs` or api to add amber "machine history is not being written (N errors)" when `errors() > 0` in the last hour.
- Test: `tests/history.rs::reports_through_machines_land_in_history`, `tests/hub.rs::hub_points_land_in_history_and_import_once`.

### Task 4: Read API

**Files:**
- Modify: `src/server/api.rs` (routes `/history/machines`, `/history/machines/{host}/metrics`, `/history/machines/{host}/series` in read_routes)
- Test: `tests/history.rs` via the router (same pattern as `tests/admission.rs`)
- [ ] Tests: `api_series_shape` (`{metric, unit, t0, step, values}` with `null`); `api_rejects_bad_ranges` (from>to → 400; > 400 d → 400); `api_unknown_host_404`; `api_without_history_404` (no machines configured → 404 "no history").
- [ ] Defaults: `to = now`, `from = to - 86400`; `metric` required.

### Task 5: Charts on the Machines view

**Files:**
- Create: `src/dashboard/charts.rs` (`pub fn spark(values: &[Option<f64>], max: Option<f64>, w: u32, h: u32, class: &str) -> String` — `<svg>` with one `<path>` whose `M` restarts after each `None`; `pub fn chart(series: &[Series], title, unit) -> String` larger, with min/max band when present, axis labels at start/end time and max value)
- Modify: `src/dashboard/machines.rs` (`card` gets three sparklines from history for the range: `load.per_core`, `mem.used_pct`, fullest `disk.used_pct{..}`; a `<details class="mhist" hx-get="/api/dashboard/machine-history-partial?host=..&range=.." hx-trigger="toggle once">` for the full charts), `view` takes `history: Option<&History>` and `range: Range`
- Modify: `src/dashboard/mod.rs` (`machines_partial` reads `?range=1h|24h|7d|90d`, default 24h; new `machine_history_partial`), `src/server/api.rs` route, `templates/dashboard.css` (`.spark`, `.mhist`, `.range` using existing tokens), range switch buttons with `hx-get` and remembered in `localStorage` (try/catch) in the dashboard template
- Findings: disk/load/memory findings render with a link `#hist-<host>` that opens that card's history details.
- Test: `tests/fleet_views.rs::machine_cards_show_sparklines_with_gaps` (a gap → two `M` in the path), `history_panel_absent_without_store`, `history_unavailable_says_so`; Playwright `machines.spec`: switching range to 7d re-renders with `step` 60 and the selection persists after reload.

### Task 6: Trace and docs

- rivet: REQ-MHIST-001 (store + retention + buffered commits), REQ-MHIST-002 (read API), REQ-MHIST-003 (charts), each `refines REQ-PROBE-001`, release v1.6.0, then implemented → verify with `// rivet: verifies` markers in the tests above.
- CHANGELOG Unreleased, `docs/topics/api-reference.md` (three endpoints), `docs/topics/machines.md` or the existing Machines topic (range switch, retention, `[history] keep_days`).
- Spec and this plan committed with the PR.
