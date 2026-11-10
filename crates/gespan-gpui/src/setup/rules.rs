//! What first-run setup decides without a window: when it shows, what the
//! agents card says about missing agents, which agent a skip picks, and which
//! folder it suggests.
use gespan_core::provider::Kind;

/// Setup shows only on a fresh install: no saved workspace and no sessions.
/// Data moved from the Tauri app has sessions, and any saved workspace is an
/// existing install.
pub(crate) fn first_run(saved: Option<&str>, has_sessions: bool) -> bool {
    saved.is_none() && !has_sessions
}

/// Whether the saved workspace is partway through setup, as after quitting
/// on a card.
pub(crate) fn setup_pending(saved: Option<&str>) -> bool {
    saved
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|state| !state["setup"].is_null())
}

/// The saved workspace with setup pending on its first card.
pub(crate) fn with_pending_setup(saved: Option<String>) -> String {
    let mut state = saved
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    state["setup"] = serde_json::json!(crate::setup::SetupStep::Welcome);
    state.to_string()
}

/// The line under the agents list naming the agents that are missing.
pub(crate) fn not_found_line(missing: &[&str]) -> String {
    match missing {
        [] => "Every supported agent is installed".into(),
        [one] => format!("Not found: {one}"),
        [a, b] => format!("Not found: {a} and {b}"),
        [a, b, rest @ ..] => format!("Not found: {a}, {b} and {} more", rest.len()),
    }
}

pub(crate) fn found_count(installed: usize, total: usize) -> String {
    format!("{installed} of {total} installed")
}

/// The agent new threads start with when the agents card is skipped: the
/// first signed-in one, else the first installed, in `Kind::ALL` order.
pub(crate) fn skip_default(
    installed: &[Kind],
    signed_in: impl Fn(Kind) -> Option<bool>,
) -> Option<Kind> {
    let ordered: Vec<Kind> = Kind::ALL
        .into_iter()
        .filter(|kind| installed.contains(kind))
        .collect();
    ordered
        .iter()
        .copied()
        .find(|kind| signed_in(*kind) == Some(true))
        .or_else(|| ordered.first().copied())
}

