//! Which provider CLIs are installed, so a new session, Pilot chat or run
//! starts on one that can actually launch (the desktop's `firstEnabledHarness`).
use crate::settings::Preferences;
use gespan_core::provider::{self, Kind};
use gpui::{App, Global};
use std::collections::HashSet;
use std::path::PathBuf;

/// The composer's hint when detection found no provider CLI at all.
pub(crate) const NO_AGENT_HINT: &str =
    "No coding agent detected. Set one up in Settings → Agent CLIs.";

/// Until `probed`, every provider counts as installed so a cold start keeps the
/// user's preference (desktop: `hasProbedHarnessAvailability`).
#[derive(Clone, Debug, Default)]
pub(crate) struct InstalledProviders {
    kinds: HashSet<Kind>,
    probed: bool,
}
impl Global for InstalledProviders {}

impl InstalledProviders {
    pub(crate) fn probed(kinds: impl IntoIterator<Item = Kind>) -> Self {
        Self {
            kinds: kinds.into_iter().collect(),
            probed: true,
        }
    }
    /// From a discovery's `(kind, resolved binary)` pairs.
    pub(crate) fn from_binaries(binaries: &[(Kind, Option<PathBuf>)]) -> Self {
        Self::probed(
            binaries
                .iter()
                .filter(|(_, path)| path.is_some())
                .map(|(kind, _)| *kind),
        )
    }
    pub(crate) fn is_probed(&self) -> bool {
        self.probed
    }
    pub(crate) fn installed(&self, kind: Kind) -> bool {
        !self.probed || self.kinds.contains(&kind)
    }
    /// Detection ran and found no provider CLI. Never true before probing.
    pub(crate) fn none_detected(&self) -> bool {
        self.probed && self.kinds.is_empty()
    }
}

/// The app's current knowledge; unprobed when nothing has been recorded yet.
pub(crate) fn snapshot(cx: &App) -> InstalledProviders {
    cx.try_global::<InstalledProviders>()
        .cloned()
        .unwrap_or_default()
}

/// Whether detection ran and found no provider CLI.
pub(crate) fn none_detected(cx: &App) -> bool {
    cx.try_global::<InstalledProviders>()
        .is_some_and(InstalledProviders::none_detected)
}

/// A new session's provider and model. Provider: the project's default, else
/// the last used one, each only if installed and not hidden (globally or in
/// the project, as the desktop's `firstEnabledHarness`), else the first such
/// one (see [`provider::pick_installed`]). Model: the project's for that provider, else
/// its last used, else `None`, the picker's first entry (its Default).
pub(crate) fn new_session_choice_with(
    prefs: &Preferences,
    project: Option<&str>,
    installed: impl Fn(Kind) -> bool,
) -> (Kind, Option<String>) {
    let hidden = match project {
        Some(project) => prefs.hidden_for(project),
        None => prefs.hidden_providers.clone(),
    };
    let installed = |kind: Kind| installed(kind) && !hidden.iter().any(|id| id == kind.id());
    let project = project.and_then(|project| prefs.project_providers.get(project));
    let project_kind = project
        .and_then(|p| p.provider.as_deref())
        .and_then(Kind::parse);
    let last = prefs.last_used_provider();
    let kind = [project_kind, last]
        .into_iter()
        .flatten()
        .find(|kind| installed(*kind))
        .unwrap_or_else(|| provider::pick_installed(project_kind.or(last), installed));
    let model = project
        .and_then(|p| p.models.get(kind.id()))
        .or(prefs.default_models.get(kind.id()))
        .cloned();
    (kind, model)
}
/// [`new_session_choice_with`] on the UI thread.
pub(crate) fn new_session_choice(
    prefs: &Preferences,
    project: Option<&str>,
    cx: &App,
) -> (Kind, Option<String>) {
    let known = snapshot(cx);
    new_session_choice_with(prefs, project, |kind| known.installed(kind))
}
/// [`new_session_choice_with`] off the UI thread: asks the CLIs directly when
/// nothing is probed yet. Tests inject `known` and never read this PATH.
pub(crate) fn new_session_choice_resolving(
    known: &InstalledProviders,
    prefs: &Preferences,
    project: Option<&str>,
) -> (Kind, Option<String>) {
    new_session_choice_with(
        prefs,
        project,
        resolving_with(known, |kind| {
            cfg!(test) || provider::resolve(kind).is_some()
        }),
    )
}
/// For work already off the UI thread: `known` once probed, else ask the CLIs
/// with `resolve`, so a launch right after startup does not trust the
/// preference.
fn resolving_with(
    known: &InstalledProviders,
    resolve: impl Fn(Kind) -> bool,
) -> impl Fn(Kind) -> bool {
    move |kind| {
        if known.is_probed() {
            known.installed(kind)
        } else {
            resolve(kind)
        }
    }
}

