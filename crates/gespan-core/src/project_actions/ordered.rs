//! JSON that keeps each object's keys in file order. `serde_json` here sorts
//! them, and a shared file should change only where it was edited.
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use serde_json::Value;
use std::collections::HashMap;

/// A JSON value whose objects keep their key order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Json {
    /// A string, number, bool or null.
    Scalar(Value),
    Array(Vec<Json>),
    Object(Object),
}

/// An object's entries, in order, each key once.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Object(Vec<(String, Json)>);

impl Object {
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    /// Sets `key` where it already is, or last.
    pub fn insert(&mut self, key: &str, value: impl Into<Json>) {
        let value = value.into();
        match self.get_mut(key) {
            Some(slot) => *slot = value,
            None => self.0.push((key.to_string(), value)),
        }
    }
    /// Sets `key` first, unless it is already there.
    pub fn insert_first(&mut self, key: &str, value: impl Into<Json>) {
        if self.get(key).is_none() {
            self.0.insert(0, (key.to_string(), value.into()));
        }
    }
    pub fn remove(&mut self, key: &str) {
        self.0.retain(|(k, _)| k != key);
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Json {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Scalar(value) => value.as_str(),
            _ => None,
        }
    }
    pub fn as_object_mut(&mut self) -> Option<&mut Object> {
        match self {
            Json::Object(object) => Some(object),
            _ => None,
        }
    }
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Json>> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }
}

impl From<Object> for Json {
    fn from(object: Object) -> Self {
        Json::Object(object)
    }
}

impl From<&str> for Json {
    fn from(text: &str) -> Self {
        Json::Scalar(text.into())
    }
}

impl From<String> for Json {
    fn from(text: String) -> Self {
        Json::Scalar(text.into())
    }
}

impl From<bool> for Json {
    fn from(flag: bool) -> Self {
        Json::Scalar(flag.into())
    }
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, to: S) -> Result<S::Ok, S::Error> {
        match self {
            Json::Scalar(value) => value.serialize(to),
            Json::Array(items) => {
                let mut seq = to.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Json::Object(Object(entries)) => {
                let mut map = to.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        from.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;
    fn expecting(&self, out: &mut std::fmt::Formatter) -> std::fmt::Result {
        out.write_str("JSON")
    }
    fn visit_bool<E>(self, flag: bool) -> Result<Json, E> {
        Ok(flag.into())
    }
    fn visit_i64<E>(self, n: i64) -> Result<Json, E> {
        Ok(Json::Scalar(n.into()))
    }
    fn visit_u64<E>(self, n: u64) -> Result<Json, E> {
        Ok(Json::Scalar(n.into()))
    }
    fn visit_f64<E>(self, n: f64) -> Result<Json, E> {
        Ok(Json::Scalar(n.into()))
    }
    fn visit_str<E>(self, text: &str) -> Result<Json, E> {
        Ok(text.into())
    }
    fn visit_string<E>(self, text: String) -> Result<Json, E> {
        Ok(text.into())
    }
    fn visit_unit<E>(self) -> Result<Json, E> {
        Ok(Json::Scalar(Value::Null))
    }
    fn visit_none<E>(self) -> Result<Json, E> {
        Ok(Json::Scalar(Value::Null))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut entries: Vec<(String, Json)> = Vec::new();
        // A repeated key keeps its first place and its last value, as
        // `serde_json` reads it. Indexed, so a huge object stays linear.
        let mut at: HashMap<String, usize> = HashMap::new();
        while let Some((key, value)) = map.next_entry::<String, Json>()? {
            match at.get(&key) {
                Some(&ix) => entries[ix].1 = value,
                None => {
                    at.insert(key.clone(), entries.len());
                    entries.push((key, value));
                }
            }
        }
        Ok(Json::Object(Object(entries)))
    }
}
