# Review setup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn on an automatic PR reviewer in two clicks from the Automations gallery. It runs read-only, and where the review goes (Gespan, approved post, or direct post) is an explicit, enforced setting for every agent.

**Architecture:** A new core module, `review_policy`, holds one pure decision function: given a review policy and a shell command, it answers Allow, Ask or Deny. All five harnesses call it from their existing permission points. Automations gain an optional `review_results`, accept `read-only` as a run mode, and get a pure `review_draft` builder. The GPUI side adds a review setup sheet in front of the existing editor. When a review automation runs, its launch attaches the policy, built from the run's event key, to `agent::Settings`.

**Tech Stack:** Rust, GPUI, serde, `shlex` (already a `gespan-core` dependency), the `gh` CLI.

**Spec:** `docs/design/review-setup.md`.

## Global Constraints

- `gespan-core` and `gespan-gpui` are each their own Cargo workspace. Run tests from `crates/gespan-core` and from `crates/gespan-gpui` with `cargo test <filter>`.
- Tests run on macOS, Windows and Linux CI. Use no macOS-only paths, and stub `gh` through the existing test seams, never the real CLI.
- Existing automations load and run unchanged. A missing `reviewResults` key means "not a review automation".
- A reviewer never edits files. It posts only on the PR that triggered the run (`--repo` and number must match), and only when Results is Draft (approval) or Post.
- PR text is untrusted input. Command matching is done by Gespan on parsed words, never by asking the agent.
- Copy is taken verbatim from the spec. Sentence case, no emoji.
- Commit messages carry no tool attribution (user rule).

## Review Focus

1. **Chained or redirected commands:** `gh pr diff 7 -R acme/app; gh pr merge 7`, `&&`, `|`, `$(…)`, backticks, `>` must be denied whole, even though the first command is a read. → Task 1 test `chained_commands_are_denied`.
2. **Spelling variants of the target:** `--repo=Acme/App`, `-R acme/app`, and a PR URL in place of the number must match the lowercased event-key target. A different number or repo must not. → Task 1 test `target_matches_spelling_variants_only`.
3. **A review automation run with no PR** (Run now, or a time trigger added in Customize): with no target, Draft and Post must deny every write, and reads still work. → Task 1 test `no_target_denies_every_write`.
4. **`gh api` that writes without saying POST:** `-f`/`-F`/`--field`/`--raw-field`/`--input` imply POST, and `-X PATCH`/`--method=DELETE` must be denied. `--method GET` with `-f` is a read. → Task 1 test `gh_api_reads_only`.
5. **A stale readiness result:** the sheet is closed and reopened, or the project changes, while a `gh auth status` check is in flight. The old result must not enable Turn on. → Task 9 test `stale_readiness_result_is_ignored`.

---

### Task 1: The review command policy

**Files:**
- Create: `crates/gespan-core/src/review_policy.rs` (unit tests inline in `mod tests`, as `provider/claude/read_only.rs` does)
- Modify: `crates/gespan-core/src/lib.rs` (add `pub mod review_policy;`)

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "kebab-case")] pub enum ReviewResults { Gespan, Draft, Post }`
  - `#[derive(Clone, Debug, PartialEq, Eq)] pub struct ReviewTarget { pub repo: String /* lowercase owner/name */, pub number: u64 }`
  - `impl ReviewTarget { pub fn from_event_key(key: &str) -> Option<Self> }`. This parses `github:<kind-or-event>:<owner/repo>:<number>[:<rest>]`. Anything that isn't `github`, has no `/` in the repo or has a non-numeric number gives `None`.
  - `#[derive(Clone, Debug, PartialEq)] pub struct ReviewPolicy { pub results: ReviewResults, pub target: Option<ReviewTarget> }`
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Verdict { Allow, Ask, Deny }`
  - `pub fn verdict(policy: &ReviewPolicy, command: &str) -> Verdict`
  - `pub fn instruction(policy: &ReviewPolicy) -> String`: the text appended to the run's prompt.

- [ ] **Step 1: Write the failing tests**

```rust
fn policy(results: ReviewResults) -> ReviewPolicy {
    ReviewPolicy { results, target: Some(ReviewTarget { repo: "acme/app".into(), number: 7 }) }
}

