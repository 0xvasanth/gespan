# First-run setup: design and implementation notes

Design canvas: https://claude.ai/artifact/LU1nZ6sAwGRHbQcePB6tn6 (boards: How setup works,
0. Welcome, 1. Coding agents, 2. Look, 3. Project, After setup, Resume/skip/no agents).

Builds on the sidebar redesign in PR 0xvasanth/gespan#419 (branch
`claude/global-project-structure`, read at aec34e5da). Land that first; the "after setup"
screen assumes its sidebar.

## Why

Gespan has no first-run experience. A new user lands in the shell with whatever folder the
app was launched from as their only project, no idea which agents Gespan found, and every
look setting at its default. Setup should get them to a working thread in their real project
in about a minute, and teach only the few things they'd otherwise go hunting for.

## Goals

- Three choices people want on day one: coding agents, look, project.
- Every card except Project can be skipped; skipping keeps the defaults.
- The project can't be skipped. The app opens into it from then on.
- Uses existing preferences. The only new state is "which card to resume on" and "done".

## Non-goals

- Not a copy of the Settings pages. The cards are purpose-built and show only top choices.
- No Inbox connections (GitHub, Linear, Jira), notifications, sidebar switches, first
  automation or sidebar tour. They keep their defaults and stay in Settings.
- No "about you" questions.

## Anti-goals

- Existing users must never see it, including those whose data came from the Tauri app.
- Setup must never block on agents: a machine with none installed can still finish.
- No emoji, sentence case, short plain wording, the app's own dark glass and colours.

## What exists today (main and #419)

- `crates/gespan-first-launch` is only an integration test: a release build opens the data a
  Tauri install left behind (`--gespan-data --check-data`) and creates none on a clean
  machine. It is not an onboarding flow and needs no change.
