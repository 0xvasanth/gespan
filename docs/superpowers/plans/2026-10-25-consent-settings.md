# Privacy Settings and Consent (Native) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Gespan Native gets a Settings → Privacy & diagnostics page (the
three consent choices, Report an issue / Open logs folder / Copy
diagnostics, the privacy notice link and the last 7 days of failure
counters), a stored consent record that re-prompts on a `consent_version`
bump, a first-run sheet, and revocation that empties the send queue and
deletes the install ID.

**Architecture:** The consent rules are pure code in a new
`gespan-core::diagnostics::consent` (record, answers, changes, which queues
to empty, install ID) and `diagnostics::outbox` (the on-disk send queue and
the consent gate every future send goes through). Native stores the state in
its own `Preferences` (saved with the workspace UI state). The Settings page
lives in `settings/pages/diagnostics.rs`; the first-run sheet in
`shell/consent.rs`. Shared files get one-line registrations only.

**Tech Stack:** Rust, GPUI, serde, chrono, uuid v4; Vitest only for
`legacyNames`.

**Spec:** GitHub issue #74 (approved design), with the contract in
`docs/diagnostics/consent.md` and the user-facing text in `docs/privacy.md`.

## Global Constraints

- `CONSENT_VERSION = 1`. Record fields exactly: `consent_version`, `crash_reports` (`"ask"|"always"|"never"`), `metrics`, `include_logs`, `decided_at` (RFC 3339 UTC ms or `null`).
- Defaults: `{1, "ask", false, true, null}`. Nothing is sent in the default state.
- First-run answers: Always → `always`, Ask → `ask`, No thanks → `never`, Dismissed → `ask`; `metrics` stays `false`, `include_logs` `true`; `decided_at` = now.
- Re-prompt (stored version < current) sets only `crash_reports`, `consent_version`, `decided_at`; keeps stored `metrics` and `include_logs`.
- Prompt is shown when there is no stored record or its version < `CONSENT_VERSION`. It is a small dismissible sheet, never a blocking modal.
- Prompt copy: "Help improve Gespan?" / "Crash reports and failure counts help us fix bugs. They never include your code, prompts or file contents." / "What we collect" / buttons "Send crash reports automatically", "Ask me each time", "No thanks".
- Settings changes take effect at once and set `decided_at`. A kind that no longer sends (crash reports not `always`; metrics off) has its queue emptied. Nothing opted in → install ID deleted. Install ID is a UUID v4 created on first opt-in.
- Settings labels: "Crash reports" (Ask each time / Always send / Never send), "Error metrics" with "(daily failure counts, no content)", "Include recent logs in reports I send".
- Privacy notice link: `https://github.com/0xvasanth/gespan/blob/main/docs/privacy.md`.
- Shared files (`settings/model.rs` `Section`, `settings/mod.rs`, `settings/preferences.rs`, `shell.rs`, `shell/notifications.rs`, `main.rs`, `diagnostics/mod.rs`) get single-line additive edits.
- Gespan names only (`src/test/legacyNames.test.ts`).

## My calls (none changes what #74 builds)