#[test] fn reads_are_allowed_in_every_mode() {
    for r in [ReviewResults::Gespan, ReviewResults::Draft, ReviewResults::Post] {
        for c in ["gh pr view 7 -R acme/app", "gh pr diff 7 --repo acme/app",
                  "gh pr checks 7 -R acme/app", "gh pr list -R acme/app",
                  "gh api repos/acme/app/pulls/7/files", "gh api --method GET repos/acme/app/pulls/7 -f per_page=100"] {
            assert_eq!(verdict(&policy(r), c), Verdict::Allow, "{r:?} {c}");
        }
    }
}
#[test] fn posting_follows_results() {
    let c = r#"gh pr review 7 -R acme/app --comment --body "Looks good; one blocker""#;
    assert_eq!(verdict(&policy(ReviewResults::Gespan), c), Verdict::Deny);
    assert_eq!(verdict(&policy(ReviewResults::Draft), c), Verdict::Ask);
    assert_eq!(verdict(&policy(ReviewResults::Post), c), Verdict::Allow);
    let c = r#"gh pr comment 7 --repo acme/app --body "x""#;
    assert_eq!(verdict(&policy(ReviewResults::Post), c), Verdict::Allow);
}
#[test] fn target_matches_spelling_variants_only() {
    let p = policy(ReviewResults::Post);
    for ok in [r#"gh pr review 7 --repo=Acme/App --comment -b "x""#,
               r#"gh pr review https://github.com/acme/app/pull/7 --comment -b "x""#] {
        assert_eq!(verdict(&p, ok), Verdict::Allow, "{ok}");
    }
    for bad in [r#"gh pr review 8 -R acme/app --comment -b "x""#,
                r#"gh pr review 7 -R acme/other --comment -b "x""#,
                r#"gh pr review 7 --comment -b "x""#, /* no repo: cwd repo is not trusted */] {
        assert_eq!(verdict(&p, bad), Verdict::Deny, "{bad}");
    }
}
#[test] fn approve_and_request_changes_are_never_allowed() {
    // Only --comment reviews; approving or blocking a PR is a human decision.
    for c in [r#"gh pr review 7 -R acme/app --approve"#, r#"gh pr review 7 -R acme/app --request-changes -b "x""#] {
        assert_eq!(verdict(&policy(ReviewResults::Post), c), Verdict::Deny, "{c}");
    }
}
#[test] fn chained_commands_are_denied() {
    for c in ["gh pr diff 7 -R acme/app; gh pr merge 7 -R acme/app", "gh pr diff 7 -R acme/app && rm -rf .",
              "gh pr diff 7 -R acme/app | sh", "gh pr view $(echo 7) -R acme/app",
              "gh pr view `echo 7` -R acme/app", "gh pr diff 7 -R acme/app > out.txt"] {
        assert_eq!(verdict(&policy(ReviewResults::Post), c), Verdict::Deny, "{c}");
    }
}
#[test] fn other_writes_are_denied() {
    for c in ["gh pr merge 7 -R acme/app", "gh pr edit 7 -R acme/app --add-label x", "gh pr close 7 -R acme/app",
              "gh issue comment 7 -R acme/app -b x", "git push", "rm file", "gh auth logout"] {
        assert_eq!(verdict(&policy(ReviewResults::Post), c), Verdict::Deny, "{c}");
    }
}
#[test] fn gh_api_reads_only() {
    let p = policy(ReviewResults::Post);
    for c in ["gh api -X POST repos/acme/app/pulls/7/reviews", "gh api --method=DELETE repos/acme/app",
              "gh api repos/acme/app/issues/7/comments -f body=x", "gh api graphql -F query=@q.graphql",
              "gh api repos/acme/app --input body.json", "gh api -XPATCH repos/acme/app"] {
        assert_eq!(verdict(&p, c), Verdict::Deny, "{c}");
    }
}
#[test] fn no_target_denies_every_write() {
    let p = ReviewPolicy { results: ReviewResults::Post, target: None };
    assert_eq!(verdict(&p, r#"gh pr review 7 -R acme/app --comment -b "x""#), Verdict::Deny);
    assert_eq!(verdict(&p, "gh pr diff 7 -R acme/app"), Verdict::Allow);
}
#[test] fn non_gh_reads_defer_to_read_only_rules() {
    // Git/ls reads keep using claude::read_only::allowed; the policy only answers for gh.
    assert_eq!(verdict(&policy(ReviewResults::Gespan), "git diff HEAD~1"), Verdict::Allow);
    assert_eq!(verdict(&policy(ReviewResults::Gespan), "cat secrets"), Verdict::Deny);
}
#[test] fn event_keys_parse_to_targets() {
    assert_eq!(ReviewTarget::from_event_key("github:pull_request_review_requested:acme/app:7:abc1234def"),
               Some(ReviewTarget { repo: "acme/app".into(), number: 7 }));
    assert_eq!(ReviewTarget::from_event_key("github:pr:acme/app:12"), Some(ReviewTarget { repo: "acme/app".into(), number: 12 }));
    for k in ["linear:issue:lin-1", "github:pr:acme:7", "github:pr:acme/app:x", "github:pr:acme/app"] {
        assert_eq!(ReviewTarget::from_event_key(k), None, "{k}");
    }
}
#[test] fn instruction_names_the_exact_command_or_forbids_posting() {
    let i = instruction(&policy(ReviewResults::Draft));
    assert!(i.contains("gh pr review 7 --repo acme/app --comment --body"));
    let i = instruction(&policy(ReviewResults::Gespan));
    assert!(i.contains("Do not post to GitHub"));
    let i = instruction(&ReviewPolicy { results: ReviewResults::Post, target: None });
    assert!(i.contains("Do not post to GitHub"));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd crates/gespan-core && cargo test review_policy`
Expected: compile error, because `review_policy` does not exist.

- [ ] **Step 3: Implement the module**

  - **Parsing.**
    - First, reject the raw command if it contains any of `;`, `&`, `|`, `` ` ``, `$(`, `>`, `<` or a newline outside a quoted string. Scan with a small quote-aware loop.
    - Then split with `shlex::split`. `None` means Deny.
  - **Answers by command.**
    - Not `gh`: return `claude::read_only::allowed(command)` mapped to Allow or Deny. Make that function `pub(crate)`-reachable if it isn't already.
    - `gh pr view|diff|checks|list`: Allow.
    - `gh api`: Allow only when no flag is `-X`/`--method` with a non-GET value (including the joined `-XPOST` and `--method=X` forms), and no `-f`/`-F`/`--field`/`--raw-field`/`--input` appears unless `--method GET` is explicit.
    - `gh pr review` or `gh pr comment`:
      - Repo: `-R`/`--repo`/`--repo=` is required unless the selector is a URL. Compare case-insensitively to `target.repo`.
      - Selector: a number or `https://github.com/<repo>/pull/<n>` that matches the target.
      - `gh pr review` also needs `--comment`/`-c` and must have no `--approve`/`-a`/`--request-changes`/`-r`.
      - If any of that fails, or the target is `None`, Deny.
      - Otherwise the verdict follows Results: Gespan → Deny, Draft → Ask, Post → Allow.
    - Everything else: Deny.
  - **The instruction.** Exact copy:
    - Gespan, or no target: `"Write your review as your final reply. Do not post to GitHub, and do not change any files."`
    - Draft or Post: `"When your review is ready, post it with exactly: gh pr review {n} --repo {repo} --comment --body \"<your review>\". Post nothing else, and do not change any files."` Add for Draft: `" The user approves the post before it reaches GitHub."`

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd crates/gespan-core && cargo test review_policy`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/gespan-core/src/review_policy.rs crates/gespan-core/src/lib.rs
git commit -m "Add the review command policy"
```

