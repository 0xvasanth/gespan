# SOP Roadmap: see and steer a Pilot's Flows as a diagram — Design

Status: draft (2026-10-25). The user asked for a design and plan without a question round.
Everything marked **(my call)** was decided here and is open to correction.

## Why

Pilots now run work through Pilot Flows (see `2026-10-25-pilot-flows-design.md`).
A Flow is a state machine (steps, transitions, guards, loops), and today nobody
can *see* one. Pilot Flows are hidden from the Flows view, and even user Flows only
show as JSON plus a list of visits.

The user wants a picture of how the steps connect and where each task stands, in
the style of React Flow diagrams. They also want to talk to it: ask a question
about the procedure, or ask for a change, and see the page react.

This is the "Procedures (SOP) view" that the Pilot Flows design captured as a
future idea. It is now being built.

## What it is

A new **SOP Roadmap** tab in the right-hand **Overview** panel of a Pilot (next to
Sessions, Habits, Documents, Changes, Activity). It has three parts:

1. **The diagram.** The Pilot's Flow drawn as boxes and arrows, top to bottom.
   - Steps are boxes and transitions are arrows.
   - Arrows carry their condition (e.g. `tests = failed`).
   - Loops back to an earlier step are drawn as dashed arrows along the side.
   - The end states, **Done** and **Failed**, are small pills.
2. **Run status on top.** Pick a task run (or "Blueprint only") and each box shows
   where that task stands:
   - not reached, running (pulsing), completed, failed or cancelled;
   - how many times the step ran;
   - the arrows the run actually took, highlighted.

   Clicking a box opens its details: the brief, outputs, conditions, every visit
   with its session, and "Open session".
3. **Ask or change.** An input at the bottom of the tab: *"Ask about this roadmap
   or ask for a change…"*.
   - **The message goes to the Pilot's own chat,** prefixed with what you're looking
     at (the Flow, the run, the selected step). The Pilot answers in its chat.
   - **Asked for a change**, the Pilot makes it with the existing actions:
     - `flows.save` changes the blueprint, for future tasks;
     - `flows.editRun` changes one running task's remaining steps.
   - **The diagram updates live.** Changed boxes are briefly marked "changed".
     If the change can't be applied (e.g. the step already ran), the Pilot says so
     in chat.

## Visibility change (deliberate)

The Pilot Flows design hid Pilot Flows completely. This design shows a Pilot's Flows
**read-only, inside that Pilot's own Overview**.
- They stay out of the global Flows view.
- They still can't be edited directly; changes go through the Pilot.

**(my call: the user asked for this view; the "hidden" rule was "for now".)**

## Goals

1. See any of the Pilot's Flows as a clear, readable diagram in a narrow right panel.
2. See each task run's live progress on the same diagram.
3. Ask questions or request changes from the tab; changes show up without reloading.
4. Feel like React Flow where it matters:
   - smooth pan and zoom, centred on the pointer;
   - fit to view;
   - a clear focus/selection state;
   - status colours you can read at a glance.

## Non-goals (this version)

- Editing by dragging (adding, connecting or moving boxes).
  Changes go through the Pilot only.
- A minimap. Flows are ≤ 30 states, so fit-to-view and pan are enough.
- Showing the diagram in the global Flows view for user Flows. The widget is
  reusable, so it can follow later.
- Tauri app support (the native app first, as with Pilot Flows).
- Sub-flows.

## Anti-goals

- **Layout jumping.** The layout must not jump when only run status changes.
  Status never triggers a re-layout.
- **Stale diagram.** It must not keep showing an old Flow after the Pilot changed it.
- **Lost requests.** A request typed in the tab must not be lost because the Pilot
  is busy. It queues like any other chat message.
- **Inventing a second edit path.** All changes go through the same `flows.*`
  actions, with the same validation and ownership checks.

## Research: React Flow, and what we take from it

React Flow (xyflow) is a web library: HTML boxes, SVG arrows, and the browser does
the hit-testing and accessibility. **We can't use it in the native GPUI app**, but we
copy its model:

- **Data model:**
  - **The domain owns the data.** The diagram only *reads* the `Flow`/run and
    emits UI events.
  - **Three separate layers:** data, layout positions, and one viewport transform
    `{x, y, zoom}`.
