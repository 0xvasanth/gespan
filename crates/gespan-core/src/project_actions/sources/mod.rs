//! Where actions are read from: one [`Source`] per editor or agent app whose
//! config lists commands. Parsers fill [`Found`] only; `tasks_json::resolve`
//! fills in variables, quoting and `cwd` afterwards.
use super::{Ctx, Found};

pub(crate) mod agent_apps;
pub(crate) mod editors;
pub(crate) mod runners;

/// A tool's config file that lists commands.
pub trait Source: Sync {
    /// Stable. It starts every [`super::Action::id`] from this source.
    fn id(&self) -> &'static str;
    /// Relative to the checkout. The first one that exists is read.
    fn files(&self) -> &'static [&'static str];
    /// The actions in `text`, which was read from `file`, one of [`Self::files`].
    fn parse(&self, ctx: &Ctx, file: &str, text: &str) -> Result<Vec<Found>, String>;
}

/// `text` as JSON, for a file that must hold an object.
pub(super) fn json_object(text: &str) -> Result<serde_json::Value, String> {
    let value: serde_json::Value =
        serde_json::from_str(&super::jsonc::strip(text).json).map_err(|error| error.to_string())?;
    if value.is_object() {
        Ok(value)
    } else {
        Err("It isn't a JSON object".into())
    }
}

/// `text` as TOML, shaped like the JSON it would be written as.
pub(super) fn toml_object(text: &str) -> Result<serde_json::Value, String> {
    let value: toml::Value = toml::from_str(text).map_err(|error| error.to_string())?;
    serde_json::to_value(value).map_err(|error| error.to_string())
}

/// A string field, if it is one and isn't blank.
pub(super) fn string(value: &serde_json::Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?;
    (!text.trim().is_empty()).then(|| text.to_string())
}

/// The string items of a list field.
pub(super) fn strings(value: &serde_json::Value, key: &str) -> Vec<String> {
    match value.get(key) {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// The string entries of an object field, in order.
pub(super) fn env(value: &serde_json::Value, key: &str) -> Vec<(String, String)> {
    match value.get(key) {
        Some(serde_json::Value::Object(env)) => env
            .iter()
            .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_string())))
            .collect(),
        _ => Vec::new(),
    }
}