---

### Task 2: Review automations in storage and the draft builder

**Files:**
- Modify: `crates/gespan-core/src/automations.rs`. Touch the `Automation` (:67) and `AutomationUpsert` (:118) structs, the `upsert`/`from_automation` mapping, and the `runtime_mode` validation at :340-345. Add tests to its existing `mod tests`.
- Create: `crates/gespan-core/src/automations/review_setup.rs`. If `automations.rs` can't take a child module without a move, use `crates/gespan-core/src/review_setup.rs` instead, keeping the same API.

**Interfaces:**
- Consumes: `review_policy::ReviewResults`.
- Produces:
  - `pub review_results: Option<ReviewResults>` on both `Automation` and `AutomationUpsert`, with `#[serde(default, skip_serializing_if = "Option::is_none")]`.
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum WhichPrs { ReviewRequested, Opened, OpenedAndDrafts }`
  - `pub struct ReviewSetupInput { pub cwd: String, pub repo: Option<String> /* None = any repo */, pub which: WhichPrs, pub harness: String, pub model: String, pub results: ReviewResults }`
  - `pub fn review_draft(input: &ReviewSetupInput) -> AutomationUpsert`
  - `pub fn find_review_automation<'a>(automations: &'a [Automation], cwd: &str) -> Option<&'a Automation>`: same `cwd` and `review_results.is_some()`.
  - `pub const REVIEW_PROMPT: &str`: the existing "Review pull requests" template prompt, with its last paragraph cut down to "Blockers first, then suggestions. Do not nitpick formatting. If the change is good, say so briefly and note residual risk." The posting instruction comes from `review_policy::instruction` at launch, not from here.

