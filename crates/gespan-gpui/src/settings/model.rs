//! The Settings pages, their rail groups and the index Settings search jumps
//! through.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Section {
    #[default]
    General,
    Diagnostics,
    /// Your machines, herdr machines and phones. Saved as `connections`
    /// before Connections came to mean services, so that still opens here.
    #[serde(alias = "connections")]
    Devices,
    /// Connected services: code hosting and issue trackers. Shown as
    /// Connections.
    Services,
    Appearance,
    Keybindings,
    Dictation,
    Chat,
    Providers,
    Mcp,
    Browser,
    Skills,
    Inbox,
    Archive,
    Worktrees,
}

pub(crate) const GROUPS: [(&str, &[Section]); 4] = [
    (
        "App",
        &[
            Section::General,
            Section::Appearance,
            Section::Keybindings,
            Section::Dictation,
            Section::Diagnostics,
        ],
    ),
    ("Connect", &[Section::Services, Section::Devices]),
    (
        "Agents",
        &[
            Section::Chat,
            Section::Providers,
            Section::Mcp,
            Section::Browser,
            Section::Skills,
        ],
    ),
    (
        "Workspace",
        &[Section::Inbox, Section::Archive, Section::Worktrees],
    ),
];

impl Section {
    /// In the rail's order.
    pub(crate) const ALL: [Section; 15] = [
        Section::General,
        Section::Appearance,
        Section::Keybindings,
        Section::Dictation,
        Section::Diagnostics,
        Section::Services,
        Section::Devices,
        Section::Chat,
        Section::Providers,
        Section::Mcp,
        Section::Browser,
        Section::Skills,
        Section::Inbox,
        Section::Archive,
        Section::Worktrees,
    ];
    pub(crate) fn id(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Diagnostics => "diagnostics",
            Section::Devices => "devices",
            Section::Services => "services",
            Section::Appearance => "appearance",
            Section::Keybindings => "keybindings",
            Section::Dictation => "dictation",
            Section::Chat => "chat",
            Section::Providers => "providers",
            Section::Mcp => "mcp",
            Section::Browser => "browser",
            Section::Skills => "skills",
            Section::Inbox => "inbox",
            Section::Archive => "archive",
            Section::Worktrees => "worktrees",
        }
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Diagnostics => "Diagnostics",
            Section::Devices => "Devices",
            Section::Services => "Connections",
            Section::Appearance => "Appearance",
            Section::Keybindings => "Keybindings",
            Section::Dictation => "Dictation",
            Section::Chat => "Chat",
            Section::Providers => "Providers",
            Section::Mcp => "MCP",
            Section::Browser => "Browser",
            Section::Skills => "Skills",
            Section::Inbox => "Inbox",
            Section::Archive => "Archive",
            Section::Worktrees => "Worktrees",
        }
    }
    /// Pages drawn in the glass presentation: a glass card per group,
    /// segmented tracks, accent sliders. A page joins by adding itself here.
    pub(crate) fn glass(self) -> bool {
        true
    }
    pub(crate) fn description(self) -> &'static str {
        match self {
            Section::General => {
                "The build you are running, how Gespan reaches you, and how it runs."
            }
            Section::Diagnostics => {
                "Report a problem, your logs, and the failures counted on this machine."
            }
            Section::Devices => {
                "Your machines over SSH, herdr machines that run agents, and the phones you pair."
            }
            Section::Services => "Your connected services. Add more from the catalogue.",
            Section::Appearance => {
                "Theme, colour, glass and layout. Changes apply as you make them."
            }
            Section::Keybindings => {
                "Every shortcut the workspace handles, from the app menu and the key handler."
            }
            Section::Dictation => "Speak into the composer. Speech is recognized on this device.",
            Section::Chat => {
                "How transcripts read, what the composer does with a follow-up, how files save, and how diffs open."
            }
            Section::Providers => {
                "Provider accounts, agent CLIs Gespan can drive, and the model new sessions start with."
            }
            Section::Mcp => "Find MCP servers across providers and manage their connections.",
            Section::Browser => {
                "Let agents open and use their own tabs in Gespan's built-in browser."
            }
            Section::Skills => {
                "Discover and manage file skills from project, personal, and harness folders."
            }
            Section::Inbox => "Notification preferences for each project.",
            Section::Archive => "Projects and conversations you have archived.",
            Section::Worktrees => "Manage additional worktrees for each project.",
        }
    }
    fn keywords(self) -> &'static str {
        match self {
            Section::General => "version update sounds notifications",
            Section::Diagnostics => {
                "diagnostics crash report issue bug email github logs folder copy failures counters errors"
            }
            Section::Devices => {
                "ssh remote host machine server environment always on herdr machines remote agents phone mobile connections"
            }
            Section::Services => {
                "github gitlab linear jira plane atlassian azure devops connect token integration services"
            }
            Section::Appearance => {
                "theme dark light color accent glass blur zoom scale wallpaper sidebar tint translucency workspace style"
            }
            Section::Keybindings => "shortcut hotkey keyboard binding",
            Section::Dictation => "voice microphone speech parakeet",
            Section::Chat => {
                "transcript composer prompt message diff review layout format save editor"
            }
            Section::Providers => {
                "account sign in login model harness claude codex gemini cli default hooks"
            }
            Section::Mcp => {
                "tools servers connections oauth authenticate login claude codex cursor opencode"
            }
            Section::Browser => {
                "browser chrome edge brave chromium web page tabs cdp built-in in-app sign in upload"
            }
            Section::Skills => "skill instructions prompt",
            Section::Inbox => "notifications mute projects categories",
            Section::Archive => "archived restore delete hidden",
            Section::Worktrees => "git branch worktree working copy project create delete",
        }
    }
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Section::General => "sliders",
            Section::Diagnostics => "bug",
            Section::Devices => "smart-phone",
            Section::Services => "link-01",
            Section::Appearance => "palette",
            Section::Keybindings => "keyboard",
            Section::Dictation => "mic-01",
            Section::Chat => "message-square",
            Section::Providers => "bot",
            Section::Mcp => "globe",
            Section::Browser => "app-window",
            Section::Skills => "sparkles",
            Section::Inbox => "inbox-hi",
            Section::Archive => "archive",
            Section::Worktrees => "folder-tree",
        }
    }
}

