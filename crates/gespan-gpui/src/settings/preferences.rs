//! App-wide preferences the Settings pages edit. Defaults are the React
//! app's (`settings.ts`, `appearance.ts`, `displayPrefs.ts`, `sounds.ts`).
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    // General
    pub sounds: bool,
    pub notifications: bool,
    pub project_notifications: serde_json::Value,
    pub notification_projects: serde_json::Value,
    pub file_tabs: String,
    pub file_word_wrap: bool,
    pub file_view_mode: String,
    pub tab_animations: bool,
    pub notes: bool,
    pub board: bool,
    pub quick_composer: bool,
    pub working_agents: bool,
    pub close_to_tray: bool,
    /// Running an action shows the terminal it runs in.
    pub action_opens_terminal: bool,
    // Appearance
    pub theme: String,
    pub workspace_theme: String,
    // Retain the legacy inset-only key when reading and saving preferences.
    // Window materials now use the shared opacity and main-pane glass values.
    pub inset_glass: bool,
    pub accent_color: Option<String>,
    pub diff_palette: String,
    pub theme_hue: u32,
    pub theme_saturation: u32,
    pub theme_dark_lightness: u32,
    pub sidebar_opacity: u32,
    // Retain the saved value for compatibility; native window blur is fixed.
    pub sidebar_blur: u32,
    pub body_glass: bool,
    pub chat_background: Option<String>,
    pub chat_background_effect: String,
    pub chat_background_scope: String,
    pub chat_background_empty: u32,
    pub chat_background_session: u32,
    pub collapsed_project_rail: String,
    pub ui_scale: u32,
    pub show_excluded_files: bool,
    // Chat
    pub transcript_layout: String,
    pub anchor_prompts: bool,
    pub paced_reveal: bool,
    pub follow_up: String,
    pub model_controls: String,
    pub autocorrect: bool,
    pub format_on_save: bool,
    pub autosave: bool,
    pub diff_view: String,
    pub composer_mascot: bool,
    pub empty_session_games: bool,
    // Providers
    pub show_remaining_usage: bool,
    pub mask_emails: bool,
    pub idle_agents: u32,
    /// `gespan_keep_awake::Mode` value, shared with desktop `gespan.keepAwake`.
    pub keep_awake: String,
    pub claude_hooks: bool,
    pub binary_paths: HashMap<String, String>,
    pub disabled_skill_paths: Vec<String>,
    pub skill_source_paths: Vec<String>,
    /// The Settings default from before last-used existed; it now only seeds
    /// `last_provider` for preferences saved without one.
    pub default_provider: Option<String>,
    /// Each provider's last picked model (desktop `saveDefaultModel`).
    pub default_models: HashMap<String, String>,
    /// The provider last picked in any picker or Settings → Providers
    /// (desktop `gespan.lastModel`); new sessions start there.
    pub last_provider: Option<String>,
    /// Last changed composer controls other than effort, keyed as the desktop's
    /// `gespan.lastModelSettings` (`fast`, `thinking`, `serviceTier`, model
    /// control ids). Effort is per provider in `provider_efforts`.
    pub last_model_settings: HashMap<String, String>,
    pub hidden_providers: Vec<String>,
    pub project_providers: HashMap<String, ProviderDefaults>,
    pub favorite_models: Vec<ModelChoice>,
    pub recent_models: Vec<ModelChoice>,
    pub model_picker_tab: Option<String>,
    /// The last access mode picked in a composer; new chats start with it.
    pub default_access: Option<String>,
    /// Last explicit effort choice for each provider, shared across projects.
    pub provider_efforts: HashMap<String, String>,
    // Pilots
    pub pilots: bool,
    pub rail_pilots_pinned: bool,
    pub pilot_menu_bar_icon: bool,
    // Archive
    pub show_archived: bool,
    // Dictation: the speech model id, and custom words one per line as typed.
    pub dictation_model: String,
    pub dictation_words: String,
    // Keybindings: a changed shortcut, or `None` for a disabled one.
    pub keybindings: HashMap<String, Option<String>>,
}

pub(crate) const UI_SCALE_PERCENTS: [u32; 16] = [
    50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200,
];