- [ ] **Step 1: Write the failing tests**

  - `review_draft_maps_every_choice`: loop over every `WhichPrs` × `ReviewResults` and assert all of:
    - `name == "Review pull requests"`
    - `workspace_mode == "current"`
    - `!reuse_session`
    - `runtime_mode == "read-only"`
    - `action == AutomationAction::Session`
    - `review_results == Some(results)`
    - `prompt == REVIEW_PROMPT`
    - The trigger events are `["pull_request_review_requested"]`, `["pull_request_opened"]`, or `["pull_request_opened", "draft_opened"]`. Every trigger has kind `"github"`.
    - `repos` is `vec!["acme/app"]` for `Some("acme/app")` and empty for `None`.
  - `review_draft_saves_and_reloads`: `Service::upsert(review_draft(..))`, then `list()`. The saved automation keeps `review_results` and `runtime_mode == "read-only"`.
  - `automations_without_review_results_load_unchanged`: put a definition JSON with no `reviewResults` key and `runtime_mode "auto"` into the table, then `list()`. Assert `review_results == None` and `runtime_mode == "auto"`.
  - `read_only_is_a_valid_run_mode`: `upsert` with `runtime_mode = "read-only"` is Ok, and `"readonly"` is still Err.
  - `find_review_automation_matches_project_and_kind`: a review automation for `/a`, a non-review one for `/a`, and a review one for `/b`. Asking for `/a` returns the first.

- [ ] **Step 2: Run to verify they fail**

Run: `cd crates/gespan-core && cargo test automations::`
Expected: compile errors on `review_results`, `review_draft`.

- [ ] **Step 3: Implement**

  - Add the fields.
  - Add `"read-only"` to the `runtime_mode` match.
  - Carry `review_results` through `upsert` → stored definition JSON → `Automation`, and through `from_automation`.
  - Write `review_draft` on top of `AutomationUpsert::new(cwd, harness, model)` so the other defaults stay the editor's.

- [ ] **Step 4: Run to verify they pass**

Run: `cd crates/gespan-core && cargo test automations::`
Expected: pass, with the existing automation tests still green.

- [ ] **Step 5: Commit** with message `Store review results on automations and build review drafts`.

---

### Task 3: Claude enforces the policy

**Files:**
- Modify: `crates/gespan-core/src/agent.rs`. Add to `Settings` (:88-175), next to `mount`: `#[serde(skip)] pub review: Option<crate::review_policy::ReviewPolicy>`.
- Modify: `crates/gespan-core/src/provider.rs`, inside `fn approval` (:532), between the `read_only_query` computation (:573-579) and the read-only deny (:591-599).
- Test: `crates/gespan-core/src/provider.rs` tests, next to `claude_bash` (:2100) and the allow/deny test (:2113).

**Interfaces:**
- Consumes: `review_policy::{verdict, Verdict}`, `Settings.review`.
- Produces: `Settings.review`, which every later harness task and Task 8 read.

- [ ] **Step 1: Write the failing tests**

Use the existing `claude_bash` helper with `settings.access = ReadOnly` and `settings.review = Some(policy(..))`. Name the test `claude_review_policy_decides_bash`.
- `gh pr diff 7 -R acme/app` is answered Allow with no `Event::Approval`.
- Under Draft, `gh pr review 7 -R acme/app --comment -b "x"` produces exactly one `Event::Approval`, with no automatic reply.
- Under Post, the same command is answered Allow.
- Under Gespan, it is answered Deny.
- `gh pr merge 7 -R acme/app` is answered Deny in all three modes.
- With `review: None`, today's behavior is unchanged: `gh pr diff` is denied in read-only.