/// SETTINGS_INDEX: (id, section, label, keywords).
pub(crate) const INDEX: &[(&str, Section, &str, &str)] = &[
    (
        "remote-machines",
        Section::Devices,
        "Your machines",
        "ssh remote connect host server environment",
    ),
    (
        "herdr-machines",
        Section::Devices,
        "herdr machines",
        "herdr remote agents machines ssh multiplexer upgrade",
    ),
    (
        "mobile-access",
        Section::Devices,
        "Your phones",
        "phone iphone android mobile app pair qr code device revoke remove testflight",
    ),
    (
        "mobile-remote",
        Section::Devices,
        "Connect away from home",
        "router port upnp nat remote address public hostname vpn wifi",
    ),
    (
        "mcp-servers",
        Section::Mcp,
        "MCP servers",
        "claude tools connections oauth authenticate login add remove",
    ),
    (
        "browser-toggle",
        Section::Browser,
        "Let agents use the browser",
        "browser chrome agents web switch on off beta",
    ),
    (
        "browser-access",
        Section::Browser,
        "Thread access",
        "browser threads access always on agents",
    ),
    (
        "browser-uploads",
        Section::Browser,
        "Upload folders",
        "browser upload files folders hidden dotfiles",
    ),
    #[cfg(target_os = "macos")]
    (
        "pilot-menu-bar-icon",
        Section::General,
        "Menu bar icon",
        "menu bar icon tray pilot quick composer chat macos hide",
    ),
    (
        "dictation-model",
        Section::Dictation,
        "Speech model",
        "voice microphone parakeet download english multilingual delete",
    ),
    (
        "dictation-words",
        Section::Dictation,
        "Custom words",
        "voice vocabulary names identifiers spelling",
    ),
    (
        "dictation-shortcut",
        Section::Dictation,
        "Shortcut",
        "voice push to talk hold tap microphone",
    ),
    (
        "project-worktrees",
        Section::Worktrees,
        "Project worktrees",
        "git branch working copy create delete manage",
    ),
    (
        "update",
        Section::General,
        "Version",
        "update upgrade release what's new build changelog",
    ),
    (
        "sounds",
        Section::General,
        "Sounds",
        "audio cue chime mute volume",
    ),
    (
        "notifications",
        Section::General,
        "Notifications",
        "notify alert toast permission reminder background",
    ),
    (
        "quick-composer",
        Section::General,
        "Quick composer",
        "spotlight global shortcut hotkey floating prompt anywhere",
    ),
    (
        "keep-awake",
        Section::General,
        "Keep screen awake",
        "keep awake sleep lock idle caffeinate display power battery prevent pause",
    ),
    (
        "action-opens-terminal",
        Section::General,
        "Show terminal for scripts",
        "actions commands run script dock background",
    ),
    (
        "terminal-placement",
        Section::General,
        "Terminal",
        "terminal placement dock bottom side right panel title bar tab shell focused layout",
    ),
    (
        "file-tabs",
        Section::Chat,
        "File tabs",
        "editor open top workspace normal session pane beside chat",
    ),
    (
        "file-word-wrap",
        Section::Chat,
        "Word wrap",
        "files editor long lines horizontal scroll wrapping",
    ),
    (
        "tab-animations",
        Section::Appearance,
        "Tab animations",
        "motion open close resize transition",
    ),
    (
        "theme",
        Section::Appearance,
        "Theme",
        "dark light system appearance mode",
    ),
    (
        "wayland-window-decorations",
        Section::Appearance,
        "Wayland window decorations",
        "wayland linux compositor title bar titlebar border decorations minimize maximize close",
    ),
    (
        "workspace-theme",
        Section::Appearance,
        "Workspace theme",
        "workspace theme color colour text accent syntax scheme preset harbor moss ember orchid",
    ),
    (
        "accent-color",
        Section::Appearance,
        "Accent color",
        "highlight bubble send button tint",
    ),
    (
        "diff-colors",
        Section::Appearance,
        "Diff colors",
        "colorblind color blind accessibility added removed red green blue orange high contrast changes",
    ),
    ("hue", Section::Appearance, "Hue", "tint color chrome"),
    (
        "saturation",
        Section::Appearance,
        "Saturation",
        "tint color neutral grey gray",
    ),
    (
        "dark-lightness",
        Section::Appearance,
        "Dark-mode lightness",
        "black brightness contrast background",
    ),
    (
        "inset-glass",
        Section::Appearance,
        "Transparent window",
        "glass inset translucent transparency opaque desktop blur layout",
    ),
    (
        "sidebar-opacity",
        Section::Appearance,
        "Sidebar opacity",
        "glass translucent transparency vibrancy",
    ),
    (
        "blur",
        Section::Appearance,
        "Blur radius",
        "glass translucent vibrancy backdrop",
    ),
    (
        "main-pane-glass",
        Section::Appearance,
        "Main pane glass",
        "translucent transparency body window",
    ),
    (
        "new-layout-style",
        Section::Appearance,
        "Workspace style",
        "rounded boxy card frame full height layout",
    ),
    (
        "sidebar-notes",
        Section::Appearance,
        "Notes",
        "notebook markdown scratchpad sidebar show hide",
    ),
    (
        "sidebar-board",
        Section::Appearance,
        "Board",
        "kanban cards columns tasks tickets track sidebar show hide",
    ),
    (
        "sidebar-activity",
        Section::Appearance,
        "Activity",
        "working agents activity live running sessions card sidebar show hide",
    ),
    (
        "sidebar-automations",
        Section::Appearance,
        "Automations",
        "sidebar entries show hide",
    ),
    (
        "sidebar-inbox",
        Section::Appearance,
        "Inbox",
        "sidebar entries show hide",
    ),
    (
        "sidebar-pilots",
        Section::Appearance,
        "Pilots",
        "sidebar entries show hide",
    ),
    (
        "compact-session-cards",
        Section::Appearance,
        "Compact view",
        "sidebar session cards density show hide",
    ),
    (
        "interface-scale",
        Section::Appearance,
        "Interface scale",
        "zoom font size bigger smaller ui",
    ),
    (
        "show-excluded-files",
        Section::Chat,
        "Show excluded files",
        "explorer gitignore ignored hidden files tree",
    ),
    (
        "chat-background",
        Section::Appearance,
        "Chat background",
        "wallpaper image picture opacity backdrop",
    ),
    (
        "transcript-layout",
        Section::Chat,
        "Transcript layout",
        "full width chat bubble message",
    ),
    (
        "anchor-prompts",
        Section::Chat,
        "Anchor prompts to top",
        "scroll position sticky message",
    ),
    (
        "paced-reveal",
        Section::Chat,
        "Smooth text reveal",
        "streaming pacing immediate latency performance animation",
    ),
    (
        "follow-up",
        Section::Chat,
        "Follow-up behavior",
        "queue steer interrupt send while running",
    ),
    (
        "model-controls",
        Section::Chat,
        "Model controls",
        "effort thinking reasoning fast service tier model picker composer",
    ),
    (
        "composer-autocorrect",
        Section::Chat,
        "Autocorrect",
        "spelling spell check autocorrect typo macos composer",
    ),
    (
        "composer-mascot",
        Section::Chat,
        "Composer mascot",
        "runner animation coin fun",
    ),
    (
        "format-on-save",
        Section::Chat,
        "Format on save",
        "prettier quotes editor save format",
    ),
    (
        "diff-view",
        Section::Chat,
        "Diff view",
        "unified editor review changes working tree",
    ),
    (
        "empty-session-games",
        Section::Chat,
        "Empty session games",
        "pacman snake tetris arcade grid fun",
    ),
    (
        "agent-clis",
        Section::Providers,
        "Agent CLIs",
        "codex opencode cursor grok pi omp fx hermes kiro antigravity devin binary path",
    ),
    (
        "provider-accounts",
        Section::Providers,
        "Provider accounts",
        "account sign in login rename remove delete credentials profile usage limit quota exhausted",
    ),
    (
        "show-remaining-usage",
        Section::Providers,
        "Show remaining usage",
        "usage limit meter bar left used quota percent",
    ),
    (
        "mask-emails",
        Section::Providers,
        "Mask account emails",
        "email privacy blur hide screenshot account",
    ),
    (
        "claude-hooks",
        Section::Providers,
        "Claude Code hooks",
        "pretooluse settings.json block command notification",
    ),
    (
        "decision-log",
        Section::Providers,
        "Decision log",
        "decisions preferences memory remember agents project choices",
    ),
    (
        "decision-meaning",
        Section::Providers,
        "Find related decisions by meaning",
        "decisions semantic embeddings vectors similar related nomic search model",
    ),
    (
        "idle-agents",
        Section::Providers,
        "Keep idle agents ready",
        "memory ram cpu performance process warm park background",
    ),
    (
        "project-notifications",
        Section::Inbox,
        "Project notifications",
        "mute resume sounds banners reminders categories",
    ),
    (
        "github",
        Section::Services,
        "GitHub",
        "gh cli connect pull request sign in",
    ),
    (
        "gitlab",
        Section::Services,
        "GitLab",
        "token self-managed merge request connect",
    ),
    (
        "azuredevops",
        Section::Services,
        "ADO",
        "azure devops boards repos pull request pat organization connect",
    ),
    (
        "jira",
        Section::Services,
        "Jira",
        "atlassian cloud site email api token issues projects connect",
    ),
    (
        "linear",
        Section::Services,
        "Linear",
        "api key issues teams connect",
    ),
    (
        "show-archived",
        Section::Archive,
        "Show archived in the sidebar",
        "hidden conversations list",
    ),
];

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SearchResult {
    pub section: Section,
    pub setting: Option<&'static str>,
    pub label: &'static str,
}

