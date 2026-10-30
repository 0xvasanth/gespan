//! Provider identity is shared by discovery, persistence, pickers and transports.
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
    pub fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
            Self::Antigravity => "antigravity",
            Self::Pi => "pi",
            Self::Omp => "omp",
            Self::Fx => "fx",
            Self::Hermes => "hermes",
            Self::Devin => "devin",
            Self::Kiro => "kiro",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::Cursor => "Cursor",
            Self::Grok => "Grok Build",
            Self::OpenCode => "OpenCode",
            Self::Antigravity => "Antigravity",
            Self::Pi => "Pi",
            Self::Omp => "omp",
            Self::Fx => "fx",
            Self::Hermes => "Hermes Agent",
            Self::Devin => "Devin",
            Self::Kiro => "Kiro CLI",
        }
    }
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }
    pub fn from_id(id: &str) -> Self {
        Self::parse(id).unwrap_or(Self::Codex)
    }
    pub fn binary_name(self) -> &'static str {
        match self {
            Self::Cursor => "cursor-agent",
            Self::Antigravity => "agy_acp_server.par",
            Self::Kiro => "kiro-cli",
            _ => self.id(),
        }
    }
    pub(super) fn install_dirs(self) -> &'static [&'static str] {
        match self {
            Self::Grok => &[".grok/bin"],
            Self::OpenCode => &[".opencode/bin"],
            Self::Fx => &[".fx/bin"],
            Self::Antigravity => &[".local/share/agy-acp"],
            Self::Hermes => &[
                ".hermes/hermes-agent/venv/bin",
                ".hermes/hermes-agent/.venv/bin",
            ],
            _ => &[],
        }
    }
    pub fn is_acp(self) -> bool {
        !matches!(self, Self::Codex | Self::Claude | Self::Pi | Self::Omp)
    }
    pub fn is_pi(self) -> bool {
        matches!(self, Self::Pi | Self::Omp)
    }
    pub fn subscription_usage(self) -> bool {
        matches!(self, Self::Codex | Self::Claude)
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
        match self {
            Self::Codex => "codex login",
            Self::Claude => "claude auth login",
            Self::Cursor => "agent login",
            Self::Grok => "grok login",
            Self::OpenCode => "opencode auth login",
            Self::Antigravity => "agy",
            Self::Pi => "pi",
            Self::Omp => "omp",
            Self::Fx => "fx login",
            Self::Hermes => "hermes model",
            Self::Devin => "devin auth login",
            Self::Kiro => "kiro-cli login",
        }
    }
    /// The name shown when the binary is missing, matching the React harness map.
    fn install_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex CLI",
            Self::Claude => "Claude Code CLI",
            Self::Cursor => "Cursor CLI",
            Self::Grok => "Grok Build CLI",
            Self::OpenCode => "OpenCode CLI",
            Self::Antigravity => "Antigravity ACP server (agy_acp_server.par)",
            Self::Pi => "Pi CLI",
            Self::Omp => "omp CLI",
            Self::Fx => "fx CLI",
            Self::Hermes => "Hermes Agent CLI",
            Self::Devin => "Devin CLI",
            Self::Kiro => "Kiro CLI",
        }
    }
    /// The install command React shows in its hint, when it has one.
    fn install_command(self) -> Option<&'static str> {
        match self {
            Self::Grok => Some("curl -fsSL https://x.ai/cli/install.sh | bash"),
            Self::Pi => Some("npm i -g @earendil-works/pi-coding-agent"),
            Self::Omp => Some("curl -fsSL https://omp.sh/install | sh"),
            Self::Fx => Some("curl -fsSL https://fx.sh/setup.sh | bash"),
            Self::Hermes => {
                Some("Install from hermes-agent.nousresearch.com, then run hermes model")
            }
            Self::Kiro => {
                Some("curl -fsSL https://cli.kiro.dev/install | bash, then kiro-cli login")
            }
            #[cfg(windows)]
            Self::Devin => Some("irm https://static.devin.ai/cli/setup.ps1 | iex"),
            #[cfg(not(windows))]
            Self::Devin => Some("curl -fsSL https://cli.devin.ai/install.sh | bash"),
            Self::Codex | Self::Claude | Self::Cursor | Self::OpenCode | Self::Antigravity => None,
        }
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
/// installed in `Kind::ALL` order, else the preferred one (or Codex), so the
/// existing "not found" guidance still shows. Mirrors the desktop app's
/// `firstEnabledHarness`.
pub fn pick_installed(preferred: Option<Kind>, installed: impl Fn(Kind) -> bool) -> Kind {
    let preferred = preferred.unwrap_or(Kind::Codex);
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
