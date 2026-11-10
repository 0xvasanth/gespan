//! Borrowing views of session-file lines: only the fields sync reads are
//! kept, and everything else (tool output, file snapshots) is skipped
//! without being built into a JSON tree.
use std::borrow::Cow;
use std::fmt;

use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;

/// A string field, or `None` when it holds anything else.
#[derive(Debug, Default)]
pub(super) struct Str<'a>(pub(super) Option<Cow<'a, str>>);

impl<'a> Str<'a> {
    pub(super) fn get(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for Str<'a> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text: Str<'de> = deserializer.deserialize_any(Lenient(Str))?;
        Ok(text)
    }
}

/// A message's `content`: a plain string, a list of blocks, or something
/// else, which is ignored.
#[derive(Debug, Default)]
pub(super) enum Content<'a> {
    Text(Cow<'a, str>),
    Blocks(Vec<Block<'a>>),
    #[default]
    Other,
}

impl<'de: 'a, 'a> Deserialize<'de> for Content<'a> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Content<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("message content")
            }
            fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(Content::Text(Cow::Borrowed(v)))
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(Content::Text(Cow::Owned(v.to_owned())))
            }
            fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
                Ok(Content::Text(Cow::Owned(v)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut blocks = Vec::new();
                while let Some(block) = seq.next_element::<Block>()? {
                    blocks.push(block);
                }
                Ok(Content::Blocks(blocks))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                drain_map(map)?;
                Ok(Content::Other)
            }
            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(Content::Other)
            }
            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(Content::Other)
            }
            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(Content::Other)
            }
            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(Content::Other)
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(Content::Other)
            }
        }
        let content: Content<'de> = deserializer.deserialize_any(V)?;
        Ok(content)
    }
}

/// One content block; a block that isn't an object reads as an empty one.
#[derive(Debug, Default)]
pub(super) struct Block<'a> {
    pub(super) kind: Str<'a>,
    pub(super) text: Str<'a>,
}

impl<'de: 'a, 'a> Deserialize<'de> for Block<'a> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Fields<'a> {
            #[serde(rename = "type", borrow, default)]
            kind: Str<'a>,
            #[serde(borrow, default)]
            text: Str<'a>,
        }
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Block<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a content block")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                let fields = Fields::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(Block {
                    kind: fields.kind,
                    text: fields.text,
                })
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
                drain_seq(seq)?;
                Ok(Block::default())
            }
            fn visit_borrowed_str<E>(self, _: &'de str) -> Result<Self::Value, E> {
                Ok(Block::default())
            }
            fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
                Ok(Block::default())
            }
            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(Block::default())
            }
            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(Block::default())
            }
            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(Block::default())
            }
            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(Block::default())
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(Block::default())
            }
        }
        let block: Block<'de> = deserializer.deserialize_any(V)?;
        Ok(block)
    }
}

/// Reads a string, and turns any other value into `None`.
struct Lenient<F>(F);

impl<'de, T, F: FnOnce(Option<Cow<'de, str>>) -> T> Visitor<'de> for Lenient<F> {
    type Value = T;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any value")
    }
    fn visit_borrowed_str<E>(self, v: &'de str) -> Result<T, E> {
        Ok((self.0)(Some(Cow::Borrowed(v))))
    }
    fn visit_str<E>(self, v: &str) -> Result<T, E> {
        Ok((self.0)(Some(Cow::Owned(v.to_owned()))))
    }
    fn visit_string<E>(self, v: String) -> Result<T, E> {
        Ok((self.0)(Some(Cow::Owned(v))))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<T, A::Error> {
        drain_seq(seq)?;
        Ok((self.0)(None))
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
        drain_map(map)?;
        Ok((self.0)(None))
    }
    fn visit_bool<E>(self, _: bool) -> Result<T, E> {
        Ok((self.0)(None))
    }
    fn visit_i64<E>(self, _: i64) -> Result<T, E> {
        Ok((self.0)(None))
    }
    fn visit_u64<E>(self, _: u64) -> Result<T, E> {
        Ok((self.0)(None))
    }
    fn visit_f64<E>(self, _: f64) -> Result<T, E> {
        Ok((self.0)(None))
    }
    fn visit_unit<E>(self) -> Result<T, E> {
        Ok((self.0)(None))
    }
}

fn drain_seq<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<(), A::Error> {
    while seq.next_element::<IgnoredAny>()?.is_some() {}
    Ok(())
}

fn drain_map<'de, A: MapAccess<'de>>(mut map: A) -> Result<(), A::Error> {
    while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
    Ok(())
}

/// A flag that reads as `false` unless it is literally `true`.
#[derive(Debug, Default)]
pub(super) struct Flag(pub(super) bool);

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Flag(matches!(
            serde_json::Value::deserialize(deserializer)?,
            serde_json::Value::Bool(true)
        )))
    }
}

/// One line of a Claude Code session file.
#[derive(Debug, Deserialize)]
pub(super) struct ClaudeLine<'a> {
    #[serde(rename = "type", borrow, default)]
    pub(super) kind: Str<'a>,
    #[serde(borrow, default)]
    pub(super) uuid: Str<'a>,
    #[serde(rename = "parentUuid", borrow, default)]
    pub(super) parent_uuid: Str<'a>,
    #[serde(rename = "logicalParentUuid", borrow, default)]
    pub(super) logical_parent_uuid: Str<'a>,
    #[serde(rename = "isSidechain", default)]
    pub(super) is_sidechain: Flag,
    #[serde(rename = "isMeta", default)]
    pub(super) is_meta: Flag,
    #[serde(rename = "isCompactSummary", default)]
    pub(super) is_compact_summary: Flag,
    #[serde(borrow, default)]
    pub(super) message: Option<ClaudeMessage<'a>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeMessage<'a> {
    #[serde(borrow, default)]
    pub(super) id: Str<'a>,
    #[serde(borrow, default)]
    pub(super) content: Content<'a>,
}

/// Just the `uuid` of a Claude line.
#[derive(Debug, Deserialize)]
pub(super) struct UuidOnly<'a> {
    #[serde(borrow, default)]
    pub(super) uuid: Str<'a>,
}

/// One line of a Codex rollout file.
#[derive(Debug, Deserialize)]
pub(super) struct CodexLine<'a> {
    #[serde(rename = "type", borrow, default)]
    pub(super) kind: Str<'a>,
    #[serde(borrow, default)]
    pub(super) payload: Option<CodexPayload<'a>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexPayload<'a> {
    #[serde(rename = "type", borrow, default)]
    pub(super) kind: Str<'a>,
    #[serde(borrow, default)]
    pub(super) message: Str<'a>,
    #[serde(default)]
    pub(super) num_turns: Option<IgnoredOr<u64>>,
    #[serde(borrow, default)]
    pub(super) role: Str<'a>,
    #[serde(borrow, default)]
    pub(super) content: Content<'a>,
}

/// A number, or `None` when the field holds anything else.
#[derive(Debug)]
pub(super) struct IgnoredOr<T>(pub(super) Option<T>);

impl<'de> Deserialize<'de> for IgnoredOr<u64> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(IgnoredOr(
            serde_json::Value::deserialize(deserializer)?.as_u64(),
        ))
    }
}