fn score(needle: &str, label: &str, keywords: &str) -> Option<f32> {
    let lower = label.to_lowercase();
    if lower.starts_with(needle) {
        Some(0.)
    } else if lower.contains(needle) {
        Some(1.)
    } else if keywords.to_lowercase().contains(needle) {
        Some(2.)
    } else {
        None
    }
}

/// searchSettings: individual settings first, then whole sections.
pub(crate) fn search(query: &str, compositor: &str) -> Vec<SearchResult> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(f32, SearchResult)> = Vec::new();
    for &(id, section, label, keywords) in INDEX {
        if id == "wayland-window-decorations" && compositor != "Wayland" {
            continue;
        }
        if id == "blur" && !crate::window_blur::BLUR_ADJUSTABLE {
            continue;
        }
        if id == "herdr-machines" && !crate::machines::MACHINES_VISIBLE {
            continue;
        }
        if let Some(s) = score(&needle, label, keywords) {
            scored.push((
                s,
                SearchResult {
                    section,
                    setting: Some(id),
                    label,
                },
            ));
        }
    }
    for section in Section::ALL {
        let keywords = format!("{} {}", section.description(), section.keywords());
        if let Some(s) = score(&needle, section.label(), &keywords) {
            scored.push((
                s + 0.5,
                SearchResult {
                    section,
                    setting: None,
                    label: section.label(),
                },
            ));
        }
    }
    scored.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1.label.to_lowercase().cmp(&b.1.label.to_lowercase()))
    });
    scored.into_iter().take(8).map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keep_awake_is_found_by_the_words_people_use_for_a_locking_laptop() {
        for query in ["keep awake", "sleep", "lock", "caffeinate"] {
            let results = search(query, "");
            let hit = results.iter().find(|r| r.setting == Some("keep-awake"));
            assert_eq!(hit.map(|r| r.section), Some(Section::General), "{query}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn menu_bar_icon_is_in_general() {
        let results = search("menu bar", "");
        let hit = results
            .iter()
            .find(|r| r.setting == Some("pilot-menu-bar-icon"));
        assert_eq!(hit.map(|r| r.section), Some(Section::General));
    }

    #[test]
    fn the_rail_groups_every_section_as_designed() {
        assert_eq!(Section::ALL.len(), 15);
        let grouped: Vec<Section> = GROUPS.iter().flat_map(|(_, s)| s.iter().copied()).collect();
        assert_eq!(grouped, Section::ALL, "ALL follows the rail, each once");
        let names: Vec<&str> = GROUPS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["App", "Connect", "Agents", "Workspace"]);
        assert_eq!(
            GROUPS[0].1,
            [
                Section::General,
                Section::Appearance,
                Section::Keybindings,
                Section::Dictation,
                Section::Diagnostics
            ]
        );
        assert_eq!(GROUPS[1].1, [Section::Services, Section::Devices]);
        assert_eq!(
            GROUPS[2].1,
            [
                Section::Chat,
                Section::Providers,
                Section::Mcp,
                Section::Browser,
                Section::Skills
            ]
        );
        assert_eq!(
            GROUPS[3].1,
            [Section::Inbox, Section::Archive, Section::Worktrees]
        );
        // Services are called Connections; machines and phones are Devices.
        assert_eq!(Section::Services.label(), "Connections");
        assert_eq!(Section::Devices.label(), "Devices");
        for query in ["dictation", "microphone", "parakeet", "microphone speech"] {
            assert!(
                search(query, "")
                    .iter()
                    .any(|r| r.section == Section::Dictation),
                "{query}"
            );
        }
        assert_eq!(
            serde_json::to_string(&Section::Dictation).unwrap(),
            "\"dictation\""
        );
    }

    #[test]
    fn kanban_finds_the_board_setting_in_the_sidebar_group() {
        assert!(
            search("kanban", "")
                .iter()
                .any(|r| r.setting == Some("sidebar-board") && r.section == Section::Appearance)
        );
    }

    #[test]
    fn color_and_translucency_default_to_neutral_black_glass() {
        use crate::settings::preferences::Preferences;
        let defaults = Preferences::default();
        assert_eq!(
            (
                defaults.theme_hue,
                defaults.theme_saturation,
                defaults.theme_dark_lightness,
                defaults.sidebar_opacity,
                defaults.sidebar_blur,
            ),
            (0, 0, 0, 90, 7)
        );
        assert!(defaults.inset_glass && defaults.body_glass);
        // Restore defaults on Appearance lands on the same values.
        let mut changed = Preferences {
            theme_hue: 240,
            theme_saturation: 40,
            theme_dark_lightness: 9,
            sidebar_opacity: 85,
            sidebar_blur: 24,
            inset_glass: false,
            body_glass: false,
            ..Preferences::default()
        };
        changed.restore_appearance();
        assert_eq!(changed, defaults);
    }

    #[test]
    fn board_is_on_until_turned_off() {
        assert!(crate::settings::preferences::Preferences::default().board);
    }

    #[test]
    fn moved_rows_are_found_on_their_new_pages() {
        for (query, id, section) in [
            ("file tabs", "file-tabs", Section::Chat),
            ("word wrap", "file-word-wrap", Section::Chat),
            ("show excluded files", "show-excluded-files", Section::Chat),
            ("tab animations", "tab-animations", Section::Appearance),
        ] {
            let hit = search(query, "")
                .into_iter()
                .find(|r| r.setting == Some(id));
            assert_eq!(hit.map(|r| r.section), Some(section), "{query}");
        }
        for id in ["file-tabs", "file-word-wrap", "show-excluded-files"] {
            let entries: Vec<_> = INDEX.iter().filter(|e| e.0 == id).collect();
            assert_eq!(entries.len(), 1, "{id} is indexed once");
            assert_eq!(entries[0].1, Section::Chat, "{id}");
        }
    }

    #[test]
    fn the_workspace_style_row_is_searchable() {
        for query in ["boxy", "workspace style"] {
            let hit = search(query, "")
                .into_iter()
                .find(|r| r.setting == Some("new-layout-style"));
            assert_eq!(hit.map(|r| r.section), Some(Section::Appearance), "{query}");
        }
    }

    #[test]
    fn retired_rows_are_not_searchable() {
        for id in [
            "inset-layout",
            "rail-pilots-pinned",
            "pilots-enabled",
            "notes",
            "board",
            "working-agents",
            "search-panel",
        ] {
            assert!(INDEX.iter().all(|e| e.0 != id), "{id} is still indexed");
        }
    }

    #[test]
    fn a_remembered_connections_section_opens_devices() {
        let old: Section = serde_json::from_str("\"connections\"").unwrap();
        assert_eq!(old, Section::Devices);
        assert_eq!(
            serde_json::to_string(&Section::Devices).unwrap(),
            "\"devices\""
        );
        let services: Section = serde_json::from_str("\"services\"").unwrap();
        assert_eq!(services, Section::Services);
        // Machines and phones are found under Devices, services under
        // Connections.
        let section = |id| INDEX.iter().find(|e| e.0 == id).map(|e| e.1);
        assert_eq!(section("remote-machines"), Some(Section::Devices));
        assert_eq!(section("mobile-access"), Some(Section::Devices));
        for id in ["github", "gitlab", "azuredevops", "jira", "linear"] {
            assert_eq!(section(id), Some(Section::Services), "{id}");
        }
        assert!(Section::Devices.description().contains("herdr"));
        assert!(Section::Devices.description().contains("phones"));
    }

    #[test]
    fn section_copy_names_what_each_page_now_holds() {
        assert_eq!(
            Section::General.description(),
            "The build you are running, how Gespan reaches you, and how it runs."
        );
        assert_eq!(
            Section::Appearance.description(),
            "Theme, colour, glass and layout. Changes apply as you make them."
        );
        assert!(!Section::General.keywords().contains("rail"));
        assert!(!Section::Appearance.keywords().contains("rail"));
    }

    #[test]
    fn wayland_decorations_are_searchable_only_on_wayland() {
        for query in ["Wayland", "window decorations", "compositor", "title bar"] {
            let results = search(query, "Wayland");
            let result = results
                .iter()
                .find(|r| r.setting == Some("wayland-window-decorations"))
                .unwrap();
            assert_eq!(result.section, Section::Appearance);
            for compositor in ["X11", "Headless", ""] {
                assert!(
                    search(query, compositor)
                        .iter()
                        .all(|r| r.setting != Some("wayland-window-decorations"))
                );
            }
        }
    }

    #[test]
    fn rows_win_their_own_name_and_shared_appearance_rows_are_searchable() {
        let results = search("theme", "Wayland");
        assert_eq!(results[0].setting, Some("theme"));
        assert_eq!(results[1].setting, Some("workspace-theme"));
        assert!(
            search("hue", "Wayland")
                .iter()
                .any(|r| r.setting == Some("hue"))
        );
        assert!(
            search("workspace theme", "Wayland")
                .iter()
                .any(|r| r.setting == Some("workspace-theme"))
        );
        let pages = search("archive", "Wayland");
        assert!(
            pages
                .iter()
                .any(|r| r.section == Section::Archive && r.setting.is_none())
        );
    }
}
