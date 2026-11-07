//! Where actions are read from: one [`Source`] per editor or agent app whose
//! config lists commands. Parsers fill [`Found`] only; `tasks_json::resolve`
//! fills in variables, quoting and `cwd` afterwards.
use super::{Ctx, Found};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use serde_json::{Map, Value};

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

/// The keys of the object at `path` in a JSON or JSONC file, in file order.
/// `serde_json` sorts an object's keys, so they are read from the text again.
pub(super) fn json_keys(text: &str, path: &[&str]) -> Vec<String> {
    let json = super::jsonc::strip(text).json;
    let mut keys = Vec::new();
    let mut parser = serde_json::Deserializer::from_str(&json);
    // On an error, the keys read so far still count; the rest go last.
    let _ = KeysAt {
        path,
        keys: &mut keys,
    }
    .deserialize(&mut parser);
    keys
}

/// The keys of the table at `path` in a TOML file, in file order. The parser
/// sorts them too, but each key keeps where it was written.
pub(super) fn toml_keys(text: &str, path: &[&str]) -> Vec<String> {
    use toml::de::{DeTable, DeValue};
    let Ok(root) = DeTable::parse(text) else {
        return Vec::new();
    };
    let mut table = root.get_ref();
    for name in path {
        let found = table.iter().find_map(|(key, value)| match value.get_ref() {
            DeValue::Table(inner) if key.get_ref() == name => Some(inner),
            _ => None,
        });
        let Some(inner) = found else {
            return Vec::new();
        };
        table = inner;
    }
    let mut keys: Vec<_> = table.keys().collect();
    keys.sort_by_key(|key| key.span().start);
    keys.into_iter()
        .map(|key| key.get_ref().to_string())
        .collect()
}

/// `map`'s entries in `order`, then any that `order` misses, sorted.
pub(super) fn in_order<'a>(
    map: &'a Map<String, Value>,
    order: &[String],
) -> Vec<(&'a String, &'a Value)> {
    let mut entries: Vec<_> = Vec::with_capacity(map.len());
    for key in order {
        if let Some(entry) = map.get_key_value(key) {
            if !entries.iter().any(|(seen, _)| *seen == entry.0) {
                entries.push(entry);
            }
        }
    }
    if entries.len() < map.len() {
        let rest: Vec<_> = map.iter().filter(|(key, _)| !order.contains(key)).collect();
        entries.extend(rest);
    }
    entries
}

/// Collects the keys of the object at `path`, skipping everything else.
struct KeysAt<'a> {
    path: &'a [&'a str],
    keys: &'a mut Vec<String>,
}

impl<'de> DeserializeSeed<'de> for KeysAt<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, from: D) -> Result<(), D::Error> {
        from.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for KeysAt<'_> {
    type Value = ();
    fn expecting(&self, out: &mut std::fmt::Formatter) -> std::fmt::Result {
        out.write_str("an object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            match self.path.split_first() {
                None => {
                    map.next_value::<IgnoredAny>()?;
                    self.keys.push(key);
                }
                Some((name, rest)) if *name == key => map.next_value_seed(KeysAt {
                    path: rest,
                    keys: &mut *self.keys,
                })?,
                Some(_) => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
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