- **Interaction vocabulary:**
  - pan by drag;
  - zoom at the pointer with Cmd/Ctrl + wheel or pinch;
  - fit view, with zoom limits;
  - click to select;
  - wide invisible hit areas on arrows (thin lines are hard to click).
- **Layout:** React Flow does none; people use dagre or ELK. Both are *layered*
  layouts (Sugiyama):
  1. break cycles by reversing back-edges;
  2. assign ranks;
  3. reduce crossings;
  4. assign coordinates.

  Loops are drawn as reversed edges routed along the side.
- **Workflow-builder status patterns (n8n, Langflow):**
  - a status ring or border per box (grey idle, accent + pulse running, green done,
    red failed);
  - the taken path highlighted;
  - condition labels on branch arrows;
  - loops drawn dashed with a counter;
  - auto-follow the running step until the user pans.

Rust layout crates exist: `rust-sugiyama` (MIT, the most complete) and young
`dagre` ports. None of them route arrows. Flows are capped at 30 states
(`MAX_STATES`), so **(my call)** we write a small, deterministic layered layout in
`gespan-core` instead of adding a dependency. It is easy to test, and the output is
stable (the same Flow always gives the same picture).

## Options considered

| Option | Verdict |
|---|---|
| **Native GPUI diagram widget (chosen)** | Live data, native speed, full control of status/selection/keyboard. We build layout, routing, hit-testing and accessibility ourselves. |
| React Flow inside the existing HTML sandbox (WKWebView used for artifacts) | Rejected. The sandbox has no bridge to the app by design: no live updates, and clicks can't reach the Pilot. It's macOS-only and a second rendering stack. |
| The native "visual" SDK (`gespan_visual` canvas) | Rejected. Its canvas has no curves, arrowheads or click targets. It runs as a sandboxed program with no store access, so it can't update live. |
| Text (Mermaid-like) rendered to an image | Rejected. No interaction and no live status. |

## What's good and what isn't

**Positives**
- **Readable at a glance:** the shape of the procedure, its branches and loops, and
  exactly where each task is.
- **Live:** the Pilot's edits and run progress show up as they happen, with no
  reload.
- **One edit path:** chat → Pilot → `flows.*`. It reuses the ownership and project
  checks and the edit rules (only steps not yet entered can change), and the Pilot
  records changes in memory as its prompt already requires.
- **Native and fast:** ≤ 30 boxes is trivial to draw every frame.
- **Reusable:** the diagram widget can later serve user Flows in the Flows view.
- **Deterministic layout:** it can be unit-tested; the same Flow gives the same picture.

**Negatives and risks**
- **We pay for what the browser gives React Flow free:**
  - layout;
  - arrow routing;
  - hit-testing;
  - focus order;
  - screen-reader semantics.

  Mitigated by small scope (≤ 30 nodes, read-only).
- **Narrow panel.** The right panel is narrow, so wide Flows need pan/zoom.
  Mitigated by a top-to-bottom layout and fit-to-view on open.
- **Chat-mediated edits are slower and less direct** than dragging, and depend on
  the Pilot understanding the request. A busy Pilot queues the request.
  Mitigated by:
  - sending precise context (Flow id, run id, selected step);
  - the Pilot replying in chat when a change can't be applied.
- **Text measurement before layout** (box sizes depend on titles). Mitigated by
  fixed-width boxes with titles truncated to two lines (my call).
- **Showing Pilot Flows** reverses the earlier "hidden" decision. It's read-only and
  per Pilot, but it's still a visible behaviour change.
- **Live updates need new plumbing.** Today `flows.save` notifies nothing
  (`pilots/host.rs`, `call_work` with `Effect::None`). A change notification must
  be added.

## How it works

### Diagram model (core, pure: `crates/gespan-core/src/flow/diagram.rs`)
- **Nodes:**
  - **Kinds:** one node per state (`Agent`, `Approval`), plus a **Start** marker
    pointing at `initial`, and **Done** / **Failed** terminal pills for `$succeeded`
    / `$failed` (only if some transition targets them).
  - **Transition labels:**
    - a transition with a guard is labelled from the guard (`field = value`,
      `status = failed`, `field is set`, …);
    - an unguarded transition after guarded siblings is labelled "otherwise".
