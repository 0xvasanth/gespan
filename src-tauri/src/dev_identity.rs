//! Development builds run as "Gespan Dev". Tauri derives every app directory
//! (gespan.db, checkpoints, notes, logs) from the identifier, and the macOS
//! webview keys localStorage by bundle identifier, so a dev build never opens
//! the installed app's data while that app is building Gespan itself.

pub(crate) const IDENTIFIER: &str = "dev.gespan.desktop.dev";
pub(crate) const PRODUCT_NAME: &str = "Gespan Dev";

pub(crate) fn apply(config: &mut tauri::Config) {
    config.identifier = IDENTIFIER.into();
    config.product_name = Some(PRODUCT_NAME.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRODUCTION_IDENTIFIER: &str = "dev.gespan.desktop";

    #[test]
    fn dev_identity_moves_app_data_away_from_the_installed_app() {
        let mut config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config.identifier, PRODUCTION_IDENTIFIER);
        apply(&mut config);
        assert_eq!(config.identifier, IDENTIFIER);
        assert_eq!(config.product_name.as_deref(), Some(PRODUCT_NAME));
    }
}