- **Where the record lives.** `DesktopPreferences` is a read-only view of the Tauri app's WebKit storage and must not be written. "Desktop preferences for Gespan Native" is Native's own `Preferences`, saved with the workspace UI state, so the record goes there as `diagnostics: gespan_core::diagnostics::ConsentState { record: Option<Consent>, install_id: Option<String> }`. The rules live in `gespan-core/src/diagnostics/consent.rs` beside the other diagnostics code rather than in `desktop_preferences/`.
- **The queue.** No sender exists yet (#75). The queue is `<log dir>/outbox/crash/` and `<log dir>/outbox/metrics/`, one file per pending send, and `outbox::drain` is the one consent gate a sender may use: it hands a `Transport` only the kinds the record allows. #75 plugs `ureq` into `Transport`. Pending crash records in `crashes/` are not the queue: the contract keeps them (for the notice, or 7 days under "Never send").
- **When a queue is emptied.** Every change empties the queue of each kind the new record does not send in the background, not only on an on→off edge. It is idempotent, so a stale queue from an older run can never be sent. "Always send" → "Ask each time" empties the crash queue too: queued items were queued for automatic sending.
- **Crash notice buttons.** The consent-driven Send / Always send buttons and hiding the notice under "Never send" are crash *sends*, so they stay with #75. Copying the stored record into `report.json` is also left for #75 (follow-up), since the fallback is the only report path today.
- **Counters view** sums failures per `(name, kind)` with "today" and "7 days" columns, most failures first; successes are not listed. Empty state: "No failures in the last 7 days." Counters load off the UI thread when the page opens.
- **When the prompt is checked.** `main` calls `shell.check_consent_prompt(cx)` once at launch, next to `check_crash_notice`, so test shells never show it unless a test asks.

## Review Focus

1. A saved preferences file from before this change (no `diagnostics` key) → loads as "no record", shows the prompt, sends nothing. (Task 3 `old_preferences_load_without_a_record`.)
2. A stored record with an unknown `crash_reports` value or garbage → treated as no record (defaults, prompt again), never as consent. (Task 1 `unreadable_record_is_no_record`.)
3. Re-prompt answer "Send automatically" with metrics previously on → metrics stay on, install ID kept, not recreated. (Task 1 `reprompt_keeps_metrics_and_include_logs`, `install_id_survives_while_opted_in`.)
4. Outbox folder missing, or holding a subfolder / unreadable file → `clear` and `drain` don't fail and don't send it. (Task 2 `clear_missing_outbox_is_ok`, `drain_skips_non_files`.)
5. Revoking from Settings while the outbox has files → files gone without restarting. (Task 4 `never_send_empties_the_crash_queue`.)

---

### Task 1: Consent record and rules (core)

**Files:**
- Create: `crates/gespan-core/src/diagnostics/consent.rs`
- Modify: `crates/gespan-core/src/diagnostics/mod.rs` (one `mod` line, one `pub use` line)

**Interfaces:**
- Produces:
  - `pub const CONSENT_VERSION: u32 = 1;`
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "lowercase")] pub enum CrashReports { Ask, Always, Never }` with `as_str()` and `ALL`.
  - `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] pub struct Consent { consent_version: u32, crash_reports: CrashReports, metrics: bool, include_logs: bool, decided_at: Option<String> }` (pub fields), `Default` = contract defaults, `fn sends(&self, Kind) -> bool`, `fn opted_in(&self) -> bool`.
  - `pub enum Answer { Always, Ask, Never, Dismissed }`.
  - `#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)] #[serde(default)] pub struct ConsentState { record: Option<Consent>, install_id: Option<String> }` with
    `fn current(&self) -> Consent`, `fn needs_prompt(&self) -> bool`,
    `fn answer(&mut self, answer: Answer, now: &str) -> Vec<Kind>`,
    `fn change(&mut self, edit: impl FnOnce(&mut Consent), now: &str) -> Vec<Kind>` — both return the kinds whose queue must be emptied.
  - `record` deserialises with `deserialize_with` that turns an unreadable value into `None`.
  - `pub fn now() -> String` (RFC 3339 UTC, millis, `Z`).
- Consumes: `outbox::Kind` (Task 2; define `Kind { Crash, Metrics }` with `ALL` in `outbox.rs` first).