- No first-run, tour or tips code. The closest pattern is the What's new modal
  (`shell/notifications.rs`, `open_whats_new`, held in `notification_state.whats_new`,
  rendered from the shell's modal stack around `shell.rs` `render`), and the persisted
  "seen" flag `Workspace::new_layout_prompt_dismissed`.
- `Workspace::restore` always calls `add_project(cwd)` for the launch folder
  (`workspace.rs` ~line 787), so "the database has no projects" is almost never true.
- Workspace state is the `native_settings` row `workspace` (`gespan-core/src/store.rs`);
  `Preferences` lives inside it (`settings/preferences.rs`).
- Adding a project: `Shell::open_project` (folder picker, ⌘O) then `workspace.add_project`
  and `select_project`. "Open folder on a machine…" lives in `shell/remote_projects.rs`.
  No clone flow, no folder drag-and-drop for projects.
- Agents: `installed_providers` probes the 12 `Kind`s (Codex, Claude, Cursor, Grok, OpenCode,
  Antigravity, Pi, Omp, Fx, Hermes, Devin, Kiro). `accounts::login::signed_in` knows sign-in
  state only for Claude, Codex and Kiro; others return `None`. `Preferences::use_by_default`
  sets the default provider (Settings › Providers "Use by default").
- Look: `Preferences.theme` (system/dark/light), `Workspace.classic_layout`,
  `Preferences.accent_color` (Blue, Violet, Pink, Orange, Green presets in
  `settings/pages/appearance.rs`), `Preferences.inset_glass` / `body_glass`.
- Empty states already in the app: sidebar "No sessions yet" / "Start one with ⌘T"; new
  chat "What should we work on in {project}?"; composer hint "No coding agent detected. Set
  one up in Settings → Agent CLIs."

## The flow

One glass card over the dimmed app, with Back, Skip and Continue. Steps 1–3 show "Step N of 3",
three progress bars, and "Skip setup" top right.

**The card can't be closed.** It has no close button. Esc, clicking outside it and the
shell's own shortcuts do nothing while it's up. The only ways out are finishing the Project
card or quitting the app, and quitting resumes on the same card next launch. "Skip setup"
doesn't close it: it jumps to Project.

## Card spec (build it pixel for pixel from the canvas)

The implementation should match the canvas boards exactly. Values are dark theme; light
mirrors them with the app's tokens.

- **Scrim:** `rgba(0,0,0,0.35)` over the whole window. The shell stays visible and blurred
  behind it.
- **Card:**
  - 720 wide, height fits the content, centred.
  - Radius 24. Padding 28 top, 40 sides, 28 bottom (Welcome: 36 top). Gap 22 between blocks
    (Welcome: 28).
  - Fill `rgba(23,23,23,0.72)`, backdrop blur 24 with saturate 1.6.
  - Border 1 px `rgba(255,255,255,0.09)`.
  - Shadow: inset `0 1px 0 rgba(255,255,255,0.10)` and `0 24px 64px rgba(0,0,0,0.55)`.
- **Header row:**
  - "Step N of 3": 12 px, `#a3a3a3`.
  - Progress bars: three 28×4 pills, gap 4. Done or current is `#af97fa`; to come is
    `rgba(235,235,235,0.14)`.
  - "Skip setup" is right-aligned, 13 px, `#a3a3a3`.
- **Title:** Inter Tight 28 px (Welcome: 34), weight 500, tracking −0.025em. Lead text is
  14/22 `#a3a3a3` (Welcome: 16/25).
- **Footer:**
  - A 1 px `rgba(255,255,255,0.07)` rule with 16 px above it.
  - "Back" is plain text, `#a3a3a3`, on the left.
  - "Skip" is a ghost pill.
  - Primary is a pill: `#ebebeb` fill, `#111111` text at weight 600, padding 9×20, with the
    brand glow `0 0 0 1px rgba(175,151,250,0.35), 0 8px 32px rgba(143,100,246,0.45)`.
  - Disabled primary: `rgba(235,235,235,0.12)` fill and `#737373` text.
- **Selected option (agent row, layout card, project row):**
  - Fill `rgba(143,100,246,0.10–0.12)` and border `rgba(175,151,250,0.55)`.
  - Radio ring `#af97fa` with an 8 px dot.
- **Agents list:**
  - Box radius 14, border `rgba(255,255,255,0.07)`, fill `rgba(255,255,255,0.025)`.
  - Rows are 12×16 padding with a 32 px mark tile (radius 8).
  - Max height 286, then it scrolls.
  - The footer line sits below a 1 px rule.
- **Status dots (7 px):**
  - Signed in: `#10b981`.
  - Not signed in: `#e8c547`.
  - Unknown: `#737373`.
- **Look card:**
  - Labels sit in a 120 px column.
  - Theme is a segmented control with radius 9 and a `rgba(235,235,235,0.06)` track.
  - Accent swatches are 26 px; the selected one gets a 2 px `#171717` gap and a 4 px
    `#ebebeb` ring.
  - The glass switch is 36×22.

0. **Welcome.** Title "Welcome to Gespan.", one line on what it is, a short description and a
   row of agent names it works with. "Get started" or "Skip setup". No step list.
1. **Coding agents.** Only installed agents, in a list that scrolls past about five rows.
   Each row: default radio, mark, name, status (Signed in / Not signed in / Signs in on first
   use) and "Sign in" where `signed_in` returns `Some(false)`. Under the list, a line built
   from what is missing: "Not found: A, B and N more" (two or fewer: "Not found: A and B";
   none: "Every supported agent is installed"), plus "6 of 12 installed". Rescan re-probes.
   Picking a radio calls `use_by_default`.
   - Zero installed: "No coding agent found." with "How to install" links and Rescan;
     Continue still works.
2. **Look.** Theme segmented control, Focused/Classic cards with sketches, accent swatches,
   Glass switch. Each pick applies live behind the card and saves at once.
3. **Project. Required.** Suggests the launch folder ("Opened from"), plus "Choose a folder…"
   (⌘O) and "Folder on a machine…". No Skip. The primary button reads "Open {name}" and is
   disabled until a folder is picked. If the user skipped earlier cards, one line lists the
   defaults kept.

After Project: `add_project` + `select_project`, mark setup done, close the card, open a new
thread in that project with the composer focused, and show a dismissible "You're set up"
toast.

## When it shows

Show setup only on a true first run: there's no saved workspace row and the store has no
sessions. In that case `main.rs` writes `setup: Welcome` into the workspace it hands the
shell, and skips creating the usual initial session in the launch folder.

Any saved workspace without a `setup` key counts as done. That covers existing installs.
Tauri migrants have sessions, so they're covered too.

Don't key it on "no projects".

The launch folder is still added as a project, as today. If it isn't the folder picked and
has no sessions, finishing setup removes it again. It's offered as the "Opened from"
suggestion, except when it's `/` or the home directory (a Dock launch).

## Skip and resume

- Skip on a card keeps that card's defaults and moves on.
- "Skip setup" on any card jumps to Project.
- Each pick saves immediately, plus `setup.step` (the card to resume on).
- Quitting reopens on that card. Quitting on Project reopens Project.
- `setup.done` is set only when a project is opened from the Project card.
- Settings › General gets "Run setup again", next to "What's new". It opens Welcome with the
  current values filled in and never removes projects.

Defaults when skipped:
- **Agent:** the first installed and signed-in agent; else the first installed one; else
  none.
- **Look:** System theme, Focused layout, the current accent and glass defaults.

## What's stored

| Value | Where | New? |
| --- | --- | --- |
| card to resume on, or done | `Workspace.setup: Option<SetupStep>` (`None` = done) | new |
| default agent | `Preferences.default_provider` / `last_provider` via `use_by_default` | existing |
| theme, accent, glass | `Preferences.theme`, `accent_color`, `inset_glass` | existing |
| layout | `Workspace.classic_layout` (through `toggle_layout`) | existing |
| project | `Workspace.projects` / `project` via `add_project` + `select_project` | existing |

`serde(default)` gives every existing install `setup = None` (done), so no migration is
needed. Only a true first run sets it. The implementation plan is in
`docs/design/first-run-setup-plan.md`.

## Implementation sketch (for the plan)

- `crates/gespan-gpui/src/setup/` (new): `mod.rs` with the `Setup` view (entity, step state,
  render of the card shell) and one file per card (`welcome.rs`, `agents.rs`, `look.rs`,
  `project.rs`). Pure helpers kept testable without GPUI: `should_show(done, sessions)`,
  `not_found_line(&[Kind])`, `skip_defaults(installed, signed_in)`.
- `workspace.rs`: add `setup_done`, `setup_step`. In `restore`, skip `add_project(cwd)` while
  setup is pending and keep the cwd as the suggestion.
- `shell.rs` / `shell/notifications.rs`: hold the setup entity like `whats_new`. Render it
  above the modal stack, open on startup when `should_show`. It is modal and can't be
  dismissed: it swallows Esc and outside clicks, and blocks the shell's shortcuts except ⌘O
  (the folder picker) and ⌘Q.
- Agents card: reuse `installed_providers::snapshot` and `accounts::login::signed_in`
  (off the UI thread); "Sign in" reuses the existing `accounts/signin.rs` flow.
- Look card: call the same setters the Appearance page uses so live preview matches.
- Project card: reuse the `open_project` picker and the remote-project modal.
- Settings › General: "Run setup again" button.

## Tests worth writing

- `should_show`: fresh store → true; sessions but no flag → false and the flag is written;
  `setup_done` true → false.
- A Tauri-migrated store (fixture `schema-v1.sql` with one session, as in
  `gespan-first-launch`) never shows setup.
- `not_found_line`: 0, 1, 2, 3 and 12 missing; names follow `Kind` order.
- `skip_defaults`: a signed-in agent beats an earlier installed one; none installed leaves the
  provider unset.
- Resume: `setup_step = Look` on restore opens Look; quitting on Project reopens Project;
  "Skip setup" sets the step to Project.
- Launch folder is not auto-added while setup is pending, and is added when chosen; `/` and
  home are never suggested.
- Finishing sets `setup_done`, selects the project and opens a new thread in it.
- Esc, a click on the scrim and the shell's shortcuts leave the card open and the step
  unchanged.

## Open questions

- Dropping a folder onto the Project card is shown in the canvas but doesn't exist yet. Keep
  it in scope or drop the hint?
- Should "Run setup again" live in General or Appearance?
- Agent marks: use the real provider logos from the app's assets (the canvas uses letters).
