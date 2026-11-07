//! A project's everyday commands, read from the files that already list them:
//! Gespan's own `.gespan/tasks.json`, other tools' configs and task runners.
//!
//! Reading a file never runs anything. An action runs only when a person or an
//! agent asks for it.
use serde::Serialize;
use std::path::{Path, PathBuf};

// Detection, which lands next, is their first caller outside the tests.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod jsonc;
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod tasks_json;

/// The icons an action can show. Anything else shows as `play`.
pub const ACTION_ICONS: [&str; 6] = ["play", "test-tube", "hammer", "rocket", "bug", "package"];

/// An action ready to list and run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    /// `<source id>:<label>`, such as `gespan:Test` or `package.json:dev`.
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    pub source: &'static str,
    /// The file it came from, as shown, such as `.vscode/tasks.json`.
    pub file: String,
    /// The shell line to run.
    pub command: String,
    /// Absolute, inside the checkout.
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// It keeps running, such as a dev server.
    pub background: bool,
    /// One of [`ACTION_ICONS`].
    pub icon: &'static str,
    /// Why it can't run. It is still listed, greyed out.
    pub blocked: Option<String>,
    /// Shown beside it, such as "Dependencies aren't run".
    pub note: Option<String>,
}

/// What a source parser yields, before variables, quoting and `cwd` resolve.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    /// False for a `process` task, whose program is quoted like its args.
    pub shell: bool,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub detail: Option<String>,
    pub background: bool,
    pub icon: Option<String>,
    pub hide: bool,
    pub blocked: Option<String>,
    pub note: Option<String>,
}

/// What parsing and resolving need to know about the project.
#[derive(Clone, Copy, Debug)]
pub struct Ctx<'a> {
    /// The active checkout: the session's worktree, else the project root.
    pub checkout: &'a Path,
    /// `bun`, `pnpm`, `yarn` or `npm`, from the lockfile.
    pub package_manager: &'static str,
}

#[cfg(test)]
mod tests;
