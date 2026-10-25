//! The shared scrubbing fixtures, for any crate's tests to run against:
//! `gespan-redact = { path = "…", features = ["fixtures"] }` as a dev-dependency.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::{Config, Redactor};

/// `fixtures/redaction.json`, compiled in.
pub const JSON: &str = include_str!("../fixtures/redaction.json");

#[derive(Debug, Deserialize)]
pub struct FixtureFile {
    pub version: u32,
    /// A machine per platform: its home, projects and known secrets.
    pub contexts: BTreeMap<String, Context>,
    pub cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
pub struct Context {
    pub home: Option<String>,
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// `input` is a string for [`Redactor::redact_text`].
    Text,
    /// `input` is any JSON for [`Redactor::redact_value`].
    Json,
}

#[derive(Debug, Deserialize)]
pub struct Case {
    pub name: String,
    pub context: String,
    pub kind: Kind,
    pub input: Value,
    pub expected: Value,
}

pub fn load() -> FixtureFile {
    serde_json::from_str(JSON).expect("fixtures/redaction.json is valid")
}

impl FixtureFile {
    /// The redactor for one of the file's contexts.
    pub fn redactor(&self, context: &str) -> Redactor {
        let context = self
            .contexts
            .get(context)
            .unwrap_or_else(|| panic!("no fixture context {context:?}"));
        Redactor::new(Config {
            home: context.home.as_ref().map(PathBuf::from),
            projects: context.projects.iter().map(PathBuf::from).collect(),
            secrets: context.secrets.clone(),
        })
    }
}

impl Case {
    /// Scrubs this case's input.
    pub fn run(&self, redactor: &Redactor) -> Value {
        self.run_on(redactor, &self.input)
    }

    /// Scrubs `value` the way this case's kind says.
    pub fn run_on(&self, redactor: &Redactor, value: &Value) -> Value {
        match self.kind {
            Kind::Text => {
                let text = value
                    .as_str()
                    .unwrap_or_else(|| panic!("text fixture {:?} needs a string", self.name));
                Value::String(redactor.redact_text(text))
            }
            Kind::Json => redactor.redact_value(value.clone()),
        }
    }
}