/// Probes every provider off the UI thread (the same check as Settings
/// discovery) and records the result. Tests inject the set instead, so they
/// never depend on this machine's PATH.
pub(crate) fn probe(cx: &mut App) {
    if cfg!(test) {
        return;
    }
    let work = cx.background_executor().spawn(async move {
        Kind::ALL
            .into_iter()
            .map(|kind| (kind, provider::resolve(kind)))
            .collect::<Vec<_>>()
    });
    cx.spawn(async move |cx| {
        let binaries = work.await;
        cx.update(|cx| cx.set_global(InstalledProviders::from_binaries(&binaries)));
    })
    .detach();
}

/// Records a Settings discovery's probe results.
pub(crate) fn record(binaries: &[(Kind, Option<PathBuf>)], cx: &mut App) {
    if cfg!(test) {
        return;
    }
    cx.set_global(InstalledProviders::from_binaries(binaries));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The new-session provider when `preferred` was last used.
    fn pick(known: &InstalledProviders, preferred: Option<Kind>) -> Kind {
        let prefs = Preferences {
            last_provider: preferred.map(|kind| kind.id().into()),
            ..Default::default()
        };
        new_session_choice_with(&prefs, None, |kind| known.installed(kind)).0
    }

    #[test]
    fn discovery_binaries_become_the_installed_set() {
        let known = InstalledProviders::from_binaries(&[
            (Kind::Codex, None),
            (Kind::Kiro, Some("/usr/local/bin/kiro-cli".into())),
        ]);
        assert!(known.is_probed());
        assert!(known.installed(Kind::Kiro));
        assert!(!known.installed(Kind::Codex));
        assert!(!known.installed(Kind::Claude));
        assert_eq!(pick(&known, Some(Kind::Codex)), Kind::Kiro);
    }

    #[test]
    fn off_the_ui_thread_an_unprobed_set_asks_the_clis() {
        let only_kiro = |kind: Kind| kind == Kind::Kiro;
        let codex = Preferences {
            last_provider: Some("codex".into()),
            ..Default::default()
        };
        let choose = |known: &InstalledProviders, prefs: &Preferences| {
            new_session_choice_with(prefs, None, resolving_with(known, only_kiro)).0
        };
        // Nothing probed yet (right after launch): resolve directly.
        let unprobed = InstalledProviders::default();
        assert_eq!(choose(&unprobed, &codex), Kind::Kiro);
        assert_eq!(choose(&unprobed, &Preferences::default()), Kind::Kiro);
        // Probed: the recorded set answers, not the CLIs.
        let known = InstalledProviders::probed([Kind::Hermes]);
        assert_eq!(choose(&known, &codex), Kind::Hermes);
    }
    #[test]
    fn none_detected_only_after_a_probe_that_found_nothing() {
        assert!(!InstalledProviders::default().none_detected());
        assert!(InstalledProviders::probed([]).none_detected());
        assert!(!InstalledProviders::probed([Kind::Kiro]).none_detected());
    }
    #[test]
    fn an_unprobed_set_counts_every_provider_as_installed() {
        let known = InstalledProviders::default();
        assert!(!known.is_probed());
        assert!(Kind::ALL.into_iter().all(|kind| known.installed(kind)));
        assert_eq!(pick(&known, Some(Kind::Hermes)), Kind::Hermes);
        assert_eq!(pick(&known, None), Kind::Codex);
    }
}