impl Default for Preferences {
    fn default() -> Self {
        Self {
            sounds: true,
            notifications: false,
            project_notifications: serde_json::json!({}),
            notification_projects: serde_json::json!([]),
            file_tabs: "pane".into(),
            file_word_wrap: true,
            file_view_mode: "preview".into(),
            tab_animations: false,
            notes: true,
            board: true,
            quick_composer: true,
            working_agents: true,
            close_to_tray: true,
            action_opens_terminal: true,
            theme: "dark".into(),
            workspace_theme: "default".into(),
            inset_glass: true,
            accent_color: None,
            diff_palette: "default".into(),
            theme_hue: 240,
            theme_saturation: 0,
            theme_dark_lightness: 9,
            sidebar_opacity: 85,
            sidebar_blur: 24,
            body_glass: true,
            chat_background: None,
            chat_background_effect: "none".into(),
            chat_background_scope: "all".into(),
            chat_background_empty: 24,
            chat_background_session: 24,
            collapsed_project_rail: "compact".into(),
            ui_scale: 100,
            show_excluded_files: false,
            transcript_layout: "chat".into(),
            anchor_prompts: true,
            paced_reveal: true,
            follow_up: "steer".into(),
            model_controls: "menu".into(),
            autocorrect: true,
            format_on_save: true,
            autosave: false,
            diff_view: "editor".into(),
            composer_mascot: true,
            empty_session_games: true,
            show_remaining_usage: false,
            mask_emails: true,
            idle_agents: 3,
            keep_awake: "off".into(),
            claude_hooks: true,
            binary_paths: HashMap::new(),
            disabled_skill_paths: Vec::new(),
            skill_source_paths: Vec::new(),
            default_provider: None,
            default_models: HashMap::new(),
            last_provider: None,
            last_model_settings: HashMap::new(),
            hidden_providers: Vec::new(),
            project_providers: HashMap::new(),
            favorite_models: Vec::new(),
            recent_models: Vec::new(),
            model_picker_tab: None,
            default_access: None,
            provider_efforts: HashMap::new(),
            pilots: true,
            rail_pilots_pinned: false,
            pilot_menu_bar_icon: true,
            show_archived: false,
            dictation_model: gespan_speech::catalog::PARAKEET_V2_EN.id.into(),
            dictation_words: String::new(),
            keybindings: HashMap::new(),
        }
    }
}

impl Preferences {
    pub(crate) fn apply_workspace_theme(&mut self, id: String) {
        let color: gpui::Hsla = gpui::rgb(super::palette(&id, false).surface).into();
        self.theme_hue = (color.h * 360.).round() as u32;
        self.theme_saturation = (color.s * 100.).round() as u32;
        self.theme_dark_lightness = ((color.l * 100.).round() as u32).min(30);
        self.workspace_theme = id;
    }

    /// SettingsView's Restore defaults: the Appearance page only.
    pub(crate) fn restore_appearance(&mut self) {
        let d = Self::default();
        self.theme = d.theme;
        self.accent_color = d.accent_color;
        self.workspace_theme = d.workspace_theme;
        self.sidebar_opacity = d.sidebar_opacity;
        self.sidebar_blur = d.sidebar_blur;
        self.theme_hue = d.theme_hue;
        self.theme_saturation = d.theme_saturation;
        self.theme_dark_lightness = d.theme_dark_lightness;
        self.body_glass = d.body_glass;
        self.inset_glass = d.inset_glass;
        self.show_excluded_files = d.show_excluded_files;
        self.chat_background_empty = d.chat_background_empty;
        self.chat_background_session = d.chat_background_session;
        self.chat_background_scope = d.chat_background_scope;
        self.diff_palette = d.diff_palette;
        self.chat_background_effect = d.chat_background_effect;
        self.chat_background = d.chat_background;
        self.ui_scale = d.ui_scale;
        self.collapsed_project_rail = d.collapsed_project_rail;
    }
}

