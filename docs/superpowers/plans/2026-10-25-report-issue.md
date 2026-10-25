# Report an Issue (Native) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Gespan Native gets a "Report an issue" sheet that builds the
diagnostics bundle from `docs/diagnostics/bundle-format.md`, shows exactly the
bytes it will save, and (with no intake yet) saves the zip to Downloads,
reveals it and opens a prefilled GitHub issue form.

**Architecture:** `gespan-core::diagnostics` gains three files: `bundle.rs`
(pure: scrubbed sources + the user's fields → entries + zip bytes, with the
5 MB cap), `collect.rs` (reads the log tail, crash files, provider CLIs and
system facts, and scrubs them once with `gespan-redact`) and `fallback.rs`
(the issue URL, saving to Downloads). `json.rs` remembers recent `error`
event ids for the toast's "Report". `gespan-gpui` gets a `report.rs` sheet
entity and `shell/report.rs` for the entry points. The preview renders
`Bundle::entries`; saving writes `Bundle::zip()` built from those same
entries, so the two cannot drift.

**Tech Stack:** Rust, GPUI, `gespan-redact`, `zip` 4 (deflate), `url`
form encoding, Vitest only for `legacyNames`.

**Spec:** GitHub issue #72 (approved design), with the contract in
`docs/diagnostics/bundle-format.md`, `consent.md`, `intake-api.md`
(fallback section) and `log-schema.md`.

## Global Constraints

- Zip name `gespan-diagnostics-native-<version>-<YYYYMMDDTHHMMSSZ>.zip`; entries at the root, UTF-8, `\n` endings.
- `report.json` always present; with "Attach diagnostics" unticked the zip holds only `report.json`.
- `logs.jsonl`: last 2,000 lines or 24 hours, whichever is smaller.
- Whole zip ≤ 5 MB (5 × 1024 × 1024); drop oldest log lines first, then oldest crash records; never `report.json`.
- `bundle_version: 1`, `kind: "user"`, `app: "native"`, `submission_id` UUID v4, `created_at` RFC 3339 UTC ms, `files` lists the other entries.
- Fallback bundle: no `contact_email`, no `install_id`; the sheet hides the contact email field in fallback mode.
- URL: `https://github.com/0xvasanth/gespan/issues/new?template=bug_report.yml&title=…&version=…&os=…&os-version=…&what-happened=…`; `os` mapped `macos`→`macOS`, `linux`→`Linux`, `windows`→`Windows`.
- Sheet must say the fallback needs a GitHub account and the attached zip will be public.
- Everything is scrubbed by `gespan-redact` before it is shown.
- Gespan names only (`src/test/legacyNames.test.ts`). Shared files (`diagnostics/mod.rs`, `main.rs` mod list, Cargo files) get small additive edits; lock files regenerate via cargo.

## My calls (no question changed what gets built)