- [ ] **Step 1: Failing tests** in `consent.rs`: `defaults_send_nothing` (default record equals contract JSON, `sends` false for both kinds, `needs_prompt` true for `ConsentState::default()`), `first_run_answers_match_the_contract` (table of 4 answers → crash_reports, metrics false, include_logs true, version 1, decided_at = now), `reprompt_keeps_metrics_and_include_logs` (stored v0 `{never, metrics true, include_logs false}` → needs_prompt; Always → `{always, true, false, v1}`), `current_record_is_not_prompted`, `unreadable_record_is_no_record` (`{"record":{"crash_reports":"sometimes"}}` and `{"record":3}` → `record == None`), `install_id_created_on_first_opt_in` (Always → UUID v4 string; Ask → None), `install_id_survives_while_opted_in` (metrics on then crash always → same id), `opting_out_of_everything_deletes_the_install_id`, `revoking_returns_the_queues_to_empty` (Always→Never returns `[Crash, Metrics]` minus kinds still sent; metrics on→off returns includes Metrics; change keeps sending kinds out of the list), `record_json_matches_the_contract` (serialised keys/values exactly as consent.md's example).
- [ ] **Step 2:** `cargo test -p gespan-core diagnostics::consent` → fails to compile (no module).
- [ ] **Step 3:** Implement; `change` sets `consent_version = CONSENT_VERSION` only via answers (a Settings edit keeps the stored version, so a pending re-prompt still shows) and always sets `decided_at`.
- [ ] **Step 4:** Same command → PASS.
- [ ] **Step 5:** Commit with Task 2.

### Task 2: Outbox and the send gate (core)

**Files:**
- Create: `crates/gespan-core/src/diagnostics/outbox.rs`
- Modify: `crates/gespan-core/src/diagnostics/mod.rs` (one `mod`, one `pub use`)

**Interfaces:**
- Produces: `pub enum Kind { Crash, Metrics }` (`ALL`, `dir_name()` = `"crash"`/`"metrics"`), `pub fn queue_dir(log_dir: &Path, kind: Kind) -> PathBuf`, `pub fn clear(log_dir: &Path, kinds: &[Kind]) -> std::io::Result<usize>`, `pub trait Transport { fn send(&mut self, kind: Kind, body: &[u8]) -> Result<(), String>; }`, `pub fn drain(log_dir: &Path, consent: &Consent, transport: &mut dyn Transport) -> usize` (sends allowed kinds only, deletes a file after a successful send, returns how many were sent).

- [ ] **Step 1: Failing tests** with a `Recorder` mock transport: `default_consent_makes_no_calls` (files in both queues and in `crashes/` → 0 calls, files untouched), `first_run_answers_other_than_always_make_no_calls` (Ask, Never, Dismissed), `always_sends_only_crash_queue` (one crash, one metrics file → 1 call of `Crash`, crash file removed, metrics file kept), `failed_send_keeps_the_file`, `clear_empties_only_named_queues`, `clear_missing_outbox_is_ok` (returns 0), `drain_skips_non_files` (subdirectory in queue is not sent).
- [ ] **Step 2:** `cargo test -p gespan-core diagnostics::outbox` → FAIL.
- [ ] **Step 3:** Implement; read queue files sorted by name.
- [ ] **Step 4:** PASS, plus `cargo test -p gespan-core diagnostics` still green.
- [ ] **Step 5:** `git commit -m "Add the diagnostics consent record and send queue"`.

### Task 3: Store consent in Native preferences, and the first-run sheet

**Files:**
- Modify: `crates/gespan-gpui/src/settings/preferences.rs` (field `pub diagnostics: gespan_core::diagnostics::ConsentState,` + `diagnostics: Default::default(),`)
- Create: `crates/gespan-gpui/src/shell/consent.rs`
- Modify: `crates/gespan-gpui/src/shell.rs` (`mod consent;`, field `consent: consent::PromptState,`, init `consent: Default::default(),`), `crates/gespan-gpui/src/shell/notifications.rs` (`.children(self.consent_prompt(cx))` in the notice stack), `crates/gespan-gpui/src/main.rs` (`shell.check_consent_prompt(cx);`)

**Interfaces:**
- Consumes: `ConsentState`, `Answer`, `outbox::clear`, `diagnostics::running_log_dir()`.
- Produces: `Shell::check_consent_prompt(&mut self, cx)`; `PromptState { shown: bool, log_dir: Option<PathBuf> }`; `Shell::answer_consent(&mut self, Answer, window, cx)` (updates `workspace.preferences.diagnostics`, saves via `set_preferences`, empties the returned queues on the background executor, hides the sheet). Debug selectors: `consent-prompt`, `consent-always`, `consent-ask`, `consent-never`, `consent-dismiss`, `consent-privacy`.

- [ ] **Step 1: Failing tests**: in `preferences.rs` `old_preferences_load_without_a_record` (`{"idle_agents":2}` → `diagnostics.record == None`, `needs_prompt()`); in `shell/consent.rs` gpui tests `first_run_shows_the_prompt_and_no_thanks_stores_never` (check → `consent-prompt` present; click `consent-never` → record never v1, prompt gone), `dismissing_counts_as_ask_each_time`, `version_bump_shows_the_prompt_again` (stored v0 record with metrics on → prompt; click `consent-always` → always, metrics still on, install id set), `current_record_shows_no_prompt`, `no_thanks_empties_the_crash_queue` (tempdir log_dir with `outbox/crash/a.json` → gone after click).
- [ ] **Step 2:** `cargo test -p gespan-gpui consent` → FAIL.
- [ ] **Step 3:** Implement the sheet as a `toast_frame()` card like the crash notice, with the contract copy; "What we collect" opens the privacy URL.
- [ ] **Step 4:** PASS.
- [ ] **Step 5:** `git commit -m "Ask for diagnostics consent on first run and after a version bump"`.

### Task 4: Settings → Privacy & diagnostics page

**Files:**
- Create: `crates/gespan-gpui/src/settings/pages/diagnostics.rs`
- Modify: `crates/gespan-gpui/src/settings/model.rs` (one line each: variant, `GROUPS` App list after General, `ALL` (13), `id` `"diagnostics"`, `label` `"Privacy & diagnostics"`, `description`, `keywords`, `icon` `"shield"` if present else an existing icon), `settings/pages/mod.rs` (`mod diagnostics;`, `pub(super) use diagnostics::DiagnosticsState;`, match arm), `settings/mod.rs` (field `diagnostics: pages::DiagnosticsState,`, init, and a one-line call `self.load_counters(cx)` on entering the section)

**Interfaces:**
- Consumes: `ConsentState::change`, `outbox::clear`, `metrics::{last_days, Count}`, `diagnostics::{open_logs_folder, summary}`, `crate::report::ReportIssue`.
- Produces: `DiagnosticsState { log_dir: Option<PathBuf>, counters: Option<Result<Vec<Count>, String>> }`; `fn summarize(counts: &[Count], today: NaiveDate) -> Vec<CounterRow>` with `CounterRow { name: String, kind: &'static str, today: u64, week: u64 }`; debug selectors `settings-crash-reports-{ask,always,never}`, `settings-toggle-error-metrics`, `settings-toggle-include-logs`, `settings-report-issue`, `settings-open-logs`, `settings-copy-diagnostics`, `settings-privacy-notice`, `settings-counter-row`, `settings-counters-empty`.

- [ ] **Step 1: Failing tests**: `summarize_totals_failures_per_name_and_kind` (rows across 3 days and 2 versions → summed, today separate, successes dropped, sorted by week desc then name), `summarize_ignores_days_outside_the_week`; gpui (layout_tests-style window on `Section::Diagnostics`): `defaults_show_ask_metrics_off_logs_on`, `never_send_empties_the_crash_queue` (tempdir outbox), `metrics_on_creates_install_id_and_off_deletes_it_and_queue`, `counters_render_rows_and_empty_state`, `copy_diagnostics_writes_the_summary` (clipboard holds `summary()` text); model test `diagnostics_is_in_the_app_group_after_general` and search for "privacy"/"crash" lands on it.
- [ ] **Step 2:** `cargo test -p gespan-gpui settings::` → FAIL.
- [ ] **Step 3:** Implement the page: groups "Sending" (the three rows + privacy link), "Help" (three buttons; Report dispatches `ReportIssue`), "Last 7 days" (counter rows). Every edit goes through `this.update(cx, |p| kinds = p.diagnostics.change(..))`, then `outbox::clear` on the background executor.
- [ ] **Step 4:** PASS.
- [ ] **Step 5:** `git commit -m "Add the Privacy & diagnostics settings page"`.

### Task 5: Verify

- [ ] `bun run native:check` and `bun run check` (one at a time), quote result lines; rerun load-flaky failures before treating them as ours.
- [ ] Commit the plan with the work.