- [ ] **Step 2: Run to verify it fails**

Run: `cd crates/gespan-core && cargo test claude_review_policy`
Expected: compile error on `review`.

- [ ] **Step 3: Implement**

When `kind == "claude"`, the tool is `Bash`, and `self.request.settings.review` is `Some`, take the verdict from `review_policy::verdict` before the read-only deny:
- Allow → `respond(Allow)`.
- Deny → `respond(Deny)`.
- Ask → fall through to the `Event::Approval` push. Skip the read-only deny for this request only.

- [ ] **Step 4: Run to verify it passes**

Run: `cd crates/gespan-core && cargo test claude_review_policy && cargo test provider::`
Expected: pass.

- [ ] **Step 5: Commit** with message `Enforce the review policy for Claude`.

---

### Task 4: Codex enforces the policy

**Files:**
- Modify: `crates/gespan-core/src/provider/lifecycle.rs`, `codex_access()` (:364-374).
- Modify: `crates/gespan-core/src/provider/codex.rs`, at the `thread/start` (:127-128) and `turn/start` (:176-203) parameters, and at the approval handler (:362-380).
- Modify: `crates/gespan-core/src/provider.rs`, `fn approval`. Extend the Task 3 branch to `kind` values starting with `codex`, using the request's `command` (Codex sends a string or an argv array; join an array with `shlex::try_join`).
- Test: `crates/gespan-core/src/provider.rs` tests, next to the network-access test (:1726) and the sandbox-parameter tests (:2279/:2289).

**Interfaces:**
- Consumes: `Settings.review`, `review_policy::verdict`.

- [ ] **Step 1: Write the failing tests**

Name the test `codex_review_runs_ask_gespan_with_network`.
- With `access = ReadOnly` and `review = Some(..)`, the `thread/start` and `turn/start` parameters have `approvalPolicy: "untrusted"`, a read-only sandbox, and `networkAccess: true`.
- Without `review`, they are exactly today's (`"never"`, no network).
- An `execCommandApproval` for `gh pr diff 7 -R acme/app` is answered Allow.
- Under Draft, one for `gh pr review 7 -R acme/app --comment -b x` raises `Event::Approval`.
- One for `rm x` is answered Deny.

- [ ] **Step 2: Run to verify it fails.** Run: `cd crates/gespan-core && cargo test codex_review`. Expected: FAIL.

- [ ] **Step 3: Implement**

`codex_access()` takes the review case into account: approval policy `"untrusted"` and read-only sandbox with network access. The approval handler routes through the shared branch in `fn approval`.

- [ ] **Step 4: Run to verify it passes.** Run: `cd crates/gespan-core && cargo test codex && cargo test launch_argv`. Expected: pass. Re-record the launch-argv fixture only if Codex argv changed, with `GESPAN_BLESS_LAUNCH_ARGV=1`, and check the diff touches only the review case.

- [ ] **Step 5: Commit** with message `Enforce the review policy for Codex`.

---

### Task 5: ACP agents enforce the policy

**Files:**
- Modify: `crates/gespan-core/src/provider/acp.rs`, in the `session/request_permission` handler (:909-1003), before the planning deny (:978-991).
- Test: `crates/gespan-core/src/provider/acp.rs` tests, next to "planning denies execution" (:2865).

**Interfaces:**
- Consumes: `Settings.review`, `review_policy::verdict`.

- [ ] **Step 1: Write the failing test**

Name the test `acp_review_policy_decides_execute`. Use a `kind: "execute"` permission request carrying the command in the tool call's raw input.
- Allow is answered `allow_once`.
- Ask pushes `Event::Approval`.
- Deny is answered `reject_once`.
- An `edit` kind is still rejected when a review policy is set.
- With no review policy, the existing planning test is unchanged.

- [ ] **Step 2: Run to verify it fails.** Run: `cd crates/gespan-core && cargo test acp_review`. Expected: FAIL.

- [ ] **Step 3: Implement**

When planning and `review` is `Some` and `kind == "execute"`, pull the command string out of the tool call:
- Use the same field `ToolCall` presents as the command (`rawInput.command`, string or array).
- If no command can be found, Deny.

Apply the verdict through `acp::respond`.

- [ ] **Step 4: Run to verify it passes.** Run: `cd crates/gespan-core && cargo test acp`. Expected: pass.

