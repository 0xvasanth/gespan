//! Keep remembered picker/file choices in the native workspace sidecar.
use super::*;

impl Shell {
    pub(super) fn observe_picker_preferences(&mut self, cx: &mut Context<Self>) {
        let observer = cx.observe_global::<crate::settings::Preferences>(|this, cx| {
            let prefs = cx.global::<crate::settings::Preferences>();
            let saved = &mut this.workspace.preferences;
            if saved.model_picker_tab == prefs.model_picker_tab
                && saved.favorite_models == prefs.favorite_models
                && saved.recent_models == prefs.recent_models
                && saved.file_view_mode == prefs.file_view_mode
                && saved.default_access == prefs.default_access
                && saved.provider_efforts == prefs.provider_efforts
                && saved.last_provider == prefs.last_provider
                && saved.default_models == prefs.default_models
                && saved.last_model_settings == prefs.last_model_settings
            {
                return;
            }
            saved.model_picker_tab = prefs.model_picker_tab.clone();
            saved.favorite_models = prefs.favorite_models.clone();
            saved.recent_models = prefs.recent_models.clone();
            saved.file_view_mode = prefs.file_view_mode.clone();
            saved.default_access = prefs.default_access.clone();
            saved.provider_efforts = prefs.provider_efforts.clone();
            let last_used = saved.last_provider != prefs.last_provider
                || saved.default_models != prefs.default_models
                || saved.last_model_settings != prefs.last_model_settings;
            saved.last_provider = prefs.last_provider.clone();
            saved.default_models = prefs.default_models.clone();
            saved.last_model_settings = prefs.last_model_settings.clone();
            // Pilots start their chats and runs on the last used choice too.
            if last_used && let Some(pilots) = this.pilots.clone() {
                let prefs = this.workspace.preferences.clone();
                pilots.update(cx, |m, cx| m.set_preferences(prefs, cx));
            }
            this.sync_settings(cx);
            this.persist(cx);
        });
        self.observations.push(observer);
    }
}
