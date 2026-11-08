//! Provider identity is shared by discovery, persistence, pickers and transports.
use super::spec::Family;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Codex,
    Claude,
    Cursor,
    Grok,
    OpenCode,
    Antigravity,
    Pi,
    Omp,
    Fx,
    Hermes,
    Devin,
    Kiro,
}
impl Kind {
    pub const ALL: [Self; 12] = [
        Self::Claude,
        Self::Codex,
        Self::Cursor,
        Self::Grok,
        Self::OpenCode,
        Self::Pi,
        Self::Omp,
        Self::Fx,
        Self::Hermes,
        Self::Kiro,
        Self::Antigravity,
        Self::Devin,
    ];
    /// The provider assumed when none is named, or a stored id names one this
    /// build does not know.
    pub const FALLBACK: Kind = Kind::Codex;
    pub fn id(self) -> &'static str {
        self.spec().id
    }
    pub fn label(self) -> &'static str {
        self.spec().label
    }
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }
    pub fn from_id(id: &str) -> Self {
        Self::parse(id).unwrap_or(Self::FALLBACK)
    }
    pub fn binary_name(self) -> &'static str {
        self.spec().binary
    }
    pub(super) fn install_dirs(self) -> &'static [&'static str] {
        self.spec().install_dirs
    }
    /// Every provider but Codex, Claude and the Pi family. OpenCode counts:
    /// its catalog and model handling follow the ACP providers'.
    pub fn is_acp(self) -> bool {
        matches!(self.family(), Family::Acp | Family::OpenCode)
    }
    pub fn is_pi(self) -> bool {
        self.family() == Family::Pi
    }
    /// What the user sees when this provider's CLI cannot be found.
    pub fn missing_binary_message(self) -> String {
        match self {
            Self::Kiro => "Kiro CLI not found. Install it with `curl -fsSL https://cli.kiro.dev/install | bash` and run `kiro-cli login`, then retry.".into(),
            _ => format!(
                "{} CLI not found. Install it and sign in, or set GESPAN_{}_BINARY.",
                self.label(),
                self.id().to_uppercase()
            ),
        }
    }
    /// Kiro gets its install and login guidance; other providers keep the
    /// caller's own wording.
    pub fn missing_binary_or(self, generic: String) -> String {
        if self == Self::Kiro {
            self.missing_binary_message()
        } else {
            generic
        }
    }
    pub fn login_hint(self) -> &'static str {
        self.spec().login_hint
    }
    fn install_name(self) -> &'static str {
        self.spec().install_name
    }
    fn install_command(self) -> Option<&'static str> {
        self.spec().install_command
    }
    /// The same "not found" copy React uses. React only checks that a binary
    /// exists, so the hint never mentions signing in.
    pub fn unavailable_hint(self) -> String {
        let name = self.install_name();
        match self.install_command() {
            Some(install) => format!(
                "{name} not found (`{install}`). Install it, or restart Gespan if it is already installed."
            ),
            None => format!(
                "{name} not found. Install it, or restart Gespan if it is already installed."
            ),
        }
    }
}