- [ ] **Step 5: Commit** with message `Enforce the review policy for ACP agents`.

---

### Task 6: opencode enforces the policy

**Files:**
- Modify: `crates/gespan-core/src/provider/opencode/events.rs`, the `permission.asked` branch (:430-482).
- Test: `crates/gespan-core/src/provider/opencode.rs` tests, next to the permission rows (:1270).

**Interfaces:**
- Consumes: `Settings.review`, `review_policy::verdict`.
- Produces: `fn permission_command(permission: &Value) -> Option<String>` in `events.rs`. It reads opencode's `pattern` field (a string or array of strings), falling back to `metadata.command`.

- [ ] **Step 1: Write the failing test**

Name the test `opencode_review_policy_decides_bash`. Cover:
- `type: "bash"` with an allowed read → `Client::permission(.., true)`
- the target post under Draft → `Event::Approval`
- under Post → allowed
- `gh pr merge` → `Client::permission(.., false)`
- `edit` → still rejected

- [ ] **Step 2: Run to verify it fails.** Run: `cd crates/gespan-core && cargo test opencode_review`. Expected: FAIL.

- [ ] **Step 3: Implement** the branch ahead of the planning reject, using `permission_command`. If there is no command, reject.

- [ ] **Step 4: Run to verify it passes.** Run: `cd crates/gespan-core && cargo test opencode`. Expected: pass.

- [ ] **Step 5: Commit** with message `Enforce the review policy for opencode`.

---

### Task 7: Pi reviews from a prefetched PR, Gespan-only

Pi's read-only mode has no shell tool and no per-tool permission round-trip (`pi.rs:37-46`, `:619-641`). It can't run `gh` reads, and it can't have a gated post. This uses the spec's stated fallback: Pi offers "keeps the review in Gespan" only. Gespan fetches the PR for it.

**Files:**
- Modify: `crates/gespan-core/src/review_policy.rs`.
- Test: the same file.

**Interfaces:**
- Produces:
  - `pub fn supported_results(kind: crate::provider::Kind) -> &'static [ReviewResults]`. Pi and omp return `&[Gespan]`. Every other kind returns all three.
  - `pub fn needs_prefetch(kind: crate::provider::Kind) -> bool`. True for Pi and omp.
  - `pub fn prefetch_context(cwd: &std::path::Path, target: &ReviewTarget) -> Result<String, String>`. It runs `gh pr view {n} --repo {repo} --json title,body,author,baseRefName,headRefName` and `gh pr diff {n} --repo {repo}` through `crate::inbox`'s existing `run_gh` helper (expose it as `pub(crate)`). It returns a block that starts `"Pull request {repo}#{n}"`, then the title, author, branches, body, and the diff in a fenced block. The diff is truncated at 400 KB with a line saying it was cut.

- [ ] **Step 1: Write the failing tests**
  - `pi_supports_gespan_only`: `supported_results(Kind::Pi) == [Gespan]`, and Claude, Codex and opencode return all three.
  - `prefetch_context_formats_and_truncates`: point the `gh` binary at a stub script through the existing `gh` resolution seam (the one Inbox tests use). Check the header line, that the diff is fenced, and the truncation note for a 500 KB diff.

- [ ] **Step 2: Run to verify they fail.** Run: `cd crates/gespan-core && cargo test review_policy`. Expected: FAIL.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run to verify they pass.** Same command; pass.

- [ ] **Step 5: Commit** with message `Let Pi review from a prefetched pull request`.

---

### Task 8: A review run carries its policy

**Files:**
- Modify: `crates/gespan-gpui/src/shell/automation_runs.rs`, `launch_automation` (:392), next to `settings.access = …` (:491).
- Test: the same file's `mod tests` (:953 onward).

**Interfaces:**
- Consumes:
  - `Automation.review_results`
  - `ReviewTarget::from_event_key(run.event_key)`
  - `review_policy::{instruction, needs_prefetch, prefetch_context}`
  - `Settings.review`
- Produces: `fn review_launch(automation: &Automation, run: &AutomationRun) -> Option<ReviewPolicy>` (pure; a free function in this file).

- [ ] **Step 1: Write the failing tests**

Name the test `review_runs_carry_policy_and_instruction`.
- `review_launch` returns `None` when `review_results` is `None`.
- For `review_results = Draft` and event key `github:pull_request_review_requested:acme/app:7:abc`, it returns a policy targeting `acme/app#7` under Draft.
- For a Run now run (no event key), the target is `None`.

