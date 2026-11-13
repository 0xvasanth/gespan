//! Where the Tauri desktop app keeps its database, and its model ids as
//! native ones.
use std::path::PathBuf;

/// Tauri's application-data location, keyed by its bundle identifier.
pub fn gespan_database() -> PathBuf {
    #[cfg(target_os = "macos")]
    let base = crate::accounts::home()
        .unwrap_or_default()
        .join("Library/Application Support");
    #[cfg(windows)]
    let base = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default());
    #[cfg(not(any(target_os = "macos", windows)))]
    // Tauri ignores an unset or relative XDG_DATA_HOME, so this must too.
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            crate::accounts::home()
                .unwrap_or_default()
                .join(".local/share")
        });
    base.join("dev.gespan.desktop/gespan.db")
}

pub fn native_model(model: &str, provider: &str) -> Option<String> {
    let model = model.trim();
    let model = model.strip_prefix(&format!("{provider}:")).unwrap_or(model);
    let model = model.split('[').next().unwrap_or(model);
    if model.is_empty() || model == "default" {
        return None;
    }
    Some(match crate::provider::Kind::parse(provider) {
        Some(kind) => kind.provider().native_model_id(model),
        None => model.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_picker_keys_are_translated_to_provider_model_ids() {
        assert_eq!(
            native_model("codex:gpt-6.1-sol", "codex").as_deref(),
            Some("gpt-6.1-sol")
        );
        assert_eq!(
            native_model("claude:opus-4.8[effort=high]", "claude").as_deref(),
            Some("claude-opus-4-8")
        );
        assert_eq!(
            native_model("claude:sonnet", "claude").as_deref(),
            Some("sonnet")
        );
        assert_eq!(
            native_model("opencode:provider/model", "opencode").as_deref(),
            Some("provider/model")
        );
        assert_eq!(native_model("codex:default", "codex"), None);
    }
}
