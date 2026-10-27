# Platform parity audit (2026-10-27)

Target split:

| Platform | App | Code |
| --- | --- | --- |
| macOS | Native (Rust + GPUI) | `crates/gespan-gpui/`, `crates/gespan-core/` |
| Linux, Windows | React + Tauri | `src/`, `src-tauri/` |
| iOS, Android | Expo, a remote client of either desktop | `mobile/`, `crates/mobile-link/` |

Tauri does **not** link `crates/gespan-core`. It only uses `mobile-link`,
`keep-awake`, `tracker-secrets`, `gespan-models` and `gespan-speech`. Anything
built only in core (Flows, Board extras, Diagnostics) is unavailable on Linux
and Windows until Tauri reimplements it or links core.

Legend: ✅ full · 🟡 partial · ❌ absent · — not applicable

## Feature matrix

| Feature | Tauri macOS | Tauri Linux/Win | Native macOS | Mobile |
| --- | --- | --- | --- | --- |
| **Providers** (12 harnesses) | ✅ | ✅ (Antigravity ❌ on Windows) | ✅ transports | ✅ via desktop |
| Steering mid-turn | ✅ all 12 | ✅ all 12 | 🟡 not Grok/fx/Kiro/Antigravity/Devin | 🟡 follow-up only |
| Compaction | 🟡 7 providers | 🟡 | 🟡 same 7 | ❌ |
| Rewind / edit last turn | 🟡 Codex/Pi/omp/OpenCode | 🟡 | 🟡 same | ❌ |
| Usage / rate limits | 🟡 Claude/Codex/OpenCode/Devin/Pi | 🟡 | 🟡 same | 🟡 Claude/Codex |
| Accounts | 🟡 Claude/Codex/Devin | 🟡 | 🟡 same (native-only storage) | 🟡 pick only |
| **Composer**: attachments | ✅ | ✅ | ✅ | ✅ |
| Slash commands / @mentions | ✅ | ✅ | ✅ | ❌ |
| Dictation | ✅ | ✅ (Linux needs libasound2) | ✅ | 🟡 iOS 26+ only |
| Prompt queue | ✅ | ✅ | ✅ | 🟡 not visible |
| **Transcript**: markdown | ✅ + Mermaid + math | ✅ | 🟡 no Mermaid, math as raw TeX | ✅ basic |
| Tool cards | ✅ | ✅ | 🟡 | 🟡 phases only |
| Inline tool diffs | ✅ | ✅ | ❌ | ❌ |
| Images in transcript | ✅ | ✅ | ✅ | ❌ |
| Thinking | ✅ | ✅ | ✅ | 🟡 label only |
| Questions / approvals | ✅ | ✅ | ✅ | ❌ status only |
| Permission / plan modes | ✅ | ✅ | ✅ | 🟡 no plan mode |
| Hold / resume | ✅ | ✅ | ✅ | ❌ |
| Checkpoints / session review | ✅ per-file Undo/Keep | ✅ | 🟡 no per-file Undo/Keep, Git only | ❌ |
| Link previews | ✅ | ✅ | ❌ | ❌ |
| Second opinion, handoff, `/btw` | ✅ | ✅ | ✅ | ❌ |
| **Workspace**: project rail | ✅ | ✅ | ✅ | ✅ list |
| Worktrees | ✅ | ✅ | ✅ | ✅ pick/create |
| Pane splits | ✅ | ✅ | ✅ | — |
| Tab groups | ✅ | ✅ | ❌ (metadata only) | — |
| Multi-window / window transfer | ✅ | ✅ | ❌ | — |
| **Files**: explorer, editor, quick open, search | ✅ | ✅ | ✅ | 🟡 viewer only; tree/search built but unmounted |
| Video in file viewer | ✅ | ✅ | 🟡 macOS only (fine) | ❌ |
| **Git**: status, stage, commit, push/pull | ✅ | ✅ | ✅ | ✅ |
| Branches, graph | ✅ | ✅ | ✅ | ❌ branch switch |
| PRs (via `gh`) | ✅ | ✅ | 🟡 | 🟡 create/view |
| **Terminal** | ✅ tabs + panes | ✅ (no foreground cmd on Windows) | 🟡 dock only, no panes | ❌ |
| **Inbox** (GitHub/GitLab/Jira/Linear/ADO) | ✅ | ✅ | ✅ | ❌ |
| Notes | ✅ | ✅ | ✅ | ❌ |
| Board | ✅ many boards | ✅ | 🟡 one board, extras in `board_native_*` | ❌ |
| Automations | ✅ (frontend scheduler) | ✅ (stops when app quits) | 🟡 source review only | ❌ |
| Pilots | ✅ | ✅ | ✅ | ❌ hidden (`PILOTS_ENABLED = false`) |
| Floating Pilot chat / menu-bar icon | ✅ | ❌ macOS-gated | ✅ | — |
| **Flows** | ❌ | ❌ | ✅ native-only | ❌ |
| Orchestration | ✅ control CLI | ✅ | 🟡 in-chat plans, no control CLI | ❌ |
| Artifacts panel | ✅ | ✅ | 🟡 Pilot documents only | ❌ |
| Skills, MCP | ✅ | ✅ | ✅ | ❌ |
| **Remote SSH hosts** | 🟡 | 🟡 | 🟡 stricter limits | — |
| Serves mobile | ✅ 24 methods | ✅ | ✅ 24 methods | — |
| **OS**: Quick Composer | ✅ | ❌ macOS-gated | 🟡 no attachments or Git controls | — |
| Notifications | ✅ | ✅ (no Linux settings link) | ✅ | 🟡 local, foreground only |
| Tray / close-to-tray | ❌ | 🟡 Windows only, Linux ❌ | 🟡 needs Pilots + menu bar | — |
| App menu | ✅ native | 🟡 in-window File/View/Terminal | 🟡 4 menus | — |
| Dock badge | ✅ | — | ✅ | — |
| Keep awake | ✅ | ✅ (Linux needs systemd-inhibit) | ✅ | — |
| Window glass | ✅ | 🟡 Windows Acrylic, Linux opaque | ✅ | — |
| Trackpad zoom | ✅ | ❌ (keyboard zoom works) | 🟡 media only | — |
| Resource pressure / stop tools | ✅ | ❌ macOS only (`/bin/ps`) | ✅ | — |
| Copy file to clipboard | ✅ | ❌ | ? | — |
| Auto-updater | ✅ | 🟡 `createUpdaterArtifacts: false` | ✅ separate feed | store |
| Deep links (`gespan://`) | ❌ | ❌ | ❌ | ❌ (scheme declared, no route) |
| Diagnostics / report issue | ❌ | ❌ | ✅ | ❌ |
| Settings: fonts | ❌ | ❌ | ❌ | — |

