# Diagnostics Settings Section Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Native Settings page, "Diagnostics", with Report an issue, Open logs folder, Copy diagnostics, and the last 7 days of failure counters.

**Architecture:** `Section::Diagnostics` is registered in `settings/model.rs` with one line per table. The page lives in `settings/pages/diagnostics.rs`. Its buttons emit `SettingsEvent`s, and the shell handles them with the same helpers the command palette uses. Those helpers move out of `shell/quick_open.rs` into `shell/diagnostics.rs`. Each visit to the page loads the counters off the UI thread through `gespan_core::diagnostics::metrics::last_days(7)`. Leaving the page drops both the rows and the in-flight load.

**Tech Stack:** Rust, GPUI, `gespan-core` diagnostics (#68, #71, #72).

**Spec:** GitHub issue #74 (rescoped 2026-10-25), part of epic #45.

## Global Constraints

- No consent, no first-run prompt, no queue, no install ID, and no network code.
- Don't edit `gespan-core/src/diagnostics/collect.rs` or `shell/report.rs` (#162 is open against them).
- Keep the `Section` registration in `settings/model.rs` to single lines. The page goes in its own file.
- Gespan names only (`src/test/legacyNames.test.ts`).
- The section sits under the App group, right after General.

## Review Focus

- **Leaving the page before the load lands.** A late load must not fill stale counters, and the next visit must load again. Test: `leaving_mid_load_drops_it_and_the_next_visit_reloads` (Task 2).
- **The store can't be read** (the file is locked or corrupt). The page shows why instead of an empty state. Test: `a_store_error_is_shown_not_an_empty_state` (Task 2).
- **Only successes recorded.** These are not failures, so the page shows the empty state. Test: `successes_alone_are_the_empty_state` (Task 2).
- **The same failure across days and app versions.** It shows as one row with the summed count. Test: `failures_sum_across_days_and_versions` (Task 2).
- **Search terms people use** ("logs", "diagnostics", "crash", "bug") land on this page. Test: `logs_and_diagnostics_land_on_the_diagnostics_page` (Task 1).

---

### Task 1: Register the section and make it searchable

**Files:**
- Modify: `crates/gespan-gpui/src/settings/model.rs`: add `Diagnostics` to the enum, `GROUPS` (App, after General), `ALL` (after General, size 13), `id` "diagnostics", `label` "Diagnostics", `description`, `keywords`, and `icon` "bug". Add `INDEX` rows `report-issue`, `open-logs`, `copy-diagnostics` and `failure-counters`.
- Modify: `crates/gespan-gpui/src/settings/pages/mod.rs`: add `mod diagnostics;` and the `Section::Diagnostics` arm.
- Create: `crates/gespan-gpui/src/settings/pages/diagnostics.rs`: a stub page, so the build stays green.
- Modify: `crates/gespan-gpui/src/shell/settings.rs`: add `"settings-nav-diagnostics"` to the nav click-through test, right after general.

- [ ] Test `logs_and_diagnostics_land_on_the_diagnostics_page`: for each of "logs", "diagnostics", "crash" and "bug", `search(q)[0].section == Section::Diagnostics`. `search("diagnostics")` also contains the page result (`setting: None`).
- [ ] Test `diagnostics_sits_under_app_after_general`: the App group's sections start `[General, Diagnostics, ..]`, and `Section::ALL` contains it.
- [ ] Watch both tests fail (compile error), then implement, then watch them pass.

### Task 2: Counters view

**Files:** `settings/pages/diagnostics.rs`, `settings/mod.rs` (fields, plus enter and leave hooks in `new` and `select_section`).

**Interfaces:**
- `pub(super) struct CounterRow { name: String, kind: ErrorKind, count: u64 }`
- `pub(super) fn failure_rows(counts: &[Count]) -> Vec<CounterRow>`: skips successes (`kind: None`), sums by (name, kind), and sorts by count descending, then name.
- `pub(super) type CounterSource = fn(u32) -> Result<Vec<Count>, String>`: defaults to `metrics::last_days`. Tests replace it.
- `SettingsView` fields: `counters: Option<Result<Vec<CounterRow>, String>>`, `counters_loading: Option<Task<()>>`, `counter_source: CounterSource`.
- `fn enter_diagnostics(&mut self, cx)`: starts the background load. `fn leave_diagnostics(&mut self)`: sets both `counters` and `counters_loading` to `None`.

Tests:
- [ ] `failures_sum_across_days_and_versions` and `successes_alone_are_the_empty_state` exercise `failure_rows` with rows from a real `Counters::open_in_memory` store (via `since`).
- [ ] `the_page_lists_counted_failures_and_an_empty_state` (gpui): a source returning rows renders `diagnostics-counter-0`. An empty source renders `diagnostics-counters-empty`.
- [ ] `leaving_mid_load_drops_it_and_the_next_visit_reloads`: open on Diagnostics, switch to General before parking, then park. `counters` is `None`. Change the source's data, return, and park. The new rows show.
- [ ] `a_store_error_is_shown_not_an_empty_state`: the source returns `Err`, and `diagnostics-counters-error` renders.

### Task 3: Actions through one shared helper

**Files:** Create `crates/gespan-gpui/src/shell/diagnostics.rs` with `impl Shell { pub(super) fn open_logs_folder(&mut self, cx); pub(super) fn copy_diagnostics(&mut self, cx) }`. Add the `mod` line to `shell.rs`. `shell/quick_open.rs` calls these helpers. Add `SettingsEvent::{ReportIssue, OpenLogs, CopyDiagnostics}`, which `shell/settings.rs` handles. ReportIssue calls `open_report(Prefill::default(), ..)`, which already exists and is `pub(super)`, so `report.rs` is untouched.

- [ ] `settings_diagnostics_buttons_copy_open_and_report` (gpui, shell): clicking `diagnostics-copy` puts `diagnostics::summary()` on the clipboard. `diagnostics-open-logs` sets `shell.error` to contain "Diagnostics have not started". `diagnostics-report` leaves `shell.report.open` as `Some`.
- [ ] The existing palette tests (`palette_copies_diagnostics_and_reports_a_missing_logs_folder`, `palette_opens_the_report_sheet`) still pass.

## My calls

- The counters show failures only (successes are hidden). One row per operation and error kind, summed over 7 days. The issue asks for "failure counters".
- The counters reload on every visit, so they are never stale. Nothing caches them across visits.
- Report an issue opens the sheet over Settings, and Settings stays open underneath.
- The page icon is `bug`, an icon the set already has.
