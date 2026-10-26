//! Which provider CLIs are installed, so a new session, Pilot chat or run
//! starts on one that can actually launch (the desktop's `firstEnabledHarness`).
use gespan_core::provider::{self, Kind};
use gpui::{App, Global};
use std::collections::HashSet;
use std::path::PathBuf;

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
    pub(crate) fn pick(&self, preferred: Option<Kind>) -> Kind {
        provider::pick_installed(preferred, |kind| self.installed(kind))
    }
}

/// The app's current knowledge; unprobed when nothing has been recorded yet.
pub(crate) fn snapshot(cx: &App) -> InstalledProviders {
    cx.try_global::<InstalledProviders>()
        .cloned()
        .unwrap_or_default()
}

/// `preferred` if installed, else the first installed provider, else `preferred`.
pub(crate) fn pick(preferred: Option<Kind>, cx: &App) -> Kind {
    cx.try_global::<InstalledProviders>().map_or_else(
        || InstalledProviders::default().pick(preferred),
        |known| known.pick(preferred),
    )
}

/// For work already off the UI thread: `known` once probed, else ask the CLIs
/// directly, so a launch right after startup does not trust the preference.
/// Tests inject `known` and never read this machine's PATH.
pub(crate) fn pick_resolving(known: &InstalledProviders, preferred: Option<Kind>) -> Kind {
    pick_resolving_with(known, preferred, |kind| {
        cfg!(test) || provider::resolve(kind).is_some()
    })
}
fn pick_resolving_with(
    known: &InstalledProviders,
    preferred: Option<Kind>,
    resolve: impl Fn(Kind) -> bool,
) -> Kind {
    if known.is_probed() {
        known.pick(preferred)
    } else {
        provider::pick_installed(preferred, resolve)
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
        assert_eq!(known.pick(Some(Kind::Codex)), Kind::Kiro);
    }

    #[test]
    fn off_the_ui_thread_an_unprobed_set_asks_the_clis() {
        let only_kiro = |kind: Kind| kind == Kind::Kiro;
        // Nothing probed yet (right after launch): resolve directly.
        assert_eq!(
            pick_resolving_with(&InstalledProviders::default(), Some(Kind::Codex), only_kiro),
            Kind::Kiro
        );
        assert_eq!(
            pick_resolving_with(&InstalledProviders::default(), None, only_kiro),
            Kind::Kiro
        );
        // Probed: the recorded set answers, not the CLIs.
        let known = InstalledProviders::probed([Kind::Hermes]);
        assert_eq!(
            pick_resolving_with(&known, Some(Kind::Codex), only_kiro),
            Kind::Hermes
        );
    }
    #[test]
    fn an_unprobed_set_counts_every_provider_as_installed() {
        let known = InstalledProviders::default();
        assert!(!known.is_probed());
        assert!(Kind::ALL.into_iter().all(|kind| known.installed(kind)));
        assert_eq!(known.pick(Some(Kind::Hermes)), Kind::Hermes);
        assert_eq!(known.pick(None), Kind::Codex);
    }
}