## Gaps by track

### 1. Tauri on Linux and Windows (shipping app)

1. Quick Composer is macOS-only (`src-tauri/src/lib.rs` cfg gates, `quickComposer.ts`).
2. Floating Pilot chat and its status icon are macOS-only (`pilot_chat.rs`).
3. Linux has no tray, so close-to-tray isn't available. Because automations
   and Pilot habits are scheduled from the frontend, they stop when the window
   closes (`App.tsx` `claimDueAutomations`).
4. Linux and Windows don't produce updater artifacts
   (`tauri.linux.conf.json`, `tauri.windows.conf.json`).
5. Resource pressure and stopping runaway tools work only on macOS (`resources.rs`).
6. Antigravity is unavailable on Windows (`harness.rs` `resolve_antigravity`).
7. The in-window menu has only File/View/Terminal (no Edit/Window/Help).
8. The terminal doesn't detect the foreground command on Windows (`pty.rs` `pty_status`).
9. Copying a file to the clipboard is macOS-only (`pasteboard.rs`).
10. Linux window glass is forced opaque. Pinch zoom is missing on Linux and Windows.
11. Install hints for Grok, omp, fx and Kiro are `curl | bash` commands, which don't work on Windows.
12. Missing native-only features: Flows, Diagnostics/report issue, Board extras.

### 2. Native on macOS vs Tauri (the behavior reference)

1. No multi-window or window transfer, and no tab groups.
2. No inline tool diffs in the transcript. No Mermaid, and math shows as raw TeX.
3. No link previews.
4. The terminal is dock-only and can't be a pane in the split tree.
5. No general Artifacts panel.
6. No orchestration control CLI.
7. Quick Composer has no attachments and no Git/workspace controls.
8. Board shows only the oldest board.
9. Steering is missing for Grok, fx, Kiro, Antigravity and Devin (Tauri steers all 12).
10. The app menu is minimal (no View/Window/Go).
11. No per-file checkpoint Undo/Keep.
12. Automations, OpenCode v2, Kiro and Antigravity have only source review or fixtures, with no live checks.

### 3. Shared-data divergence

- Board extras live in native-only `board_native_*` tables.
- Flows live in `flows`, `flow_runs` and `flow_visits`, which Tauri doesn't read.
- Native preferences are saved to `gespan.db.gpui.json`. They're read one way
  from Tauri's webview storage, so changes don't flow back.
- Native stores accounts under the `native_settings` key `provider-accounts`.

### 4. Mobile

Protocol bugs (native host):

1. `session.delete` returns `Ok` even when `delete_session` returns early,
   for example with detached file panes or when already deleting
   (`crates/gespan-gpui/src/shell/mobile_host.rs`, `shell/session_groups.rs`).
2. `session.start` deduplicates retries only in memory, so a retry after a
   restart can create a duplicate session. Tauri derives a fixed ID.
3. Git actions from the phone aren't recorded in the inbox (`gespan-gpui/src/mobile/project.rs`).
4. Archive and delete only refuse Pilot sessions. Tauri also refuses
   orchestration-lead and habit-run sessions.

Features:

5. The file tree and file search (`FilesView`) are built and served by both
   desktops, but never mounted.
6. No answering of approvals or questions. This needs a new protocol method on both desktops.
7. Pilots are hidden behind `PILOTS_ENABLED = false`.
8. No branch switching, slash commands, @mentions, plan mode or rewind.
9. No push notifications or background refresh.
10. Only one desktop can be paired, and `gespan://pair` deep links aren't handled.
11. Android: no dictation, no native transcript, and SwiftUI-only controls
    with no Android branch (model slider, session filter, appearance pickers,
    turn metrics sheet).
12. `mobile/README.md` refers to files that no longer exist.