/// The provider to start with: the preferred one if installed, else the first
/// installed in `Kind::ALL` order, else the preferred one (or `Kind::FALLBACK`), so the
/// existing "not found" guidance still shows. Mirrors the desktop app's
/// `firstEnabledHarness`.
pub fn pick_installed(preferred: Option<Kind>, installed: impl Fn(Kind) -> bool) -> Kind {
    let preferred = preferred.unwrap_or(Kind::FALLBACK);
    if installed(preferred) {
        return preferred;
    }
    Kind::ALL
        .into_iter()
        .find(|kind| installed(*kind))
        .unwrap_or(preferred)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pick_installed_keeps_an_installed_preference() {
        let installed = |k: Kind| matches!(k, Kind::Claude | Kind::Kiro);
        assert_eq!(pick_installed(Some(Kind::Kiro), installed), Kind::Kiro);
        // Claude comes first in `Kind::ALL`, but an installed preference wins.
        assert_eq!(pick_installed(Some(Kind::Kiro), |_| true), Kind::Kiro);
    }
    #[test]
    fn pick_installed_skips_a_missing_preference_for_the_first_installed() {
        let only_kiro = |k: Kind| k == Kind::Kiro;
        assert_eq!(pick_installed(Some(Kind::Codex), only_kiro), Kind::Kiro);
        assert_eq!(pick_installed(None, only_kiro), Kind::Kiro);
        // Several installed: the first in `Kind::ALL` order.
        let two = |k: Kind| matches!(k, Kind::Devin | Kind::Cursor);
        assert_eq!(pick_installed(Some(Kind::Codex), two), Kind::Cursor);
    }
    #[test]
    fn pick_installed_falls_back_to_the_preference_when_nothing_is_installed() {
        assert_eq!(pick_installed(Some(Kind::Kiro), |_| false), Kind::Kiro);
        assert_eq!(pick_installed(None, |_| false), Kind::Codex);
        // With no preference and everything installed (not probed yet), the
        // historic default holds rather than jumping to `Kind::ALL[0]`.
        assert_eq!(pick_installed(None, |_| true), Kind::Codex);
    }
    #[test]
    fn an_id_this_build_does_not_know_opens_as_the_fallback() {
        // Rows saved by a newer build, or edited by hand, still open.
        for id in ["", "future-agent", "Claude", " codex"] {
            assert_eq!(Kind::from_id(id), Kind::FALLBACK, "{id:?}");
        }
        for kind in Kind::ALL {
            assert_eq!(Kind::from_id(kind.id()), kind);
        }
    }
    #[test]
    fn registry_matches_the_react_harnesses() {
        let source = include_str!("../../tests/fixtures/desktop/harness-register.ts");
        for kind in Kind::ALL {
            assert_eq!(Kind::parse(kind.id()), Some(kind));
            assert!(source.contains(&format!("../providers/{}/", kind.id())));
        }
        assert_eq!(Kind::parse("unknown"), None);
        assert_eq!(Kind::Cursor.binary_name(), "cursor-agent");
    }
    #[test]
    fn kiro_is_a_known_acp_provider_named_kiro_cli() {
        let kiro = Kind::parse("kiro").expect("kiro parses");
        assert_eq!(kiro, Kind::Kiro);
        assert_eq!(kiro.id(), "kiro");
        assert_eq!(kiro.label(), "Kiro CLI");
        assert_eq!(kiro.binary_name(), "kiro-cli");
        assert_eq!(kiro.login_hint(), "kiro-cli login");
        assert!(kiro.is_acp());
        assert!(!kiro.is_pi());
        assert_eq!(Kind::ALL.len(), 12);
        let hermes = Kind::ALL.iter().position(|k| *k == Kind::Hermes).unwrap();
        assert_eq!(Kind::ALL[hermes + 1], Kind::Kiro);
    }
    #[test]
    fn only_kiro_overrides_a_callers_missing_binary_text() {
        assert_eq!(
            Kind::Kiro.missing_binary_or("Kiro CLI not found.".into()),
            Kind::Kiro.missing_binary_message()
        );
        assert_eq!(
            Kind::Codex.missing_binary_or("Codex CLI not found.".into()),
            "Codex CLI not found."
        );
    }
    #[test]
    fn kiro_missing_binary_message_is_actionable() {
        assert_eq!(
            Kind::Kiro.missing_binary_message(),
            "Kiro CLI not found. Install it with `curl -fsSL https://cli.kiro.dev/install | bash` and run `kiro-cli login`, then retry."
        );
        assert_eq!(
            Kind::Hermes.missing_binary_message(),
            "Hermes Agent CLI not found. Install it and sign in, or set GESPAN_HERMES_BINARY."
        );
    }
    #[test]
    fn unavailable_hint_matches_the_react_copy() {
        assert_eq!(
            Kind::Antigravity.unavailable_hint(),
            "Antigravity ACP server (agy_acp_server.par) not found. Install it, or restart Gespan if it is already installed."
        );
        assert_eq!(
            Kind::Pi.unavailable_hint(),
            "Pi CLI not found (`npm i -g @earendil-works/pi-coding-agent`). Install it, or restart Gespan if it is already installed."
        );
        assert_eq!(
            Kind::Codex.unavailable_hint(),
            "Codex CLI not found. Install it, or restart Gespan if it is already installed."
        );
    }
    #[test]
    fn hints_match_the_react_source() {
        let source = include_str!("../../tests/fixtures/desktop/harness-availability.ts");
        for kind in Kind::ALL {
            assert!(
                source.contains(&format!("\"{}\"", kind.install_name())),
                "React is missing the install name for {}",
                kind.id()
            );
            if let Some(install) = kind.install_command() {
                assert!(
                    source.contains(install),
                    "React is missing the install command for {}",
                    kind.id()
                );
            }
        }
        assert!(source.contains(
            "not found${how}. Install it, or restart Gespan if it is already installed."
        ));
    }
}
