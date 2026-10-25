# `BoardProvider` Trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `BoardProvider` trait in `gespan-core` that the external-board work (#98, #99, #100, #103) builds on, with the existing Linear and Jira clients moved under it, the Inbox still working through them, and an in-memory fake for sync tests.

**Architecture:** A new `provider_boards` module holds the trait, its data model and a typed error. The Linear and Jira HTTP/GraphQL clients (transport, queries, parsers, validation) move from `inbox/` to `provider_boards/` as `LinearProvider` and `JiraProvider`. Each keeps inherent methods with the Inbox's exact shapes and also implements the trait. `inbox/linear.rs` and `inbox/jira.rs` keep only what is Inbox-specific (credential files and the `linear_*` / `jira_*` entry points), and delegate to the providers. `FakeProvider` is an in-memory implementation for sync tests.

**Tech Stack:** Rust 2021, `ureq` 2 (blocking), `serde_json`, existing `gespan-core` crate. No new dependencies.

**Spec:** GitHub issue #97 (approved design). Background: "Part 2: The provider layer" in epic #49. Working rules: #60.

## Global Constraints

- New code lives in `crates/gespan-core/src/provider_boards.rs` and `crates/gespan-core/src/provider_boards/**`. The only edits outside are `crates/gespan-core/src/inbox/**` and one `pub mod provider_boards;` line in `crates/gespan-core/src/lib.rs`.
- The trait covers: connect, whoami, list containers, states, labels and members, list and get issues, create, update, move, comment, `sync_since`, `parse_webhook`.
- Write operations (create, update, move) for Linear and Jira are stubs for #98 and #99.
- The existing Inbox tests (`inbox.rs`, `inbox/media.rs`) pass unchanged. The Inbox's requests, error strings, credential file names (`linear-token`, `jira-config.json`) and serialized shapes do not change.
- `src-tauri/` is untouched: it has its own copies of the clients and does not depend on `gespan-core`.
- Gates: `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-core provider_boards`, `bun run check`, `bun run native:check`.
- Gespan names only (`src/test/legacyNames.test.ts`). Commits carry no tool or AI attribution.

## Design calls (mine, not in the issue)