- [ ] **Step 2: Run to verify it fails.** Run: `cd crates/gespan-gpui && cargo test review_runs_carry`. Expected: FAIL.

- [ ] **Step 3: Implement**

In `launch_automation`:
- Set `settings.review = review_launch(..)`.
- Append `"\n\n" + instruction(&policy)` to the run's prompt.
- When `needs_prefetch(kind)` and there is a target, run `prefetch_context` inside the existing background `work` task and prepend its output to the prompt. If it fails, the run fails with the error, recorded through `update_automation_run` as other launch errors are.

- [ ] **Step 4: Run to verify it passes.** Run: `cd crates/gespan-gpui && cargo test automation_runs`. Expected: pass.

- [ ] **Step 5: Commit** with message `Attach the review policy to review automation runs`.

---

### Task 9: The review setup sheet

**Files:**
- Create: `crates/gespan-gpui/src/automations/review_setup.rs`. It holds the state, the pure readiness rules and the render.
- Create: `crates/gespan-gpui/src/automations/review_setup_tests.rs`.
- Modify: `crates/gespan-gpui/src/automations/templates.json`. Remove `review-draft-prs`. Give `review-pull-requests` the new description "Review pull requests automatically, without changing any files" and add `"setup": "review"`.
- Modify: `crates/gespan-gpui/src/automations.rs`:
  - add `#[serde(default)] setup: Option<String>` to `Template` (:72)
  - add a `review_setup: Option<ReviewSetup>` field to `Automations`
  - add `fn open_review_setup(&mut self, cx)`
  - the gallery click (`gallery.rs`, at the `begin(Some(selected…))` call) goes to `open_review_setup` when `template.setup == Some("review")`
- Modify: `crates/gespan-core/src/inbox.rs`. Make `github_repositories` (:728) `pub`.
- Modify: `crates/gespan-gpui/src/shell/terminal_panes.rs` or `shell.rs`. Handle `Event::SignInGithub` by opening a terminal pane (`open_terminal_pane`, :262) running `gh auth login --web` in the project, and calling `Automations::recheck_github` when it exits.

**Interfaces:**
- Consumes:
  - `review_draft`, `ReviewSetupInput`, `WhichPrs`, `find_review_automation` (Task 2)
  - `supported_results` (Task 7)
  - `inbox::Service::status("github")`
  - `inbox::github_repositories`
  - `remote_workspace::is_remote`
- Produces:
  - `pub(crate) enum Readiness { Checking, NotInstalled, SignedOut, Connected, NoGithubRemote, Remote }`
  - `pub(crate) fn readiness(installed: bool, authenticated: bool, repos: &[String], remote: bool, chosen_repo: Option<&str>) -> Readiness`
  - `pub(crate) fn can_turn_on(r: Readiness) -> bool`: true only for `Connected`
  - `pub(crate) fn default_repo(repos: &[String]) -> Option<String>`: `Some` only when there is exactly one
  - `pub(crate) struct ReviewSetup { input: ReviewSetupInput, readiness: Readiness, repos: Vec<String>, generation: u64 }`
  - Sheet event: `Event::SignInGithub { cwd: String }`, added to the existing `automations::Event` enum.

**Copy (verbatim):**
- Sentence: "When a pull request in {repo} {which}, Gespan reviews it with {model}, without changing any files, and {results}."
- Which: "asks for your review" / "is opened" / "is opened, drafts too"
- Results: "keeps the review in Gespan" / "drafts a review for you to approve" / "posts the review on the PR"
- Repo: "any repo in this project" when `None`
- Readiness:
  - "Checking GitHub…"
  - "GitHub CLI isn't installed." with buttons "Get GitHub CLI" and "Check again"
  - "Sign in to GitHub to continue." with buttons "Sign in" and "Check again"
  - "GitHub connected ✓"
  - "This project isn't on GitHub." (Repository shows "Choose a repository…")
  - "Quick setup isn't available for remote projects yet."
- Buttons: "Turn on", "Customize…", "Cancel"

