//! First-run setup: the card a fresh install opens on, which walks through
//! coding agents, the look and the first project.
mod agents;
pub(crate) mod card;
mod look;
mod project;
pub(crate) mod rules;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod ui;
mod welcome;

/// The card setup is on, saved so a relaunch resumes there. A workspace with
/// none has finished setup, or never needed it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum SetupStep {
    Welcome,
    Agents,
    Look,
    Project,
}