- **Back-edges:** found by a depth-first search from `initial` in declared state
  order. The search is deterministic.
- **Layout:** layered, top to bottom.
  1. **Ranks:** the longest path from Start over forward edges.
  2. **Order within a rank:** start with declared order, then a fixed number of
     barycenter sweeps.
  3. **Coordinates:** fixed node size and gaps, centred per rank.
  4. **Unreachable states:** placed after the reachable ones, in their own rank.
- **Edge geometry:**
  - forward edges are a cubic bezier from the bottom of the source to the top of
    the target;
  - back-edges run as an orthogonal-ish path out the right side, up a side lane,
    and into the target's right side;
  - parallel edges get offsets;
  - each edge gets a label anchor point.
- **Output:** a plain struct (`Diagram { nodes, edges, bounds }`) in flow
  coordinates. It contains no GPUI types.

### Run overlay (core, pure)
- **Per state**, from a run's visits:
  - status: the latest visit's status, else "not reached";
  - visit count;
  - whether it is the run's `current_state`.
- **Per edge:** whether the run took it, meaning consecutive visits `a → b`, or
  the last visit → a terminal state.
- **Changed states:** from the run's `RunEdit.states` (run edits), or from a diff
  of two Flow definitions (blueprint edits).

### Widget (GPUI: `crates/gespan-gpui/src/flows/diagram.rs`)
- **Drawing:** a `canvas()` paints:
  - edges with `PathBuilder` (cubic bezier, dashed back-edges via segments), with
    arrowheads;
  - label chips;
  - nodes with `paint_quad` (rounded, status border, pulse for running);
  - text via `text_system().shape_line`.

  Zoom is applied by scaling coordinates; there is no canvas transform in GPUI.
- **Viewport `{x, y, zoom}`:**
  - drag to pan;
  - Cmd/Ctrl+wheel and pinch to zoom at the pointer (as in `image_lightbox.rs`);
  - zoom limits 0.4–2.0;
  - fit-to-view on first show and when the Flow changes structure.
- **Hit-testing:** node rectangles; edges within 8 px of the path, sampled.
- **Selection and keyboard:**
  - Tab or arrow keys move focus between nodes along edges;
  - Enter opens details;
  - F fits; + and − zoom; Esc clears the selection.
- **Accessibility:** the widget is a focusable element with a role and a label. The
  focused node's title and status are announced.
- **Auto-follow:** the view keeps the running node in view until the user pans
  manually. A "re-centre" button turns following back on.

### Roadmap tab (GPUI: `crates/gespan-gpui/src/pilots/roadmap.rs`)
- A new `OverviewTab::Roadmap`, labelled "SOP Roadmap".
- **Pickers at the top:**
  - **Flow:** the Pilot's own Flows, newest first;
  - **Run:** "Blueprint only", then this Flow's runs grouped by goal (task names).
- **The diagram** fills the tab; a node details drawer slides up when a node is
  selected.
- **Empty state:** "This Pilot has no Flows yet. Ask it to create one below."
- **The "Ask or change" input** sits at the bottom.

### Ask or change
- The input sends a user message to the Pilot's main chat. Through the normal
  queue it is steered if the chat is busy, or queued; it is never dropped.
- The message text is the user's words plus a short context line, exactly:
  `[SOP Roadmap · Flow "<name>" (<flowId>) · run <runId or "blueprint"> · step "<stateId>"]`.
  The step is included only if a node is selected.
- **Prompt addition:** messages from the SOP Roadmap name the Flow, run and step
  the user is looking at.
  - **Answering questions:** answer from `flows.read`/`flows.run`.
  - **Change requests:** the request itself is the user's confirmation.
    - "For this task" → `flows.editRun`;
    - "from now on" or unspecified → `flows.save` (blueprint);
    - say which was done;
    - record the change in memory;
    - if a change can't be applied, explain why.