- **Module name `provider_boards`.** `crate::provider` already means agent CLIs. The issue names this path.
- **A trait, not an enum.** Plane (#100) and the fake both need to plug in. The sync engine (#103) can take `&dyn BoardProvider`.
- **Blocking and `&self`.** Core's HTTP is blocking `ureq`, run on workers, so the trait is synchronous. The trait is `Send + Sync` so a worker thread can own a provider.
- **Stubs return an error, not `unimplemented!()`.** A panic in a desktop app's worker is worse than an error. Stubbed methods return `ProviderError::NotSupported { provider, operation }`, which is also easy to test. #98, #99 and #100 replace them.
- **What is stubbed for Linear and Jira.** `create_issue`, `update_issue` and `move_issue` are stubs (the issue says so). `sync_since` is also a stub, because delta sync is in #98 ("delta sync") and #99 ("JQL delta sync"). `parse_webhook` is also a stub, because #49 says to check payloads against live instances before coding parsers, and those fixtures come from #109.
- **What is real for Linear and Jira.** connect, whoami, list containers, states, labels, members, list issues (paged), get issue and comment. Comment is a write, but the clients already post comments for the Inbox, so the trait reuses that code.
- **`connect` is a provided method** that calls `whoami`. A provider is built from credentials, so connecting means proving they work and learning who we are. Storing credentials stays with the caller (the Inbox today, the keychain after #102).
- **States map to a fixed `StateGroup`.** Linear `type`: `triage`, `backlog`, `unstarted`, `started`, `completed`, `canceled` map one to one. Jira `statusCategory.key`: `new` → `Unstarted`, `indeterminate` → `Started`, `done` → `Completed`. Anything else → `Unknown`. This is the column the status-mapping table in #49 keys on.
- **Jira labels have no IDs.** `BoardLabel.id` is the label name.
- **Paging.** Linear labels and members and Jira labels and projects follow every page, so a big team never silently loses rows. `list_issues` returns one page plus an opaque cursor (Linear `endCursor`, Jira `nextPageToken`). The Inbox query only gains `pageInfo` and an optional `$after`, so its behaviour is the same.
- **The fake is `#[cfg(test)] pub(crate)`.** The sync engine (#103) lives in `gespan-core`, so crate tests can reach it, and release builds don't ship it.
- **The move is pinned by tests written at the new path.** Moved parsers get characterization tests written against `provider_boards::…` before the code moves. They fail to compile first and pass once the code has moved.

## Review Focus

1. Two issues updated in the same second straddling a page boundary in `sync_since`: neither may be skipped or repeated. Fake test `sync_since_breaks_timestamp_ties_by_id` (Task 2).
2. A Jira project-statuses response that repeats a status in several issue types: the status is listed once. Test `jira_states_are_deduplicated_across_issue_types` (Task 5).
3. Linear label or member lists longer than one page: all rows returned, and a page that claims more but gives no cursor stops with an error instead of looping. Test `linear_paging_follows_end_cursor_and_rejects_stuck_pages` (Task 4).
4. A `move_issue` to a state from another container, or without fields the target state requires: rejected with `InvalidInput`, issue unchanged. Fake test `move_rejects_foreign_state_and_missing_required_fields` (Task 2).
5. The Inbox with stored credentials but bad input (empty comment, malformed issue key, non-email Jira login): the same error strings as before, with no network call. Tests in Tasks 3 and 5.

---

### Task 1: Model, error and trait

**Files:**
- Create: `crates/gespan-core/src/provider_boards.rs`, `crates/gespan-core/src/provider_boards/model.rs`
- Modify: `crates/gespan-core/src/lib.rs` (add `pub mod provider_boards;`)

**Interfaces:**
- Produces (all `pub`, re-exported from `provider_boards`):
  - `enum ProviderKind { Linear, Jira, Fake }` with `as_str()` → `"linear"`, `"jira"`, `"fake"`. (`Fake` is `#[cfg(test)]`.)
  - `enum StateGroup { Triage, Backlog, Unstarted, Started, Completed, Cancelled, Unknown }` with `from_linear_type(&str)` and `from_jira_category(&str)`
  - `struct Account { id, name, email: String }`, `struct Container { id, key, name: String }`, `struct WorkflowState { id, name: String, group: StateGroup }`, `struct BoardLabel { id, name, color: String }`, `struct Member { id, name, avatar_url: String }`
  - `struct IssueFilter { container_ids: Vec<String>, assigned_to_me: bool, include_closed: bool, limit: u32 }`
  - `struct Page<T> { items: Vec<T>, next_cursor: Option<String> }`
  - `struct RemoteIssue { id, key, title, url, container_id, updated_at: String, state: WorkflowState, labels: Vec<BoardLabel>, assignees: Vec<Member>, description: Option<String> }`
  - `struct IssueDraft { title: String, description: String, state_id: Option<String>, label_ids: Vec<String>, assignee_ids: Vec<String> }`
  - `struct IssuePatch { title, description: Option<String>, label_ids, assignee_ids: Option<Vec<String>> }`
  - `struct MoveTarget { state_id: String, required_fields: BTreeMap<String, serde_json::Value> }`
  - `struct CommentRef { id, url: String }`
  - `struct SyncCursor(pub String)`, `struct SyncBatch { changes: Vec<RemoteIssue>, next_cursor: SyncCursor, has_more: bool }`
  - `enum ChangeEvent { Created, Updated, Deleted }`, `struct ChangeHint { entity: String, id: String, event: ChangeEvent }`
  - `enum ProviderError { NotSupported { provider: ProviderKind, operation: &'static str }, InvalidInput(String), Remote(String) }` with `Display` (`NotSupported` → `"{Provider label} does not support {operation} yet"`; the others print their message), `std::error::Error`, `From<String>` → `Remote`.
  - `type ProviderResult<T> = Result<T, ProviderError>`
  - `trait BoardProvider: Send + Sync` with `kind`, `whoami`, provided `connect` (calls `whoami`), `list_containers`, `list_states(container_id)`, `list_labels(container_id)`, `list_members(container_id)`, `list_issues(&IssueFilter, cursor: Option<&str>) -> Page<RemoteIssue>`, `get_issue(id)`, `create_issue(container_id, &IssueDraft)`, `update_issue(id, &IssuePatch)`, `move_issue(id, &MoveTarget)`, all returning `RemoteIssue`; `comment(issue_id, body, reply_to: Option<&str>) -> CommentRef`; `sync_since(container_id, Option<&SyncCursor>) -> SyncBatch`; `parse_webhook(headers: &[(String, String)], body: &[u8]) -> Vec<ChangeHint>`.

- [ ] **Step 1: Write failing tests in `model.rs`:** `linear_state_types_map_to_groups` (all six types including `canceled` → `Cancelled`, `"bogus"` → `Unknown`, case-insensitive), `jira_categories_map_to_groups` (`new`, `indeterminate`, `done`, `undefined` → `Unknown`), `not_supported_error_names_provider_and_operation` (`"Linear does not support create_issue yet"`), `string_errors_become_remote` (`ProviderError::from("x".to_string())` is `Remote("x")` and prints `x`).
- [ ] **Step 2: Run** `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-core provider_boards` — FAIL (module missing).
- [ ] **Step 3: Implement** the types and trait above. Data types derive `Clone, Debug, Default` where possible, `PartialEq, Eq`, and `Serialize, Deserialize` with `rename_all = "camelCase"` (the sync engine will store them).
- [ ] **Step 4: Run** the same command — PASS.
- [ ] **Step 5: Commit** `Add the BoardProvider trait and its data model`.

### Task 2: `FakeProvider` for sync tests

**Files:**
- Create: `crates/gespan-core/src/provider_boards/fake.rs` (`#[cfg(test)] pub(crate) mod fake;`)

**Interfaces:**
- Consumes: Task 1.
- Produces: `FakeProvider::new() -> Self` (one container `C1` with states `todo` (Unstarted), `doing` (Started), `done` (Completed)); builders `with_container(Container, Vec<WorkflowState>)`, `with_issue(RemoteIssue)`, `with_labels(container, Vec<BoardLabel>)`, `with_members(container, Vec<Member>)`, `require_fields(state_id, &[&str])`; `fail_next(operation: &'static str, ProviderError)`; `comments(issue_id) -> Vec<(CommentRef, String)>`; `now() -> String`. All state behind a `Mutex`. Every write sets `updated_at` from a logical clock that advances one second per write (RFC 3339, base `2026-01-01T00:00:00Z`).

Behaviour:
- `list_issues`: filters by `container_ids` (empty = all), drops `Completed`/`Cancelled` unless `include_closed`, orders by `updated_at` descending then `id`, pages by `limit` with the cursor as the decimal offset. A cursor that is not a number → `InvalidInput`.
- `sync_since`: issues in the container with `(updated_at, id)` strictly after the cursor, ascending, at most 50 per batch; `next_cursor` encodes the last `(updated_at, id)` as `"{updated_at}|{id}"` (unchanged when the batch is empty); `has_more` when more remain.
- `create_issue`/`update_issue`/`move_issue`/`comment`: mutate and return the new value. Unknown issue or container → `InvalidInput`. `move_issue` to a state not in the issue's container, or missing a required field → `InvalidInput`, issue unchanged. `comment` with a `reply_to` that doesn't exist → `InvalidInput`; empty body → `InvalidInput`.
- `parse_webhook`: body is a JSON object or array of `{ "entity", "id", "event": "created" | "updated" | "deleted" }`; invalid JSON or an unknown event → `InvalidInput`.
- `fail_next(op, err)`: the next call to `op` returns `err`, then behaviour returns to normal.

- [ ] **Step 1: Write failing tests:** `connect_returns_the_fake_account`, `list_issues_filters_closed_and_pages_by_offset` (5 issues, 2 done, limit 2: pages of 2, 1, then `next_cursor: None`; `include_closed` gives 5), `sync_since_returns_changes_in_update_order_and_resumes` (no cursor → all; after a `move_issue`, resuming from the returned cursor yields only the moved issue; an empty batch keeps the cursor), `sync_since_breaks_timestamp_ties_by_id` (51 issues with the same `updated_at`: first batch 50 with `has_more`, second batch exactly the 51st), `move_rejects_foreign_state_and_missing_required_fields`, `comment_threads_and_rejects_unknown_parent`, `parse_webhook_reads_single_and_batched_hints` (and rejects `"event":"archived"` and non-JSON), `fail_next_fails_once_then_recovers`.
- [ ] **Step 2: Run** the Task 1 command — FAIL.
- [ ] **Step 3: Implement** `FakeProvider` as above.
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `Add an in-memory board provider for sync tests`.

### Task 3: Move the Linear client under the provider layer

**Files:**
- Create: `crates/gespan-core/src/provider_boards/linear.rs`
- Modify: `crates/gespan-core/src/inbox/linear.rs`

**Interfaces:**
- Produces: `pub struct LinearProvider` with `LinearProvider::new(token: &str) -> Self`, and inherent methods with today's Inbox shapes: `viewer() -> Result<(), String>` (the `VIEWER_QUERY` check), `teams() -> Result<Vec<LinearTeam>, String>`, `inbox_issues(assigned_to_me: bool, state: &str, team_ids: &[String], limit: u32) -> Result<Vec<LinearIssue>, String>`, `issue_details(id) -> Result<LinearIssueDetails, String>`, `issue_thread(id) -> Result<LinearIssueThread, String>`, `post_comment(id, body, parent_id) -> Result<String, String>` (returns the URL; input validation moves here). The `Linear*` data types move too; `inbox::linear` re-exports them so `inbox.rs` compiles unchanged.
- `inbox/linear.rs` keeps `LinearStatus`, the token-file helpers and the seven `linear_*` functions. Each reads the token and calls `LinearProvider`.

- [ ] **Step 1: Write failing characterization tests in `provider_boards/linear.rs`** (against the moved functions): `parses_issue_fields_and_trims_label_hash` (state name/type, team key as `repo`, project, `#ff0000` → `ff0000`, a node without `id` dropped), `nests_replies_under_their_root_comment` (reply to a reply goes under the root; a reply whose parent is missing becomes top level; sorted by `createdAt`), `auth_graphql_errors_become_invalid_key` (`{"errors":[{"message":"Authentication required"}]}` → `"Linear API key is invalid"`; other messages pass through), `issue_filter_matches_inbox_request` (assigned + open + teams → `assignee.isMe`, `team.id.in`, `state.type.nin [completed, canceled]`; `"all"` drops the state clause), `comment_create_requires_success`.
- [ ] **Step 2: Write failing Inbox tests in `inbox/linear.rs`** with a temp data dir holding a `linear-token` file: `comment_validation_happens_before_any_request` (`"bad id!"` → `"Missing Linear issue"`; empty body → `"Comment cannot be empty"`; parent `"x y"` → `"Invalid Linear comment"`), `empty_token_disconnects` (`linear_set_token(dir, "  ")` removes the file and returns `connected: false`), `list_without_token_is_empty`.
- [ ] **Step 3: Run** — FAIL (compile errors at the new path).
- [ ] **Step 4: Move** the transport, queries, types, parsers and validation to `provider_boards/linear.rs` and rewrite the Inbox functions to delegate. Add `pageInfo { hasNextPage endCursor }`, `$after: String` and `state { id … }` to the issues query; the Inbox passes no `after`.
- [ ] **Step 5: Run** the Task 1 command and `… -p gespan-core inbox` — PASS, existing Inbox tests untouched.
- [ ] **Step 6: Commit** `Move the Linear client under provider_boards`.

### Task 4: `BoardProvider` for Linear

**Files:**
- Modify: `crates/gespan-core/src/provider_boards/linear.rs`

**Interfaces:**
- Consumes: Tasks 1 and 3.
- Produces: `impl BoardProvider for LinearProvider`. New queries: `viewer { id name displayName email }`; `team(id) { states(first: 100) { nodes { id name type position } } }` (sorted by `position`); `team(id) { labels(first: 100, after) … }` and `team(id) { members(first: 100, after) … }` with `pageInfo`; `issue(id)` with the list fields plus `description`. `list_issues` maps `IssueFilter` onto `issue_filter` (`include_closed` → `"all"`), passes `after`, and returns `endCursor` as `next_cursor` when `hasNextPage`. `RemoteIssue.key` = identifier, `container_id` = team id. Stubs: `create_issue`, `update_issue`, `move_issue`, `sync_since`, `parse_webhook` → `NotSupported`. Container IDs and issue IDs are validated with `valid_linear_id` before any request (`InvalidInput`).
- Helper: `fn follow_pages(fetch: impl FnMut(Option<String>) -> Result<Value, String>, connection: &str) -> Result<Vec<Value>, String>`, which collects `nodes` and stops on `hasNextPage: false`; `hasNextPage: true` without a new `endCursor` → error `"Linear pagination did not advance"`.

- [ ] **Step 1: Write failing tests:** `viewer_becomes_account` (falls back from `displayName` to `name`), `states_are_ordered_by_position_and_grouped`, `linear_paging_follows_end_cursor_and_rejects_stuck_pages` (two pages of labels collected in order; a repeated cursor errors), `members_use_display_name_and_avatar`, `issue_node_maps_to_remote_issue` (state id/group, labels with ids, assignee as member, description), `list_issues_page_exposes_end_cursor_only_when_more`, `write_operations_are_not_supported_yet` (create/update/move/sync_since/parse_webhook → `NotSupported` with the right operation names; no token needed, so no network), `malformed_ids_are_rejected_before_any_request`.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.** Parsing is in pure `fn parse_*(&Value)` functions so the tests need no network.
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `Implement BoardProvider for Linear reads`.

### Task 5: Move the Jira client and implement `BoardProvider` for Jira

**Files:**
- Create: `crates/gespan-core/src/provider_boards/jira.rs`
- Modify: `crates/gespan-core/src/inbox/jira.rs`

**Interfaces:**
- Produces: `pub struct JiraProvider` built by `JiraProvider::new(site, email, token) -> Result<Self, String>` (normalizes the site and validates the email: `"Enter the email address of your Atlassian account"`). `pub(crate) struct JiraConfig` (same serde shape as the `jira-config.json` file) moves here; `JiraProvider::from_config` / `config()` link the two. Inherent Inbox-shaped methods: `myself_account_id()`, `projects()`, `inbox_issues(assigned_to_me, state, project_ids, limit)`, `issue_details(key)`, `issue_thread(key)`, `post_comment(key, body) -> Result<String, String>`. `inbox/jira.rs` keeps `JiraStatus`, the config-file helpers and the seven `jira_*` functions, delegating.
- `impl BoardProvider for JiraProvider`: whoami (`/rest/api/3/myself`: `accountId`, `displayName`, `emailAddress`), containers (projects), states (`/rest/api/3/project/{id}/statuses`, flattened over issue types, deduplicated by status id, group from `statusCategory.key`), labels (`/rest/api/3/label`, every page by `startAt`/`isLast`), members (`/rest/api/3/user/assignable/search?project={id}&maxResults=100`, `active: false` users dropped), list issues (`/rest/api/3/search/jql` with `nextPageToken` in and out, fields plus `status.id`), get issue (`/rest/api/3/issue/{key}` with `description` as Markdown via the existing ADF converter), comment (existing code; `reply_to` → `InvalidInput("Jira comments cannot be replies")`). Stubs as for Linear. Project IDs must be digits and issue keys must pass `require_issue_key` before any request.

- [ ] **Step 1: Write failing characterization tests at the new path:** `parses_issue_number_url_and_time` (`ENG-42` → 42, `{site}/browse/ENG-42`, `+0000` → `+00:00`), `jql_drops_non_numeric_projects_and_bounds_empty_queries`, `site_normalization` (`yourteam` → `https://yourteam.atlassian.net`, `http://…` rejected, user-info rejected, port kept), `project_pages_are_followed_and_deduplicated` (and a stuck page with `isLast: false` errors), `adf_renders_lists_code_and_links`, `text_to_adf_splits_paragraphs_and_breaks`.
- [ ] **Step 2: Write failing trait and Inbox tests:** `myself_becomes_account`, `jira_states_are_deduplicated_across_issue_types`, `label_pages_are_followed`, `inactive_members_are_dropped`, `issue_maps_to_remote_issue_with_markdown_description`, `search_page_exposes_next_page_token`, `write_operations_are_not_supported_yet`, `replies_are_rejected`, and in `inbox/jira.rs` (temp data dir): `bad_email_is_rejected_before_any_request`, `empty_token_disconnects`, `malformed_issue_key_is_rejected_before_any_request` (`"ENG 1"` → `"Missing Jira issue"`).
- [ ] **Step 3: Run** — FAIL.
- [ ] **Step 4: Move and implement**, keeping parsers pure.
- [ ] **Step 5: Run** the Task 1 command and `… -p gespan-core inbox` — PASS.
- [ ] **Step 6: Commit** `Move the Jira client under provider_boards and implement its reads`.

### Task 6: Verify and open the PR

- [ ] Run `bun run check` and `bun run native:check` and record the result lines.
- [ ] Self-review (superpowers:requesting-code-review), fix findings, re-run both.
- [ ] Push `boards/97-provider-trait` and open a PR to `main` with the repo template, `Closes #97`, and Follow-ups: `src-tauri`'s own Linear/Jira copies still duplicate the clients (they can't use `gespan-core` today); the stubs listed under Design calls.