- [ ] **Step 1: Write the failing tests** in `review_setup_tests.rs`
  - `readiness_rules`: every row of the spec's readiness table maps to its state, and `can_turn_on` is true only for Connected. `NoGithubRemote` with a chosen repo becomes Connected when signed in. `Remote` wins over everything.
  - `default_repo_only_when_single`: `[] → None`, `["acme/app"] → Some`, two repos → `None`.
  - `stale_readiness_result_is_ignored`: open the sheet (generation 1), reopen it (generation 2), then deliver generation 1's Connected result. Readiness stays Checking and Turn on stays disabled.
  - `results_menu_respects_harness`: with a Pi model chosen, the Results menu offers only "keeps the review in Gespan". Switching from Claude with Post to a Pi model resets Results to Gespan.
  - `turn_on_saves_the_shown_values`: drive a `TestAppContext` page (the pattern in `layout_tests.rs`) with a stubbed Connected status. Pick "is opened, drafts too" and Post, then click Turn on. The service holds one automation equal to `review_draft` of the same input, enabled, and it is selected in the list.
  - `existing_review_automation_opens_in_editor`: with a review automation already saved for the project, clicking the card opens the editor on it, and the count stays 1.
  - `customize_carries_the_draft`: Customize opens the editor with the sheet's draft and does not save.
  - `sheet_layout_fits`: the sheet lays out without horizontal overflow at the narrow and wide widths `layout_tests.rs` already uses.

- [ ] **Step 2: Run to verify they fail.** Run: `cd crates/gespan-gpui && cargo test review_setup`. Expected: FAIL.

- [ ] **Step 3: Implement**
  - Readiness loads off the UI thread in one background task: status, repos, `is_remote`. It is tagged with `generation`.
  - The inline phrases reuse the existing `ChoiceMenu`/`menu_popover` pattern. Each phrase is a `Role::Button` with an `aria_label` like "Results: keeps the review in Gespan".
  - The model phrase reuses `ModelList`.
  - "Get GitHub CLI" opens `https://cli.github.com` with the app's existing open-URL helper.
  - Turn on calls the same save path as the editor, then `select(id)`.

- [ ] **Step 4: Run to verify they pass.** Run: `cd crates/gespan-gpui && cargo test review_setup && cargo test automations`. Expected: pass, with `contract_tests.rs` still green. Every template trigger must still be fireable.

- [ ] **Step 5: Commit** with message `Add the two-click review setup sheet`.

---

### Task 10: The editor shows read-only and Results

**Files:**
- Modify: `crates/gespan-gpui/src/automations/editor.rs`:
  - add `("read-only", "Read-only")` as the first entry of the Access options (:334-337)
  - add a Results row in the Session section (:1009 onward), rendered only when `draft.review_results.is_some()`, using a new `Choice::Results` variant in `automations.rs`
- Test: `crates/gespan-gpui/src/automations/layout_tests.rs`.

- [ ] **Step 1: Write the failing tests**
  - `editor_offers_read_only_access`: the Access menu lists Read-only, and choosing it saves `runtime_mode == "read-only"`.
  - `results_row_only_for_review_automations`: a review automation shows a Results row, and changing it to Draft saves `Some(Draft)`. A non-review automation has no Results row.
  - `results_row_respects_harness`: for a Pi automation the row offers only Gespan.

- [ ] **Step 2: Run to verify they fail.** Run: `cd crates/gespan-gpui && cargo test editor_offers_read_only results_row`. Expected: FAIL.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run to verify they pass.** Run: `cd crates/gespan-gpui && cargo test automations`. Expected: pass.

- [ ] **Step 5: Commit** with message `Show read-only access and review results in the automation editor`.

---

### Task 11: End-to-end check

- [ ] **Step 1:** Run both full suites:
  - `cd crates/gespan-core && cargo test`
  - `cd crates/gespan-gpui && cargo test`
  - `cargo clippy --all-targets -- -D warnings` in both crates, if CI runs it (check `.github/workflows`)

  Expected: all green.
- [ ] **Step 2:** Launch the app with a stub Claude (`GESPAN_CLAUDE_BINARY`, see the PR video memory notes) and a stub `gh` on `PATH` that reports signed in and returns a PR item with a review request.
  - Open Automations → Review pull requests and check the sentence and readiness line.
  - Click Turn on and confirm the automation appears enabled.
  - Trigger an Inbox refresh and confirm a run starts in read-only mode.
  - Under Draft, confirm the post shows as an approval notice with the review text.
- [ ] **Step 3:** Record the PR video of Step 2, following the repo's PR description format.