/// [`Preferences::apply_last_controls`] for work holding only the controls:
/// the chosen model is looked up in the cached catalog by `settings.account`
/// and `settings.model`, so set those first.
pub(crate) fn apply_last_controls_to(
    last: &HashMap<String, String>,
    settings: &mut gespan_core::agent::Settings,
    kind: gespan_core::provider::Kind,
) {
    let model =
        gespan_core::catalog::cached_models_for(kind, &settings.account).and_then(|models| {
            match settings.model.as_deref() {
                Some(id) => models
                    .into_iter()
                    .find(|m| m.id == id || m.alias_target.as_deref() == Some(id)),
                None => {
                    let default = models.iter().position(|m| m.default).unwrap_or(0);
                    models.into_iter().nth(default)
                }
            }
        });
    apply_controls(last, settings, kind, model.as_ref());
}

/// Merges remembered controls only where the chosen model defines them with
/// a valid value (desktop `mergeModelSettings`). Without a catalog, only the
/// provider's known controls apply; anything else is dropped. Effort lives in
/// `provider_efforts` and is never merged here.
pub(crate) fn apply_controls(
    last: &HashMap<String, String>,
    settings: &mut gespan_core::agent::Settings,
    kind: gespan_core::provider::Kind,
    model: Option<&gespan_core::catalog::Model>,
) {
    use gespan_core::provider::Kind;
    let boolean = |value: &str| matches!(value, "true" | "false");
    for (key, value) in last {
        let key = key.as_str();
        if matches!(
            key,
            "effort" | "reasoning" | "reasoningEffort" | "reasoning_effort"
        ) {
            continue;
        }
        match model {
            Some(model) => match key {
                "fast" if model.fast && boolean(value) => settings.fast = value == "true",
                "thinking" if model.thinking && boolean(value) => {
                    settings.thinking = value.parse().ok()
                }
                "serviceTier" if model.tiers.iter().any(|(id, _)| id == value) => {
                    settings.service_tier = Some(value.clone())
                }
                _ if model
                    .controls
                    .iter()
                    .any(|c| c.id == key && c.choices.iter().any(|(v, _)| v == value)) =>
                {
                    settings.model_settings.insert(key.into(), value.clone());
                }
                _ => {}
            },
            None => match (kind, key) {
                (Kind::Claude, "fast") if boolean(value) => settings.fast = value == "true",
                (Kind::Claude, "thinking") if boolean(value) => {
                    settings.thinking = value.parse().ok()
                }
                (Kind::Claude, "context") => {
                    settings.model_settings.insert(key.into(), value.clone());
                }
                (Kind::Codex, "serviceTier") => settings.service_tier = Some(value.clone()),
                _ => {}
            },
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ProviderDefaults {
    pub provider: Option<String>,
    pub models: HashMap<String, String>,
    pub hidden: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ModelChoice {
    pub provider: String,
    pub model: Option<String>,
}

impl gpui::Global for Preferences {}
impl Preferences {
    /// The last used provider; older preferences fall back to their default.
    pub(crate) fn last_used_provider(&self) -> Option<gespan_core::provider::Kind> {
        self.last_provider
            .as_deref()
            .or(self.default_provider.as_deref())
            .and_then(gespan_core::provider::Kind::parse)
    }
    /// A user's pick in any picker: it becomes the last used provider, and the
    /// model that provider's last model (`None`, its Default, forgets it).
    pub(crate) fn remember_choice(
        &mut self,
        kind: gespan_core::provider::Kind,
        model: Option<String>,
    ) {
        self.last_provider = Some(kind.id().into());
        match model {
            Some(model) => self.default_models.insert(kind.id().into(), model),
            None => self.default_models.remove(kind.id()),
        };
    }
    /// Settings → Providers "Use by default": also the last used choice.
    pub(crate) fn use_by_default(&mut self, kind: gespan_core::provider::Kind) {
        self.default_provider = Some(kind.id().into());
        self.last_provider = Some(kind.id().into());
    }
    /// A composer control changed; `None` clears it.
    pub(crate) fn remember_control(&mut self, key: &str, value: Option<String>) {
        match value {
            Some(value) => self.last_model_settings.insert(key.into(), value),
            None => self.last_model_settings.remove(key),
        };
    }
    /// Merges the last used controls the chosen model supports into a new
    /// session's settings (desktop `preferredModelSettings`).
    pub(crate) fn apply_last_controls(
        &self,
        settings: &mut gespan_core::agent::Settings,
        kind: gespan_core::provider::Kind,
    ) {
        apply_last_controls_to(&self.last_model_settings, settings, kind);
    }
    pub(crate) fn hidden_for(&self, project: &str) -> Vec<String> {
        let mut hidden = self.hidden_providers.clone();
        if let Some(settings) = self.project_providers.get(project) {
            hidden.extend(settings.hidden.clone());
        }
        hidden.sort();
        hidden.dedup();
        hidden
    }
}

#[cfg(test)]
mod keep_awake_tests {
    use super::Preferences;
    use gespan_keep_awake::Mode;

    #[test]
    fn saved_preferences_from_before_keep_awake_load_as_off() {
        let old: Preferences = serde_json::from_str(r#"{"idle_agents":2}"#).unwrap();
        assert_eq!(old.idle_agents, 2);
        assert_eq!(Mode::parse(&old.keep_awake), Some(Mode::Off));
        let saved: Preferences =
            serde_json::from_str(r#"{"keep_awake":"agents_running"}"#).unwrap();
        assert_eq!(Mode::parse(&saved.keep_awake), Some(Mode::AgentsRunning));
    }
}

#[cfg(test)]
mod last_controls_tests {
    use super::apply_controls;
    use gespan_core::agent::Settings;
    use gespan_core::catalog::{Model, ModelControl};
    use gespan_core::provider::Kind;
    use std::collections::HashMap;

    fn model(fast: bool, tiers: &[&str], controls: &[(&str, &[&str])]) -> Model {
        Model {
            id: "m".into(),
            name: "M".into(),
            alias_target: None,
            efforts: Vec::new(),
            default_effort: None,
            fast,
            thinking: false,
            default: true,
            tiers: tiers
                .iter()
                .map(|t| (t.to_string(), t.to_string()))
                .collect(),
            controls: controls
                .iter()
                .map(|(id, choices)| ModelControl {
                    id: id.to_string(),
                    name: id.to_string(),
                    current: String::new(),
                    choices: choices
                        .iter()
                        .map(|c| (c.to_string(), c.to_string()))
                        .collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn remembered_controls_apply_only_where_the_model_supports_them() {
        let last: HashMap<String, String> = [
            ("fast", "true"),
            ("thinking", "true"),
            ("context", "1m"),
            ("serviceTier", "flex"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let apply = |kind: Kind, model: Option<&Model>| {
            let mut settings = Settings::default();
            apply_controls(&last, &mut settings, kind, model);
            settings
        };
        // A Cursor model defining a boolean thinking control and other
        // context sizes: only the valid thinking value carries over.
        let cursor = model(
            false,
            &[],
            &[("thinking", &["false", "true"]), ("context", &["200k"])],
        );
        let s = apply(Kind::Cursor, Some(&cursor));
        assert_eq!(
            s.model_settings,
            HashMap::from([("thinking".into(), "true".into())])
        );
        assert!(!s.fast && s.thinking.is_none() && s.service_tier.is_none());
        // A model with fast and the remembered tier takes both.
        let codex = model(true, &["flex"], &[]);
        let s = apply(Kind::Codex, Some(&codex));
        assert!(s.fast);
        assert_eq!(s.service_tier.as_deref(), Some("flex"));
        assert!(s.model_settings.is_empty());
        // No catalog yet: only the provider's known controls.
        let s = apply(Kind::Claude, None);
        assert!(s.fast);
        assert_eq!(s.thinking, Some(true));
        assert_eq!(s.model_settings["context"], "1m");
        for unknown in [Kind::Omp, Kind::Pi, Kind::Cursor, Kind::Kiro] {
            let s = apply(unknown, None);
            assert!(!s.fast, "{unknown:?}");
            assert_eq!(s.thinking, None, "{unknown:?}");
            assert!(s.model_settings.is_empty(), "{unknown:?}");
        }
        // A level-valued thinking (Pi, Omp) is not Claude's boolean.
        let pi = model(false, &[], &[("thinking", &["low", "high"])]);
        assert!(apply(Kind::Pi, Some(&pi)).model_settings.is_empty());
    }
}
