//! First-run setup: the card a fresh install opens on, which walks through
//! coding agents, the look and the first project.
mod agents;
pub(crate) mod card;
mod look;
mod project;
pub(crate) mod rules;
#[cfg(test)]
pub(crate) mod test_support;
mod tools;
pub(crate) mod ui;
mod welcome;

/// The card setup is on, saved so a relaunch resumes there. A workspace with
/// none has finished setup, or never needed it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum SetupStep {
    Welcome,
    Agents,
    /// Shared tools: the MCP hub. Saved resume steps from before it load by
    /// name, so adding it moved no one's place.
    Tools,
    Look,
    Project,
}

#[cfg(test)]
mod tests {
    use super::SetupStep;

    #[test]
    fn a_saved_step_from_before_tools_still_loads() {
        for (json, step) in [
            ("\"Agents\"", SetupStep::Agents),
            ("\"Look\"", SetupStep::Look),
            ("\"Project\"", SetupStep::Project),
        ] {
            assert_eq!(serde_json::from_str::<SetupStep>(json).unwrap(), step);
        }
    }

    #[test]
    fn the_header_counts_four_steps() {
        assert_eq!(super::ui::step_label(1), "Step 1 of 4");
        assert_eq!(super::ui::step_label(3), "Step 3 of 4");
    }
}
