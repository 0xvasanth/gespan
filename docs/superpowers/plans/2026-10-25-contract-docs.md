# Diagnostics Contract Docs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Write the shared diagnostics contract (log schema, report bundle, consent, intake API, privacy) that the Native and Tauri tickets code against.

**Architecture:** Five Markdown documents under `docs/diagnostics/` and `docs/privacy.md`. Each contract document has a field table (`| Field | Type | Required | Meaning |`) and fenced `json` / `jsonl` examples. A Vitest contract test reads the documents and checks that the examples parse, carry every required field from their own table, agree on shared constants (`consent_version`, `bundle_version`, `app` values, the deny list), and that every relative link resolves. The docs can then change without silently contradicting themselves.

**Tech Stack:** Markdown, Vitest (node environment), Bun.

**Spec:** GitHub issue #66 (approved design). Background: the "Shared contract", Part 4 (Consent) and Part 5 (bundle, intake) sections of epic #45; sibling tickets #67 (deny list), #68 and #77 (log files), #73 (intake), #74 and #82 (consent storage).

## Global Constraints

- Gespan names only. `src/test/legacyNames.test.ts` must pass. Use the app ids `dev.gespan.native` and `dev.gespan.desktop`, the env var `GESPAN_LOG` and the repo `0xvasanth/gespan`.
- Log line: `{ ts, level, target, message, app: "native"|"tauri", version, session_id?, span?, fields{}, event_id }`.
- Bundle: `bundle_version: 1`, zip named `gespan-diagnostics-<app>-<version>-<timestamp>.zip`, containing `report.json`, `logs.jsonl`, `crashes/*.json`, `counters.json` and `environment.json`. Size cap 5 MB.
- Consent: crash reports Ask each time (default) / Always send / Never send; error metrics Off (default) / On; include logs On (default) / Off. Record `{ consent_version, crash_reports, metrics, include_logs, decided_at }`. Dismissing the prompt counts as "Ask each time".
- Deny-listed fields: `prompt`, `content`, `diff`, `output`, `body`, `env`, `audio`, `transcript` (from #67).
- Intake: `POST /v1/reports` returns `{ report_id, issue_url }`; `POST /v1/metrics` takes the daily aggregate. Auth is a per-build public ingest key plus server-side rate limiting. The contact email stays in the private bundle only. R2 lifecycle is 90 days.
- Local logs: 7 days or 50 MB, whichever limit is hit first.
- Docs only: no app code changes. The only non-doc file is the contract test.

## My calls (not fixed by the issue)

- **A contract test.** #66 is docs only, but TDD needs a failing test. `src/test/diagnosticsContract.test.ts` checks that the docs agree with themselves and with each other. It is not filler: the Native and Tauri tickets will copy these examples, so a broken example or a mismatched constant is a real defect.
- **Tauri log file name:** `gespan-tauri.YYYY-MM-DD.jsonl`, mirroring #68's `gespan-native.YYYY-MM-DD.jsonl`.
- **Ingest key header:** `X-Gespan-Ingest-Key`. Error body: `{ "error": { "code", "message" } }`.
- **`crash_reports` values:** `"ask" | "always" | "never"`. `metrics` and `include_logs` are booleans. `decided_at` is RFC 3339 UTC, or `null` until the user decides.
- **Retention values the epic leaves open:** local counters 30 days; server-side metric aggregates 13 months; error tracker 90 days, to match R2. These are flagged for the owner to confirm in review.
- **Deletion requests:** a GitHub issue quoting the `report_id`, which does not identify the person. No private contact address exists yet. This is flagged for the owner.
- **No `scrubbing.md`:** it is not in #66's scope. #67 owns the rules and the fixtures, and these docs link to `crates/gespan-redact` by name only.

## Review Focus

- A JSONL example line that is valid JSON but missing a required field: the test checks every example line against the table.
- An example that puts a deny-listed key (for example `prompt`) inside `fields`: the test rejects it.
- `consent_version` written as 1 in one doc and 2 in another: the test checks they agree.
- A relative link that goes nowhere (for example `../privacy.md` from the wrong depth): the test resolves every relative link.
- First-run prompt wording changed in only one place: privacy.md must quote consent.md's prompt heading exactly, and the test checks it.

---

### Task 1: Contract test plus log schema

**Files:**
- Create: `src/test/diagnosticsContract.test.ts`
- Create: `docs/diagnostics/log-schema.md`

**Interfaces:**
- Produces, as test helpers in the test file: `doc(path): string`; `jsonBlocks(md): unknown[]` (each ```` ```json ```` block parsed whole, each ```` ```jsonl ```` line parsed separately); `requiredFields(md, heading): string[]` (rows of the table under `heading` whose Required column is `yes`, with backticks stripped from the first column).
- Doc conventions that later tasks follow: the field table sits under a `## Fields` heading (or a named heading passed to `requiredFields`), and examples use the `json` / `jsonl` fence languages.

- [ ] **Step 1:** Write the tests under `describe("log schema")`:
  - every `jsonl` example line parses and includes each required field: `ts`, `level`, `target`, `message`, `app`, `version`, `fields`, `event_id`;
  - `app` is `native` or `tauri`, and both appear across the examples;
  - `level` is one of `error`, `warn`, `info`, `debug`, `trace`;
  - `ts` matches RFC 3339 UTC with milliseconds;
  - no `fields` object contains a deny-listed key;
  - the doc's deny-list section lists exactly the 8 names.
- [ ] **Step 2:** Run `bunx vitest run src/test/diagnosticsContract.test.ts`. Expected: FAIL (ENOENT on `log-schema.md`).
- [ ] **Step 3:** Write `log-schema.md`. Cover the file locations per OS and app (dev is a separate folder), rotation, levels and `GESPAN_LOG`, the field table, the deny list, the `event_id` format (UUID v4), how a Tauri `diagnostics_log` record maps to a line, and examples.
- [ ] **Step 4:** Run again. Expected: PASS.
- [ ] **Step 5:** Commit.

### Task 2: Bundle format

**Files:** Create `docs/diagnostics/bundle-format.md`; Modify `src/test/diagnosticsContract.test.ts`

- [ ] **Step 1:** Write the tests:
  - the `report.json` example has every required field from `## report.json` and `bundle_version === 1`;
  - its `app` is `native` or `tauri`;
  - its `consent.consent_version` equals the one in consent.md (that half is added in Task 3);
  - `contact_email` is documented as never published;
  - the layout lists all 5 entries;
  - the `counters.json` and `environment.json` examples parse and include their required fields;
  - `environment.json` has no key containing `path`, `name` or `url`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Write the doc: layout, zip naming, the 5 MB cap, a schema plus an example for each file, crash record fields (with room for a minidump), and versioning rules.
- [ ] **Step 4:** Run. Expected: PASS. **Step 5:** Commit.

### Task 3: Consent

**Files:** Create `docs/diagnostics/consent.md`; Modify the test.

- [ ] **Step 1:** Write the tests:
  - the consent record example has the 5 fields;
  - the defaults example is `crash_reports: "ask"`, `metrics: false`, `include_logs: true`, `decided_at: null`;
  - `consent_version` is the same in consent.md, bundle-format.md and privacy.md (privacy is added in Task 5);
  - the first-run prompt has 3 buttons, and dismissing it maps to "ask".
- [ ] **Step 2:** Run. FAIL. **Step 3:** Write the doc: the choices, defaults, record, version-bump rule, revocation, install ID, prompt wording, and the change log of versions. **Step 4:** PASS. **Step 5:** Commit.

### Task 4: Intake API

**Files:** Create `docs/diagnostics/intake-api.md`; Modify the test.

- [ ] **Step 1:** Write the tests:
  - both `POST /v1/reports` and `POST /v1/metrics` appear as headings;
  - the reports success response has `report_id` and `issue_url`;
  - the metrics request example has the required fields, and each counter's `error_kind` matches `^[a-z_]+$`;
  - the error example has `error.code` and `error.message`;
  - the status table includes 400, 401, 413, 429 and 5xx.
- [ ] **Step 2:** FAIL. **Step 3:** Write the doc: base URL from the build config, the ingest key header, multipart shape (`bundle` zip + `meta` JSON), limits, responses, idempotency, and what the server does with the data. **Step 4:** PASS. **Step 5:** Commit.

### Task 5: Privacy

**Files:** Create `docs/privacy.md`; Modify the test.

- [ ] **Step 1:** Write the tests:
  - privacy.md has the sections `What is collected`, `Where it goes`, `How long it is kept` and `How to have it deleted`;
  - it states `consent_version`;
  - it lists every deny-listed field as never sent;
  - every relative link in all five docs resolves to an existing file.
- [ ] **Step 2:** FAIL. **Step 3:** Write the doc. **Step 4:** PASS. **Step 5:** Commit.

### Task 6: Verify

- [ ] Run `bun run check` and confirm that `legacyNames.test.ts` and the new test pass.