- **Contact email.** #72 asks for the field; the contract hides it in fallback mode, and fallback is the only mode until #75. The sheet has the field behind `Destination::Intake`; today it is hidden, and `bundle.rs` tests cover both destinations.
- **Consent and install id.** #74 hasn't landed, so `report.json.consent` is the contract's default record and `install_id` is omitted (fallback omits it anyway). `include_logs` (default `true`) sets the logs entry's starting state.
- **Counters.** #71 hasn't landed. `Sources::counters` is an `Option<Value>` the app passes as `None`; `counters.json` appears once #71 wires its store in (follow-up).
- **Logs are scrubbed as one text block** with `Redactor::redact_text`, the way the crate's log-tail fixtures expect (cut first lines, multi-line values). Crash files, environment and counters go through `redact_value`. The user's title, description and steps go through `redact_text`; the contact email is not scrubbed (it is the user's own, and private).
- **Crash notice** offers "Report…" (opens the sheet, a `user` report with the crash records attached, title `Crash in <file>:<line>`) and "Don't send" (deletes the records). The consent-driven Send / Always send buttons are `crash` sends and belong to #74/#75. After the fallback saves a zip, the crash records it carried are deleted locally: the user now holds them, and otherwise the notice returns every launch.
- **Toast event ids.** The JSON layer keeps the last 32 `error` lines' `(Instant, event_id)`. When an app error toast first appears, the shell snapshots the ids logged in the 10 s before it; "Report" on that toast passes them as `event_ids` and prefills the title with the toast text.
- **Help menu** is added to the existing app menu bar (`file_editor::menus`), as `Help → Report an issue…`.
- **Removing things from the preview** is per entry (logs, each crash record, environment, counters), not per line.
- **Long URLs.** GitHub rejects very long URLs; the description, then the steps, are cut so the URL stays ≤ 8,000 bytes, ending in `… (cut; see the attached zip)`. The zip still has the full text.
- **Locale** from `LC_ALL`/`LC_MESSAGES`/`LANG`, else macOS `AppleLocale`, as BCP 47 (`en_GB.UTF-8` → `en-GB`); `und` when unknown. **OS version** from `sw_vers -productVersion` / `/etc/os-release` `VERSION_ID` / `cmd /c ver`; `unknown` on failure.
- **Build channel:** `Channel::Dev` → `dev`; release data with `-preview` in the version → `preview`; else `release`.

## Review Focus

1. A Downloads folder that already has a zip with the same name → save as `…-1.zip`, never overwrite. (Task 3 test `save_never_overwrites`.)
2. A log file with non-JSON lines, or lines with no or bad `ts` (a panic's raw text) → kept in the tail, not dropped as "old". (Task 2 test `tail_keeps_lines_without_a_timestamp`.)
3. Title/description containing a home path, token or email → scrubbed in `report.json` *and* in the URL. (Task 1 `user_text_is_scrubbed`, Task 3 `url_uses_scrubbed_report_fields`.)
4. Non-ASCII and `&`/`#`/newlines in user text → URL round-trips through form decoding exactly. (Task 3 `url_round_trips_special_characters`.)
5. A crash file that isn't valid JSON → included as scrubbed text, not a failed bundle. (Task 2 `unparsable_crash_is_scrubbed_as_text`.)

---

### Task 1: Bundle assembly (`bundle.rs`)

**Files:**
- Modify: `crates/gespan-core/Cargo.toml` (add `gespan-redact = { path = "../gespan-redact" }`, `zip = { version = "4", default-features = false, features = ["deflate-flate2"] }`; dev-dep `gespan-redact` with `features = ["fixtures"]`), `crates/gespan-gpui/Cargo.lock` (regenerated).
- Create: `crates/gespan-core/src/diagnostics/bundle.rs`
- Modify: `crates/gespan-core/src/diagnostics/mod.rs` (one `mod`, one `pub use`)

**Interfaces — Produces:**
```rust
pub const MAX_BUNDLE_BYTES: usize = 5 * 1024 * 1024;
pub enum Destination { Intake, Fallback }
pub struct Report { pub title: String, pub description: String, pub steps: String,
                    pub contact_email: Option<String>, pub event_ids: Vec<String> }
pub struct System { pub version: String, pub build_channel: &'static str, pub os: String,
                    pub os_version: String, pub arch: String, pub locale: String }
pub struct Sources { pub logs: String /* scrubbed, oldest first, '\n'-joined */,
                     pub crashes: Vec<(String /*file name*/, String /*scrubbed body*/)>,
                     pub counters: Option<String>, pub environment: Option<String>,
                     pub consent: serde_json::Value, pub install_id: Option<String> }
pub struct Selection { pub attach: bool, pub logs: bool, pub crashes_excluded: BTreeSet<String>,
                       pub counters: bool, pub environment: bool }
pub struct Entry { pub name: String, pub bytes: Vec<u8> }
pub struct Bundle { pub file_name: String, pub entries: Vec<Entry>, /* private zip */ }
impl Bundle { pub fn zip(&self) -> &[u8]; pub fn report(&self) -> serde_json::Value; pub fn entry(&self, name) -> Option<&Entry> }
pub struct Stamp { pub submission_id: String, pub created_at: DateTime<Utc> }
pub fn build(report: &Report, system: &System, sources: &Sources, selection: &Selection,
             destination: Destination, redactor: &Redactor, stamp: &Stamp, cap: usize) -> Result<Bundle, String>;
pub fn default_consent() -> serde_json::Value;
```
Crash entries are named `crashes/<file name>`. Zip entries use a fixed
modification time (the `created_at` instant), deflate, in `entries` order, so
the same input gives the same bytes.

- [ ] **Step 1: Write failing tests** in `bundle.rs`:
  - `zip_holds_exactly_the_preview_entries`: unzip `bundle.zip()` → same names, order and bytes as `bundle.entries`; `report.json` first; `files == ["logs.jsonl","crashes/a.json","environment.json"]`.
  - `unticked_attach_sends_only_report_json`: `entries.len() == 1`, `files == []`.
  - `fallback_leaves_out_contact_email_and_install_id` / `intake_keeps_contact_email_unscrubbed`.
  - `report_json_matches_the_contract`: `bundle_version == 1`, `kind == "user"`, `app == "native"`, `created_at == "2026-10-25T09:14:03.000Z"`, `steps` omitted when empty, `event_ids` present when empty, file name `gespan-diagnostics-native-0.1.0-20261025T091403Z.zip`.
  - `user_text_is_scrubbed`: title `"crash in /Users/alice/x"` with home `/Users/alice` → `"crash in ~/x"`; description with `ghp_…` → `<redacted>`.
  - `excluded_entries_are_left_out`: logs off, one crash excluded.
  - `over_the_cap_drops_oldest_logs_then_oldest_crashes`: small `cap`; newest log line survives, oldest gone; then with incompressible crash bodies the older crash goes first; `report.json` stays even when nothing else fits.
  - `same_input_same_bytes`.
- [ ] **Step 2:** `cd crates/gespan-gpui && cargo test -p gespan-core diagnostics::bundle` → fails to compile.
- [ ] **Step 3:** Implement. Cap loop: rebuild, and while `zip.len() > cap` drop ⌈10 %⌉ (≥1) of the remaining log lines from the front; when logs are empty, drop the entry, then the oldest remaining crash.
- [ ] **Step 4:** Tests pass.
- [ ] **Step 5:** Commit `Build the diagnostics bundle and its zip from one list of entries`.

### Task 2: Collecting and scrubbing sources (`collect.rs`)

**Files:** Create `crates/gespan-core/src/diagnostics/collect.rs`; modify `mod.rs` (one `mod`, one `pub use`). Test fixtures via `gespan_redact::fixtures`.

**Interfaces — Consumes:** Task 1 types. **Produces:**
```rust
pub const TAIL_LINES: usize = 2_000;
pub fn log_tail(log_dir: &Path, now: DateTime<Utc>, max_lines: usize) -> String; // raw, oldest first
pub fn collect(log_dir: &Path, redactor: &Redactor, now: DateTime<Utc>,
               environment: Option<Environment>, counters: Option<Value>) -> Sources;
pub struct Environment { pub providers: Vec<(String, Option<String>)>, pub projects_count: usize, pub sessions_count: usize }
pub fn installed_providers() -> Vec<(String, Option<String>)>; // Kind::ALL resolved, `--version` parsed, in parallel
pub fn system(version: &str, channel: Channel) -> System;
pub fn redactor_for_this_machine(projects: Vec<PathBuf>) -> Redactor; // home from HOME/USERPROFILE
pub fn running_log_dir() -> Option<PathBuf>; // from STARTED
pub fn locale_from(env: impl Fn(&str) -> Option<String>, apple: Option<String>) -> String;
```

- [ ] **Step 1: Failing tests:**
  - `tail_takes_the_last_lines_across_days` (yesterday + today files, `max_lines` 3 → last 3, oldest first).
  - `tail_drops_lines_older_than_24_hours` (today's file has a line `ts` 25 h old → dropped; files older than yesterday never read).
  - `tail_keeps_lines_without_a_timestamp`.
  - `every_redaction_fixture_passes_through_the_bundle`: for each fixture case, write a log dir for its context: text cases as the day's log file, json cases as `crashes/case.json`; `collect` → `build` → unzip; `logs.jsonl` == `expected + "\n"`; crash entry parses to `expected`.
  - `unparsable_crash_is_scrubbed_as_text`.
  - `environment_json_has_no_paths` (shape `{providers:[{id,version}], projects_count, sessions_count}`).
  - `locale_from` cases: `en_GB.UTF-8`→`en-GB`, `C`/`POSIX`→fall through, apple `en_GB`→`en-GB`, none→`und`.
- [ ] **Step 2:** run → fail. **Step 3:** implement. **Step 4:** pass.
- [ ] **Step 5:** Commit `Collect and scrub the log tail, crash records and environment for a report`.

### Task 3: GitHub prefill fallback (`fallback.rs`)

**Files:** Create `crates/gespan-core/src/diagnostics/fallback.rs`; `mod.rs` additive.

**Interfaces — Produces:**
```rust
pub const ISSUE_FORM: &str = "https://github.com/0xvasanth/gespan/issues/new";
pub const MAX_URL_BYTES: usize = 8_000;
pub fn issue_url(report: &serde_json::Value) -> String;   // from the bundle's own report.json
pub fn downloads_dir() -> Option<PathBuf>;
pub fn save_bundle(bundle: &Bundle, dir: &Path) -> io::Result<PathBuf>; // create_new, -1, -2…
pub fn remove_crashes(log_dir: &Path, names: &[String]);  // names as in the zip, `crashes/x.json`
```
Query order: `template=bug_report.yml`, `title`, `version`, `os`, `os-version`, `what-happened`, then `repro` when `steps` is present.

- [ ] **Step 1: Failing tests:** `url_fills_every_field` (parse back with `url::Url::query_pairs`), `os_takes_the_form_label` (all three), `url_uses_scrubbed_report_fields` (built from a real `build` output), `url_round_trips_special_characters`, `long_text_is_cut_to_fit`, `save_never_overwrites`, `saved_bytes_are_the_bundle_zip`, `remove_crashes_only_touches_named_files`.
- [ ] **Step 2–4:** fail, implement, pass.
- [ ] **Step 5:** Commit `Prefill the GitHub bug form and save the bundle to Downloads`.

### Task 4: Recent error event ids (`json.rs`)

**Files:** Modify `crates/gespan-core/src/diagnostics/json.rs` (record in `on_event`), `mod.rs` (`pub use`).

**Produces:** `pub fn recent_error_events(within: Duration) -> Vec<String>` (oldest first, at most 32 kept); `pub fn note_error_event(event_id: String)`.

- [ ] **Step 1:** Failing test `error_lines_are_remembered_for_reports`: capture an `error!` and an `info!`; the error's `event_id` is in `recent_error_events(10 s)`, the info's isn't.
- [ ] **Step 2–4.** **Step 5:** Commit `Remember recent error event ids for the Report action`.

### Task 5: The report sheet (`report.rs`)

**Files:** Create `crates/gespan-gpui/src/report.rs`; modify `crates/gespan-gpui/src/main.rs` (one `mod report;`).

**Interfaces — Produces:**
```rust
actions!(report, [ReportIssue]);
pub(crate) struct Prefill { pub title: String, pub event_ids: Vec<String> }
pub(crate) struct ReportSheet { … }
pub(crate) enum ReportEvent { Close }
impl ReportSheet {
  pub fn new(prefill: Prefill, context: SheetContext, cx) -> Self; // starts collecting in the background
  pub fn bundle(&self) -> Option<&Bundle>;
  pub fn preview_text(&self, name: &str) -> Option<String>;
  pub fn submit(&mut self, cx);            // validates, saves, reveals, opens URL
}
pub(crate) struct SheetContext { pub log_dir: Option<PathBuf>, pub projects: Vec<String>,
  pub sessions_count: usize, pub version: String, pub channel: Channel,
  pub downloads: Option<PathBuf>, pub destination: Destination }
```
Layout: title (required), "What happened?" (required), "Steps to reproduce"
(optional), contact email (only `Destination::Intake`), "Attach diagnostics
(recommended)" checkbox, then the preview: one row per entry with an include
checkbox (none on `report.json`), name, size, and a "Show" toggle that opens
the entry's text in a scrolling monospace `uniform_list` of its lines. The
public warning, verbatim:

> No report service is connected yet. Saving puts the zip in your Downloads
> folder and opens a new GitHub issue in your browser. You need a GitHub
> account, and the zip you attach there will be public.

Primary button: "Save zip and open GitHub". After success the sheet shows
`Saved <file> to Downloads. Drag it into the GitHub issue that just opened.`
with "Done"; on failure an alert with the error and a "Copy report" button
(copies `report.json`).

- [ ] **Step 1: Failing gpui tests:**
  - `preview_text_is_the_zip_bytes`: set fields, wait for collect; for every entry `preview_text(name).as_bytes() == unzip(bundle.zip())[name]`.
  - `typing_updates_report_json_in_the_preview`.
  - `unticking_attach_leaves_only_report_json`, `removing_an_entry_drops_it_from_files`.
  - `title_and_description_are_required` (no save, inline error).
  - `contact_email_is_hidden_in_fallback`.
  - `submit_saves_the_previewed_zip_and_opens_the_prefilled_url` (temp Downloads; saved bytes == `bundle.zip()`; `cx.opened_url()` has `title=` and `what-happened=`).
- [ ] **Step 2–4.** **Step 5:** Commit `Add the Report an issue sheet with an exact bundle preview`.

### Task 6: Entry points (`shell/report.rs`)

**Files:** Create `crates/gespan-gpui/src/shell/report.rs`; modify `shell.rs` (field, `mod`, one `on_action`, one overlay child), `quick_open.rs` + `shell/quick_open.rs` (command), `file_editor.rs` (`Help` menu), `shell/notifications.rs` (Report button on the app error card, crash notice card).

**Produces:** `Shell::open_report(prefill, window, cx)`, `Shell::show_crash_notice(crashes: Vec<PendingCrash>, cx)`, `diagnostics::pending_crashes(log_dir) -> Vec<PendingCrash { name, title }>` (core, in `collect.rs`).

- [ ] **Step 1: Failing tests:**
  - `palette_opens_the_report_sheet` (`>Report an issue`).
  - `report_issue_action_opens_the_sheet` (dispatch `ReportIssue`).
  - `error_toast_report_prefills_title_and_event_ids` (note an event id, show `shell.error`, click `report-error`).
  - `crash_notice_report_prefills_crash_title`, `crash_notice_dont_send_deletes_records`.
  - core: `pending_crashes_titles_from_location` (`"src/a.rs:12:5"` string and `{file,line}` object → `Crash in src/a.rs:12`).
- [ ] **Step 2–4.** **Step 5:** Commit `Open Report an issue from the Help menu, palette, error toasts and the crash notice`.

### Final

- [ ] `bun run native:check`, `bun run check` — quote the result lines.
- [ ] Rebase on `origin/diag/68-native-tracing`, push, PR to `diag/68-native-tracing`.
