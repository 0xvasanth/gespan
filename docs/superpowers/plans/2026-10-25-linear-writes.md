# Linear Writes and Delta Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `LinearProvider` creates, updates, moves (by `stateId`) and comments on issues, and returns resumable, oldest-first delta batches from `sync_since`, without handing our own writes back as remote changes.

**Architecture:** Every GraphQL call goes through a small `LinearTransport` trait. Production uses the existing `ureq` transport, and tests use a scripted transport that records each request (query and variables) and replays a fixture response. Writes are pure "build variables" and "parse payload" functions wrapped by the trait methods. `sync_since` asks Linear for the oldest page of a team's issues newer than an `updatedAt` watermark, and the cursor is that watermark. The provider keeps an in-memory ledger of the `(issue id, updatedAt)` each of its own writes produced, and drops matching rows from sync batches.

**Tech Stack:** Rust 2021, `ureq` 2, `serde_json`, `chrono` (already a dependency). No new dependencies.

**Spec:** GitHub issue #98 (approved design). Background: the Linear section of epic #49. Working rules: #60. Builds on #97 (`provider_boards`, PR #154).

## Global Constraints

- Code changes stay in `crates/gespan-core/src/provider_boards/linear.rs`, plus this plan.
- Scope (from #98): `issueCreate`, `issueUpdate(stateId, …)` and `commentCreate`, delta sync with `updatedAt gt` and the workflow-state `type` mapping, and the OAuth `actor=app` path where available.
- Done when (from #98): request and response fixture tests, and loop prevention drops events whose actor is us.
- The Inbox's requests, error strings and shapes don't change (`inbox_issues`, `issue_details`, `issue_thread`, `post_comment`, `verify`, `teams`).
- Gates: `cargo test --manifest-path crates/gespan-gpui/Cargo.toml -p gespan-core provider_boards`, `bun run check`, `bun run native:check`.
- Gespan names only (`src/test/legacyNames.test.ts`). Commits and the PR carry no tool or AI attribution.

## Design calls (mine, not in the issue)

- **An injectable transport (`LinearTransport`).** "Request and response fixture tests" means checking the exact request each trait method sends and how it reads the reply, so the transport has to be swappable. `LinearProvider::new(token)` keeps the `ureq` path. Tests use `LinearProvider::with_transport(Arc<dyn LinearTransport>)`. The fixtures are inline JSON in the tests, so they don't collide with #109's `provider_boards/fixtures/**`, which holds recorded webhook deliveries.
- **The `actor=app` path.** `actor=app` is a parameter on the OAuth *authorize* URL. A token issued that way makes every mutation come from the app. Gespan has no Linear OAuth flow yet, only personal API keys. So, "where available": `LinearProvider::oauth(access_token)` sends `Authorization: Bearer <token>`, as Linear requires for OAuth tokens. API keys stay raw (and a pasted `Bearer ` prefix is still stripped). The request bodies are the same in both modes. The OAuth/PKCE connect flow is a follow-up.
- **Loop prevention: an echo ledger, not actor ids, for polling.** A Linear `Issue` has no "updated by" field, so delta polling can't see who made a change. Each successful create, update or move records `issue id → updatedAt` (as an instant) from the mutation's response. `sync_since` drops a row whose id and `updatedAt` instant match the ledger, and removes that ledger entry. The cursor still advances past it. A later edit by anyone else has a newer `updatedAt`, so it gets through. The ledger is in memory: #103's durable layers cover restarts. Webhook actor checks belong to `parse_webhook`, which waits on #109's fixtures (as #97 decided).
- **Delta sync order.** The trait promises oldest first. Linear's `orderBy: updatedAt` is newest first (the Inbox and `list_issues` already rely on that), and Linear documents no ascending sort. So `sync_since` asks for `last: 50` (Relay backward paging), which gives the oldest 50 rows of the window, and sorts them ascending by `(instant, id)`.
  - **Guard:** when `hasPreviousPage` is true (more rows exist), the page must come back newest first. If it doesn't, return `Remote("Linear returned issues out of order")`. If Linear ever behaves differently, sync then fails loudly instead of silently skipping issues.
- **The cursor is a millisecond watermark.**
  - `SyncCursor("<ms>")` means every change up to and including that instant has been delivered. An absent or empty cursor means "from the start" (no `updatedAt` filter). Anything else is `InvalidInput`.
  - Each call filters `team.id eq <team>` and `updatedAt gt <ISO of ms>`.
  - If more rows exist, the rows sharing the page's newest instant may straddle the page edge. Those rows are held back for the next call, which re-reads them with `gt`.
  - If the whole page is one instant, the call instead reads that instant fully with `updatedAt eq` (forward paging, `first: 100`) and returns it.
  - `has_more` is `hasPreviousPage`. After a full-instant read it is always `true`, which can cost one extra empty call.
  - With no changes, the cursor comes back unchanged. The batch carries `description`, so #103 doesn't need one `get_issue` per change.
- **Paged states.** `list_states` follows `states(first: 100, after:)` with the existing `follow_pages` (from the #97 round-2 note), so a move target is never missing from a big team.
- **Input rules (`InvalidInput` before any request):**
  - An empty title on create, or a `Some("")` title on update, is rejected.
  - Linear has one assignee, so more than one assignee id is rejected. `assignee_ids: Some(vec![])` on update sends `assigneeId: null` (unassign).
  - An update with no fields set is rejected, since there is nothing to send.
  - A move with an invalid `state_id`, or with non-empty `required_fields` (Linear has no transition screens; refusing beats silently dropping them, the same rule as the fake), is rejected.
  - Malformed ids anywhere (`valid_linear_id`) are rejected.
- **Mutation replies.**
  - `success: false` → `Remote("Could not <create|update> Linear issue")`.
  - `success: true` with no issue → `Remote("Linear did not return the issue")`.
  - The returned issue is parsed with the same `parse_remote_issue` as reads, with description included.

## Review Focus

1. More than 50 issues updated at the same instant (bulk edit): none skipped, none repeated forever. `sync_reads_a_crowded_instant_in_full` (Task 3).
2. Rows sharing the newest instant split across the page edge: held back, then delivered next call. `sync_holds_back_the_boundary_instant` (Task 3).
3. Our own move is not echoed back, but someone else's later edit of the same issue is. `sync_drops_echoes_of_our_own_writes_only` (Task 3).
4. A team with more than 100 workflow states: every state listed, so moves can target any of them. `states_follow_every_page` (Task 2).
5. A bad or foreign cursor (`"abc"`, `"12|x"`) is rejected without a request, and an empty cursor starts from the beginning. `sync_cursor_empty_means_start_and_garbage_is_rejected` (Task 3).

---

### Task 1: Transport seam and OAuth bearer auth

**Files:** Modify `crates/gespan-core/src/provider_boards/linear.rs`

**Interfaces:**
- Produces:
  - `pub trait LinearTransport: Send + Sync { fn execute(&self, query: &str, variables: Value) -> Result<Value, String>; }`
  - `LinearProvider { transport: Arc<dyn LinearTransport>, ledger: Arc<Mutex<HashMap<String, i64>>> }`
  - `LinearProvider::new(token)` (API key) and `LinearProvider::oauth(access_token)` (Bearer)
  - `#[cfg(test)] LinearProvider::with_transport(Arc<dyn LinearTransport>)`
  - `fn authorization_header(auth: &LinearAuth) -> String`
  - Test helper `Scripted`: a queue of `Result<Value, String>` replies, with `requests() -> Vec<(String, Value)>`.

- [ ] Failing test `oauth_tokens_use_bearer_and_api_keys_stay_raw`: `Bearer abc` for OAuth `" abc "`, and `lin_api_x` for the API key `"Bearer lin_api_x"`.
- [ ] Failing test `scripted_transport_records_whoami_request`: `whoami` via `Scripted` sends `VIEWER_ACCOUNT_QUERY` with `{}` and returns the parsed account. A queued error string surfaces as `ProviderError::Remote`.
- [ ] Implement, run `provider_boards::linear` (pass), and commit.

### Task 2: Writes: create, update, move, and paged states

**Files:** Modify `linear.rs`

**Interfaces:**
- Produces:
  - `fn create_variables(team_id: &str, draft: &IssueDraft) -> ProviderResult<Value>`, giving `{"input": {teamId, title, description?, stateId?, labelIds?, assigneeId?}}`. Empty optional fields are omitted.
  - `fn update_variables(id: &str, patch: &IssuePatch) -> ProviderResult<Value>`, giving `{"id", "input": {title?, description?, labelIds?, assigneeId?}}`.
  - `fn move_variables(id: &str, target: &MoveTarget) -> ProviderResult<Value>`, giving `{"id", "input": {"stateId"}}`.
  - `fn parse_issue_payload(data: &Value, field: &str, verb: &str) -> Result<RemoteIssue, String>`
  - `ISSUE_CREATE_MUTATION` and `ISSUE_UPDATE_MUTATION`, both selecting `success issue { <BOARD_ISSUE fields incl. description> }`
  - `TEAM_STATES_QUERY` with `$after` and `pageInfo`

- [ ] Failing tests:
  - `create_sends_team_title_and_optional_fields`: the exact variables, and the reply is parsed into a `RemoteIssue` with state group `Unstarted` from `type: "unstarted"`.
  - `create_rejects_empty_title_and_two_assignees_without_a_request`: `Scripted` records 0 requests.
  - `update_sends_only_set_fields_and_null_unassigns`
  - `update_rejects_an_empty_patch_and_empty_title`
  - `move_sends_state_id_and_maps_the_new_state_type`: `type: "completed"` → `Completed`.
  - `move_rejects_required_fields_and_bad_state_ids`
  - `mutation_failures_are_remote_errors`: `success: false`, and `success: true` with `issue: null`.
  - `comment_sends_issue_body_and_parent`: the exact `commentCreate` input, and `CommentRef` back.
  - `states_follow_every_page`: two `Scripted` pages, all states returned in position order, and the second request carries `after: "c1"`.
- [ ] Remove `write_operations_are_not_supported_yet`, except for `parse_webhook`, which stays `NotSupported`.
- [ ] Implement, test (pass), and commit.

### Task 3: Delta sync with echo-ledger loop prevention

**Files:** Modify `linear.rs`

**Interfaces:**
- Consumes: the ledger from Task 1 (written by create, update and move in Task 2: `record_write(&RemoteIssue)`).
- Produces:
  - `SYNC_ISSUES_QUERY($last, $filter)`, selecting `pageInfo { hasPreviousPage }` and the issue fields incl. description.
  - `SYNC_INSTANT_QUERY($first, $after, $filter)`, which pages forward.
  - `fn decode_sync_cursor(&SyncCursor) -> ProviderResult<Option<i64>>`
  - `fn iso_millis(ms: i64) -> String`, giving e.g. `2026-03-01T10:00:00.000Z`.

- [ ] Failing tests:
  - `sync_from_start_has_no_updated_filter_and_returns_oldest_first`: the request filter is exactly `{team:{id:{eq:"t1"}}}` with `last: 50`. A reply in newest-first order comes back oldest first, `has_more` is false, and the cursor is the newest ms.
  - `sync_resumes_with_updated_at_gt_the_watermark`: the filter has `updatedAt: {gt: "2026-03-01T10:00:00.000Z"}`.
  - `sync_with_no_changes_keeps_the_cursor`
  - `sync_holds_back_the_boundary_instant`: `hasPreviousPage` true, and the rows at the newest instant are held back.
  - `sync_reads_a_crowded_instant_in_full`: the page is a single instant, then two `eq` pages; everything is returned, and the cursor is that instant.
  - `sync_rejects_an_out_of_order_page`
  - `sync_cursor_empty_means_start_and_garbage_is_rejected`
  - `sync_drops_echoes_of_our_own_writes_only`: move via `Scripted`, then sync returns that row (dropped) plus another, then sync returns the same id with a newer `updatedAt` (kept).
- [ ] Also test that the sync contract matches the fake: `sync_contract_matches_the_fake`. Run the same scenario (three issues, resume with the returned cursor, one later move) against `FakeProvider` and against a `Scripted` Linear, and assert the same id order and the same "resume returns only newer" behaviour.
- [ ] Implement, test (pass), and commit.

### Task 4: Verify and ship

- [ ] Run `cargo fmt`, the clippy gate, `bun run check` and `bun run native:check` (one at a time), and quote the result lines.
- [ ] Self-review (superpowers:requesting-code-review), fix what it finds, and open the PR against `boards/97-provider-trait`.

## Follow-ups (not in #98)

- A Linear OAuth (PKCE) connect flow that requests `actor=app` and refreshes tokens.
- `parse_webhook` for Linear with an actor-id check, once #109's fixtures land.
- Archived and trashed issues in `sync_since` (`includeArchived`), for #103's "remote delete archives the card".
- Reuse one `ureq::Agent` per provider (already in #154's follow-ups).

## Self-review changes (after Task 3)

- **A full page counts as "more"** even when `hasPreviousPage` is false, so the boundary hold-back can't be skipped by a mis-set flag.
- **A cheap probe** (`issues(first: 1)` with `updatedAt lt <oldest on page>`) runs on every page that has more. It proves the `last:` page really is the oldest end. If anything older turns up, sync stops with `Remote("Linear skipped older issues while syncing")`.
- **The crowded-instant read must include every row the page already showed at that instant.** If it doesn't, sync stops with `Remote("Linear returned an incomplete set of issues")`.
- **Ledger entries are retired only when a newer `updatedAt` supersedes them**, not on the first match. A replay from an older cursor still leaves our write out.
- **Bad input is rejected:** an out-of-range cursor is `InvalidInput`, and a sync row without `updatedAt` is `Remote`.
- **Docs:** the trait's `sync_since` comment says providers may leave out their own writes, that callers apply what writes return, and that they still tolerate echoes. The fake deliberately doesn't leave them out.
- **Not done, recorded as a risk:** the commit-visibility race of any `updatedAt` watermark. A transaction stamped before the cursor can become visible after it. #103's webhooks and periodic reconcile are the backstop. A fixed overlap window would re-deliver the same rows on every quiet poll.