/// The launch folder, offered as the first project unless it is the root or
/// home folder a Dock or Finder launch starts in.
pub(crate) fn suggested_folder(launch: &str, home: &str) -> Option<String> {
    let trim = |path: &str| path.trim_end_matches(['/', '\\']).to_string();
    let folder = trim(launch);
    (!folder.is_empty() && folder != trim(home)).then(|| launch.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::SetupStep;
    use crate::workspace::Workspace;
    use gespan_core::provider::Kind;

    #[test]
    fn first_run_only_without_saved_state_or_sessions() {
        assert!(first_run(None, false));
        // Data moved from the Tauri app has sessions but no workspace row.
        assert!(!first_run(None, true));
        // Any saved workspace is an existing install.
        assert!(!first_run(Some("{}"), false));
    }

    #[test]
    fn pending_setup_is_written_into_saved_state() {
        let ws = Workspace::restore(Some(&with_pending_setup(None)), &[], "/w");
        assert_eq!(ws.setup, Some(SetupStep::Welcome));
        let kept = with_pending_setup(Some(r#"{"new_layout_boxy":true}"#.into()));
        let ws = Workspace::restore(Some(&kept), &[], "/w");
        assert_eq!(ws.setup, Some(SetupStep::Welcome));
        assert!(ws.new_layout_boxy);
    }

    #[test]
    fn setup_stays_pending_across_a_relaunch() {
        assert!(setup_pending(Some(&with_pending_setup(None))));
        assert!(setup_pending(Some(r#"{"setup":"Project"}"#)));
        assert!(!setup_pending(Some(r#"{"setup":null}"#)));
        assert!(!setup_pending(Some("{}")));
        assert!(!setup_pending(None));
    }

    #[test]
    fn a_saved_workspace_without_setup_counts_as_done() {
        let ws = Workspace::restore(Some(r#"{"classic_layout":true}"#), &[], "/w");
        assert_eq!(ws.setup, None);
    }

    #[test]
    fn a_save_from_before_the_classic_layout_went_skips_setup() {
        // Keys the workspace no longer has still mark an existing install.
        let old = r#"{"classic_layout":true,"new_layout_prompt_dismissed":true}"#;
        assert!(!first_run(Some(old), false));
        assert!(!setup_pending(Some(old)));
        assert_eq!(Workspace::restore(Some(old), &[], "/w").setup, None);
        // Only no save and no sessions is a fresh install.
        assert!(first_run(None, false));
    }

    #[test]
    fn the_resume_step_survives_a_save() {
        let mut ws = Workspace::restore(Some(&with_pending_setup(None)), &[], "/w");
        ws.setup = Some(SetupStep::Project);
        let saved = serde_json::to_string(&ws).unwrap();
        let ws = Workspace::restore(Some(&saved), &[], "/w");
        assert_eq!(ws.setup, Some(SetupStep::Project));
    }

    #[test]
    fn not_found_line_names_what_is_missing() {
        assert_eq!(not_found_line(&[]), "Every supported agent is installed");
        assert_eq!(not_found_line(&["Kiro"]), "Not found: Kiro");
        assert_eq!(
            not_found_line(&["Grok", "Kiro"]),
            "Not found: Grok and Kiro"
        );
        assert_eq!(
            not_found_line(&["Grok", "Omp", "Fx", "Hermes", "Devin", "Kiro"]),
            "Not found: Grok, Omp and 4 more"
        );
        assert_eq!(found_count(6, 12), "6 of 12 installed");
    }

    #[test]
    fn skip_default_prefers_a_signed_in_agent() {
        let installed = [Kind::Codex, Kind::Claude];
        assert_eq!(
            skip_default(&installed, |k| Some(k == Kind::Claude)),
            Some(Kind::Claude)
        );
        // Without sign-in information the first installed agent, in Kind order.
        assert_eq!(skip_default(&installed, |_| None), Some(Kind::Claude));
        assert_eq!(skip_default(&[Kind::Codex], |_| None), Some(Kind::Codex));
        assert_eq!(skip_default(&[], |_| Some(true)), None);
    }

    #[test]
    fn dock_launch_folders_are_never_suggested() {
        assert_eq!(suggested_folder("/", "/Users/a"), None);
        assert_eq!(suggested_folder("/Users/a", "/Users/a"), None);
        assert_eq!(suggested_folder("/Users/a/", "/Users/a"), None);
        assert_eq!(suggested_folder("", "/Users/a"), None);
        assert_eq!(
            suggested_folder("/Users/a/langfuse", "/Users/a").as_deref(),
            Some("/Users/a/langfuse")
        );
    }

    #[test]
    fn finish_setup_selects_the_choice_and_drops_an_empty_launch_project() {
        let mut ws = Workspace::restore(Some(&with_pending_setup(None)), &[], "/Users/a");
        ws.finish_setup("/Users/a/langfuse", "/Users/a", false);
        assert_eq!(ws.setup, None);
        assert_eq!(ws.project, "/Users/a/langfuse");
        assert_eq!(ws.projects, vec!["/Users/a/langfuse".to_string()]);
    }

    #[test]
    fn finish_setup_keeps_a_launch_project_that_has_sessions() {
        let mut ws = Workspace::restore(Some(&with_pending_setup(None)), &[], "/Users/a/old");
        ws.finish_setup("/Users/a/new", "/Users/a/old", true);
        assert!(ws.projects.contains(&"/Users/a/old".to_string()));
        assert_eq!(ws.project, "/Users/a/new");
    }

    #[test]
    fn finish_setup_on_the_launch_folder_keeps_it() {
        let mut ws = Workspace::restore(Some(&with_pending_setup(None)), &[], "/Users/a/app");
        ws.finish_setup("/Users/a/app", "/Users/a/app", false);
        assert_eq!(ws.projects, vec!["/Users/a/app".to_string()]);
    }
}
