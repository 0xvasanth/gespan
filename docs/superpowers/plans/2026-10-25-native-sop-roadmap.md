# Native SOP Roadmap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A "SOP Roadmap" tab in a Pilot's Overview draws its Flows as a live node-and-arrow diagram with per-run status, and lets the user ask questions or request changes that the Pilot applies and the diagram reflects.

**Architecture:**
- **Core:** `gespan-core` gets a pure `flow::diagram` module. It builds the graph from a `Flow`, lays it out deterministically top to bottom, routes edges, and derives a run overlay. There are no GPUI types in it.
- **Widget:** `gespan-gpui` gets a reusable diagram widget (canvas painting, viewport, hit-testing, keyboard).
- **Tab:** a Roadmap tab in the Pilot Overview loads data off-thread and refreshes on a new "flows changed" notification from `Pilots`.
- **Asking:** an input in the tab sends a visible message to the Pilot's chat.

**Tech Stack:** Rust, GPUI (vendored `gpui-pre`), `gespan-core`, cargo `native-dev` profile. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-25-sop-roadmap-design.md`. Read its "Decisions settled after review" section; it is binding.

## Global Constraints

- Native app only. No new crates. Pilot Flows stay out of the global Flows view (`flows/view.rs` filter unchanged).
- Changes reach Flows only through the existing `flows.*` actions; the tab never writes the store.
- The layout is deterministic: the same `Flow` always gives identical coordinates. Status-only updates never re-run the layout.
- Context line sent to chat, exactly: `[SOP Roadmap · Flow "<name>" (<flowId>) · run <runId|blueprint> · step "<stateId>"]`. The `· step …` part appears only when a node is selected. It goes on its own last line of a visible (`internal: false`) user message.
- Queued notice copy, exactly: `Queued · the Pilot will pick this up when it's free`.
- Empty state copy, exactly: `This Pilot has no Flows yet. Ask it to create one below.`
- Input placeholder, exactly: `Ask about this roadmap or ask for a change…`.
- Tab label: `SOP Roadmap`. Zoom limits are 0.4 to 2.0. The "changed" badge lasts 10 s. Guard labels are truncated to 24 characters with `…`.
- Tests: `cargo test --locked --manifest-path crates/gespan-gpui/Cargo.toml -p <crate> --profile native-dev -- --test-threads=1 <filter>`. Clippy `-D warnings`, `cargo fmt`, and `bun run native:test` at the end.
- GPUI test hazard: a message reaching an idle Pilot chat starts a real provider turn and hangs the test. Use the existing holds in `pilots/tests.rs`. `timeout` isn't installed; use `perl -e 'alarm N; exec @ARGV' …`.
- Name: Gespan only (`src/test/legacyNames.test.ts`). Commits only when the user says so, and never with Claude attribution.
- Branch: `feat/native-sop-roadmap`, based on the Pilot Flows branch (PR #166), since this depends on Pilot Flows.

## Review Focus

1. **A Flow with an unreachable state or a state with no transitions** still draws, and nothing panics. Unreachable states sit in a trailing rank. *(Task 1 test.)*
2. **A self-loop (`a → a`) and two parallel edges `a → b`** are both visible and clickable, with separate label anchors. *(Task 1 test.)*
3. **The selected run is deleted, or its Flow is deleted, while the tab is open.** The tab falls back to "Blueprint only" or the empty state without a stale diagram. *(Task 4 test.)*
4. **A rejected `flows.*` call** (validation error, foreign id) does not fire "flows changed" and does not show a "changed" badge. *(Task 5 test.)*
5. **The user zooms or pans, then a status update arrives.** The viewport is not reset, and auto-follow stays off until "re-centre". *(Task 3 test.)*

---

### Task 1: Diagram model and layout (core)

**Files:**
- Create: `crates/gespan-core/src/flow/diagram.rs`; register `pub mod diagram;` in `flow/mod.rs`.

**Interfaces (produces):**
```rust
pub enum NodeKind { Start, Agent, Approval, Succeeded, Failed }
pub struct DiagramNode { pub id: String, pub kind: NodeKind, pub title: String, pub rank: usize, pub order: usize, pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
pub struct DiagramEdge { pub from: String, pub to: String, pub label: Option<String>, pub full_label: Option<String>,
    pub back: bool, pub implicit: bool, pub points: Vec<(f32, f32)>, pub label_at: (f32, f32) }
// `points`: cubic bezier control points [p0, c1, c2, p1] for forward edges; polyline vertices for back edges.
pub struct Diagram { pub nodes: Vec<DiagramNode>, pub edges: Vec<DiagramEdge>, pub width: f32, pub height: f32 }
pub const NODE_W: f32 = 200.0; pub const NODE_H: f32 = 56.0; pub const RANK_GAP: f32 = 72.0; pub const NODE_GAP: f32 = 32.0;
pub const START_ID: &str = "$start"; // terminal ids reuse `$succeeded` / `$failed`
pub fn guard_label(guard: &Option<Guard>, is_fallback: bool) -> Option<String>; // spec decision 9 wording; None for a plain unguarded single edge
pub fn build(flow: &Flow) -> Diagram;
pub fn structure_key(flow: &Flow) -> u64; // hash of state ids, kinds and transition (from,to) pairs: decides re-layout
```

**Algorithm decisions** (not determined by the signatures):
- **Back-edges:** DFS from `initial`, visiting transitions in declared order; an edge to a node on the DFS stack is a back-edge. A self-loop is a back-edge.
- **Ranks:** longest path from Start over forward edges. Start is rank 0, `initial` is rank 1. Terminals go one below the deepest rank that targets them. States unreachable from `initial` go in a trailing rank, in declared order.
- **Order within a rank:** start from declared order, then do 4 barycenter sweeps (down, up, down, up). Break ties by declared order.
- **Coordinates:** each rank row is centred on the widest rank; `y = rank * (NODE_H + RANK_GAP)`. Terminal pills and Start are `NODE_H/2` tall.
- **Forward edges:** bottom-centre to top-centre, with control points offset vertically by half the gap. Parallel edges between the same pair get x offsets of ±12 per extra edge.
- **Back-edges:** exit the source's right edge, go into a lane `24 * lane_index` px to the right of the widest node in the ranks spanned, run up, and enter the target's right edge.
- **Labels:** a guarded edge followed by an unguarded sibling gets the label "otherwise" on the unguarded one. Labels are truncated to 24 chars + `…`; `full_label` keeps the original.

- [ ] **Step 1: Write the failing tests** in `diagram.rs`:
  - `linear_flow_ranks_top_to_bottom`: start → a → b → `$succeeded`. Ranks are 0, 1, 2, 3, and `y` increases.
  - `retry_loop_is_a_back_edge_on_the_side`: test → implement on failure. That edge has `back == true`, and its polyline's max x exceeds every node's right edge.
  - `self_loop_and_parallel_edges_are_distinct`: `a → a`, plus two edges `a → b` with different guards. Three edges, distinct `label_at`.
  - `unreachable_and_dead_end_states_still_draw`: Review Focus 1.
  - `layout_is_deterministic`: calling `build` twice on the same Flow gives equal `Diagram`s, and so does building it from a serde round-tripped clone.
  - `guard_labels_follow_the_wording`: every case of decision 9, plus a 40-char value truncated to 24 + `…` with `full_label` intact.
  - `structure_key_ignores_briefs_but_not_transitions`.
- [ ] **Step 2: Run** `… -p gespan-core … flow::diagram`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run.** Expected: PASS. Clippy and fmt are clean.
- [ ] **Step 5: Stage.**

### Task 2: Run overlay (core)

**Files:**
- Modify: `crates/gespan-core/src/flow/diagram.rs` (same module, new section)

**Interfaces:**
- Consumes: `contract::select_transition(state, VisitResult, &HashMap<String, OutputValue>) -> Option<&Transition>` (already `pub`), `FlowRun`, `FlowVisit`, `RunEdit`.
- Produces:
```rust
pub enum NodeStatus { NotReached, Running, AwaitingApproval, Completed, Failed, Cancelled }
pub struct Overlay { pub nodes: HashMap<String, (NodeStatus, u32 /*visits*/)>, pub current: Option<String>,
    pub taken: HashSet<(String, String, usize /*edge index among same pair*/)>, pub ended_at: Option<String /*$succeeded|$failed*/>, pub implicit_fail: bool }
pub fn overlay(run: &FlowRun, visits: &[FlowVisit]) -> Overlay;
pub fn changed_states(before: Option<&Flow>, after: &Flow, edits: &[RunEdit]) -> HashSet<String>;
// state ids whose FlowState differs (any field) or that are new, plus the ids in every RunEdit.states
```
- Taken edges: for each completed or failed visit, call `select_transition(state, result, outputs)` on the run snapshot. Mark that transition's (from, to, index) as taken. Skip visits with no outputs. A Failed run whose last visit selected no transition sets `implicit_fail = true`, and Task 3 draws a dashed implicit edge to the Failed pill.

- [ ] **Step 1: Write the failing tests:**
  - `overlay_marks_latest_status_and_visit_counts` (a node visited twice: counts 2, status from the latest visit).
  - `taken_edge_matches_the_runners_choice_between_parallel_edges`.
  - `failed_run_without_failed_transition_sets_implicit_fail`.
  - `interrupted_visit_without_outputs_highlights_nothing`.
  - `changed_states_from_diff_and_run_edits`.
- [ ] **Step 2: Run** `… flow::diagram`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Stage.**

### Task 3: Diagram widget (GPUI)

**Files:**
- Create: `crates/gespan-gpui/src/flows/diagram.rs` (declare it in `flows/mod.rs`; make it `pub(crate)` for `pilots/`)
- Create: `crates/gespan-gpui/src/flows/diagram/tests.rs`

**Interfaces (produces):**
```rust
pub(crate) struct Viewport { pub x: f32, pub y: f32, pub zoom: f32 }
impl Viewport { pub fn fit(content: Size<f32>, area: Size<f32>, padding: f32) -> Self; pub fn zoom_at(&mut self, factor: f32, pointer: Point<f32>); // clamps 0.4..=2.0
    pub fn to_screen(&self, p: (f32, f32)) -> Point<f32>; pub fn to_flow(&self, p: Point<f32>) -> (f32, f32); }
pub(crate) enum Hit { Node(String), Edge(usize) }
pub(crate) fn hit_test(d: &Diagram, flow_point: (f32, f32), tolerance: f32) -> Option<Hit>; // nodes win over edges; edge tolerance 8px / zoom, sampled at 24 points per curve
pub(crate) struct DiagramView { /* Entity */ }
impl DiagramView {
    pub fn new(cx: &mut Context<Self>) -> Self;
    pub fn set(&mut self, diagram: Diagram, overlay: Option<Overlay>, changed: HashSet<String>, structure: u64, cx: &mut Context<Self>); // re-fits only when `structure` changed
    pub fn selected(&self) -> Option<&str>;
}
pub(crate) enum DiagramEvent { Selected(Option<String>), Open(String) }
```

**Painting:**
- `canvas()` with `PathBuilder::cubic_bezier_to` for forward edges.
- Back-edges and implicit edges are dashed: draw alternating 6-px segments.
- Arrowheads are filled triangles.
- Nodes use `paint_quad` (radius 8), with a border colour per `NodeStatus` from existing theme status tokens. A Running node pulses by animating the border alpha.
- A visit-count badge appears when the count is above 1, and a "changed" badge shows for 10 s.
- Labels use `text_system().shape_line`. Titles wrap to 2 lines, then `…`.
- Taken edges use the accent colour; others are muted.
- The terminal pill named by `overlay.ended_at` is lit (green Done / red Failed).
- With `implicit_fail`, draw a dashed implicit edge from the last visited node to Failed.
- Start never shows a status.

**Interaction:**
- Drag pans. Cmd/Ctrl+wheel or pinch zooms at the pointer (copy the `image_lightbox.rs` approach).
- Click selects.
- Keyboard: Tab or arrow keys move focus along edges, Enter emits `Open`, F fits, +/− zoom, Esc clears.
- The element has a role and an aria label, which announces the focused node: `"<title>, <status>"`.
- Auto-follow: on `set` with a new `current`, pan it into view unless the user panned since the last re-centre. A "Re-centre" button resets that.

- [ ] **Step 1: Write the failing tests** (pure math, no window):
  - `fit_centres_content_and_respects_zoom_limits`.
  - `zoom_at_keeps_the_pointer_fixed`.
  - `hit_test_prefers_nodes_and_finds_edges_within_tolerance`.
  - `status_update_keeps_the_viewport` (Review Focus 5: call `set` with the same `structure` after a manual pan; the viewport is unchanged and following stays off).
  - `structure_change_refits`.
- [ ] **Step 2: Run** `… -p gespan-gpui … flows::diagram`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run.** Expected: PASS. Then take a manual visual check with `bun run native:dev` and a fixture Flow, and save a screenshot in the report.
- [ ] **Step 5: Stage.**

### Task 4: SOP Roadmap tab

**Files:**
- Create: `crates/gespan-gpui/src/pilots/roadmap.rs` (declare `mod roadmap;` in `pilots/mod.rs`)
- Modify: `crates/gespan-gpui/src/pilots/panels.rs`:
  - `OverviewTab` (≈10): add `Roadmap`;
  - the tab array (≈814): add `(OverviewTab::Roadmap, "SOP Roadmap", <icon>, None)`;
  - the body dispatch (≈912);
  - the id match (≈854): `"pilot-overview-roadmap"`.
- Test: `crates/gespan-gpui/src/pilots/tests.rs`

**Interfaces:**
- Consumes: Task 1/2 `build`, `overlay`, `changed_states`, `structure_key`; Task 3 `DiagramView`, `DiagramEvent`. Store helpers `FlowStore::owned`, `owned_flow`, `owned_run`, `owned_runs` (via `Shell::pilot_flows_store()`).
- Produces:
```rust
pub(super) struct Roadmap { pilot: String, flow: Option<String>, run: Option<String> /*None = blueprint*/, generation: u64, /* … */ }
impl Roadmap {
    pub fn new(pilot: String, owner: WeakEntity<Shell>, cx: &mut Context<Self>) -> Self;
    pub fn reload(&mut self, cx: &mut Context<Self>); // off-thread; drops stale results by generation
    pub fn selection_context(&self) -> Option<String>; // the exact context line from Global Constraints
}
```
- One `Entity<Roadmap>` per Pilot, kept on `Pilots` next to `overview_tab`. `reload` runs when the tab becomes visible, and on `tick_flows` while it is visible.
- Pickers:
  - **Flow:** `owned(pilot)`, newest `updatedAt` first.
  - **Run:** "Blueprint only", then `owned_runs(pilot, None, 50)` for the selected Flow, labelled `"<task> · <goal>"`.
- **Drawn definition:** the run's `run.flow` when a run is selected, else the saved Flow (decision 1).
- **Node details drawer** (on `Open`): title, the full brief, outputs, every transition with `full_label`, and the visit list (seq, status, duration). "Open session" reuses the existing session-open action for `visit.session_id`.

- [ ] **Step 1: Write the failing tests:**
  - `roadmap_tab_is_listed_and_selectable`.
  - `roadmap_shows_the_empty_state_without_flows` (exact copy).
  - `roadmap_lists_only_this_pilots_flows_and_runs`.
  - `selected_run_draws_its_snapshot_not_the_blueprint` (after `flows.editRun` adds a state, the run view has it and the blueprint view doesn't).
  - `deleted_run_or_flow_falls_back_cleanly` (Review Focus 3).
  - Use draft-free Flows whose launches are held (`held_launches`).
- [ ] **Step 2: Run** `… pilots::tests::roadmap`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Stage.**

### Task 5: Live updates and the "changed" badge

**Files:**
- Modify: `crates/gespan-gpui/src/pilots/host.rs` (after a successful `flows.save|delete|start|editRun|cancel|resume`, call `self.flows_changed(&pilot_id, cx)`)
- Modify: `crates/gespan-gpui/src/pilots/mod.rs` (add `fn flows_changed(&mut self, pilot: &str, cx)`, which reloads that Pilot's `Roadmap` if it exists and is visible, else marks it dirty)
- Modify: `crates/gespan-gpui/src/pilots/flow_reviews.rs` (`flow_event`: also reload the visible Roadmap whose run matches, status-only)
- Modify: `crates/gespan-gpui/src/pilots/roadmap.rs` (keep `last_loaded: Option<Flow>` per Flow id and pass it to `changed_states`; re-layout only when `structure_key` differs)

- [ ] **Step 1: Write the failing tests:**
  - `flows_save_updates_an_open_roadmap_and_badges_the_changed_state`.
  - `edit_run_badges_the_runs_edited_states`.
  - `a_step_event_repaints_status_without_relayout` (the `structure_key` is unchanged and `set` doesn't refit).
  - `rejected_flows_calls_do_not_fire_flows_changed` (Review Focus 4: a validation error and a foreign id).
- [ ] **Step 2: Run** `… pilots::tests::roadmap`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Stage.**

### Task 6: Ask or change from the tab, and the Pilot's instructions

**Files:**
- Modify: `crates/gespan-gpui/src/pilots/roadmap.rs` (an input at the bottom using the `quick_composer.rs` `input::Input` + `Submitted` pattern; the exact placeholder)
- Modify: `crates/gespan-gpui/src/pilots/mod.rs` (a send path: `enqueue(pilot, format!("{text}\n{context}"), vec![], /*internal*/ false, cx)` + `ensure_chat`; track the returned message id until it is delivered, to show the queued notice)
- Modify: `crates/gespan-core/src/pilot.rs` `context` (add the paragraph below after the Flow paragraph)
- Test: `crates/gespan-gpui/src/pilots/tests.rs`, `crates/gespan-core/src/pilot/tests.rs`

Prompt paragraph, exactly:

`"Messages whose last line starts with [SOP Roadmap · come from the Roadmap tab and name the Flow, run and step the user is looking at. Answer questions about them from flows.read and flows.run. A change request there is the user's confirmation: use flows.editRun when they mean only this task, otherwise flows.save for the blueprint; say which you changed, record it in memory, and if it cannot be applied, say why."`

- [ ] **Step 1: Write the failing tests:**
  - `roadmap_message_is_visible_with_the_exact_context_line` (with and without a selected step).
  - `roadmap_message_to_a_busy_or_paused_pilot_queues_and_shows_the_notice` (exact copy; the notice clears after delivery).
  - Core: `context_explains_roadmap_requests` (asserts the exact paragraph and that the Flow paragraph is still present).
- [ ] **Step 2: Run** `… pilots::tests::roadmap_message` and `… -p gespan-core … pilot::tests::context_explains_roadmap`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Run the gates:** clippy, `cargo fmt --check`, `bun run native:test`, `bunx vitest run src/test/legacyNames.test.ts`. Expected: all green.
- [ ] **Step 6: Stage.** Report for commit approval.