### Live updates
- After any successful `flows.save`, `flows.delete`, `flows.editRun`,
  `flows.cancel`, `flows.resume` or `flows.start`, the Pilots entity emits a
  per-Pilot "flows changed" notification. Existing step and finish events already
  arrive through `Pilots::flow_event`.
- The Roadmap reloads the selected Flow and run off-thread, using a generation
  counter (as `FlowsView::refresh` does), and repaints.
- **Re-layout** happens only when the Flow's structure changed; status-only
  updates repaint in place.
- **Change highlight:** changed nodes get a "changed" badge for 10 s.

## Constraints

- Native app only, built on `gespan-core` (pure model and layout) and `gespan-gpui`
  (widget and tab).
- No new dependencies.
- Reuse the `flows.*` actions, ownership checks and edit rules unchanged.
- **Pilot Flows stay out of the global Flows view.**
- **Tests:**
  - layout determinism;
  - back-edge detection;
  - overlay derivation;
  - hit-testing math;
  - tab wiring;
  - the live update after `flows.save`/`editRun`;
  - the context line sent to the chat.
- **Project rules:**
  - cargo `native-dev` profile, clippy `-D warnings`, `cargo fmt`;
  - use only the name Gespan in code and docs;
  - no Claude attribution in commits/PRs.

## Decisions settled after review (all my calls)

1. **Which definition is drawn.**
   - With a run selected, the diagram draws **that run's snapshot** (`run.flow`),
     which is what the run actually executes.
   - With "Blueprint only", it draws the current saved Flow.
   - Switching between them may re-layout when their structures differ. The
     no-jump rule is about *status* updates, and status never re-lays out.
2. **Taken edges.**
   - Expose the runner's own transition choice (`select_transition` in
     `flow/contract.rs`) as a pure function, and re-evaluate it on each visit's
     status and outputs. That way the highlighted arrow is exactly the one the
     runner took.
   - If outputs are missing (e.g. an interrupted visit), highlight nothing for
     that step.
   - A run that ended Failed with no `$failed` transition gets a dashed implicit
     arrow into the Failed pill.
3. **The message in chat.**
   - The message is a normal, visible user message (`internal: false`), so the
     user sees what they asked.
   - The context line is appended on its own last line.
4. **A Pilot that can't take the message now** (busy, paused, no chat open yet):
   - The input stays enabled and the message is queued through the normal outbox.
   - The tab shows "Queued · the Pilot will pick this up when it's free" until
     it's delivered.
5. **Where "flows changed" fires.** In `Pilots`, once, after any *successful*
   `flows.*` mutating action returns (save, delete, start, editRun, cancel, resume).
   - Failed calls stay silent.
   - Run progress keeps coming through `Pilots::flow_event`.
6. **Staying fresh.** On top of the change notification, the tab reloads:
   - when it becomes visible;
   - on the Pilots 10-second tick while it is visible.

   This covers edits from other paths (e.g. recovery after a restart).
7. **"Changed" badge.**
   - **Run snapshots:** the badge comes from `RunEdit.states`.
   - **Blueprints:** it comes from a diff against the copy the tab last loaded, so
     edits made before the roadmap was first opened show no badge; edits made
     while it was hidden are badged when it is shown again.
   - A state counts as changed if anything in it differs (brief, outputs,
     transitions, limits).
8. **Statuses.**
   - The overlay status set is: not reached, running, awaiting approval (amber,
     used only by user Flows later), completed, failed, cancelled.
   - Start has no status.
   - A terminal pill lights up when the run ended there.
9. **Labels.**
   - **Guard labels:**
     - `<field> = <value>`, `<field> ≠ <value>`;
     - `<field> is set` / `is not set`;
     - `<field> contains <value>`;
     - `status = <completed|failed>`;
     - "otherwise".
   - **Truncation:** labels are cut to 24 characters with "…". The full text
     shows in the node's details.

## Left to the builder

- Exact sizes, gaps and colours, using existing theme tokens and status colours
  where they exist.
- The number of barycenter sweeps and the edge sampling resolution.
- How the details drawer animates.
- Whether the Flow and run pickers are menus or segmented controls.

## Future

- The same diagram in the global Flows view for user Flows, editable by drag.
- A minimap, if Flows grow past 30 states.
- Tauri parity.
