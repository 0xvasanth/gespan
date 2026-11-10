//! What each provider is and can do, in one table. Code asks a provider's
//! [`Spec`] rather than keeping its own list of kinds, so adding a provider
//! means adding a row here and, when it brings a new protocol, a family module.
use super::Kind;

/// The wire protocol a provider speaks. Each family has its own module that
/// owns its launch arguments, per-turn state and frame handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// `codex app-server` JSON-RPC (provider/codex.rs).
    Codex,
    /// Claude Code's stream-json (provider/claude.rs).
    Claude,
    /// Pi's RPC mode (provider/pi.rs).
    Pi,
    /// The Agent Client Protocol (provider/acp.rs).
    Acp,
    /// OpenCode's HTTP server and event stream (provider/opencode.rs).
    OpenCode,
}

/// The CLI's own sign-in commands, run in a terminal Gespan opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Login {
    pub sign_in: &'static [&'static str],
    pub sign_out: Option<&'static [&'static str]>,
}

/// How Gespan keeps several accounts of one provider apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accounts {
    /// A private config home per account, when the CLI reads one from the
    /// environment. Without it the provider only has its default account
    /// plus labels.
    pub profile: Option<ProfileHome>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileHome {
    /// Variables pointed at the account's directory.
    pub vars: &'static [&'static str],
    /// The default account's home, under the user's home directory.
    pub default_dir: &'static str,
    /// Credentials from the environment that would override the account.
    pub clear: &'static [&'static str],
}

/// Optional behaviour that depends on what the CLI supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    /// Side questions ("btw") can run on this provider.
    pub side_threads: bool,
    /// The CLI can drop its last turn, so Gespan can rewind it natively.
    pub rewind: bool,
    /// The CLI parses a leading slash command itself, so Gespan must pass
    /// the prompt through untouched.
    pub raw_slash_commands: bool,
    /// Follow-ups reach a running turn. The value is how long Gespan waits
    /// for the CLI to acknowledge one; `None` waits as long as the turn.
    pub steer: Option<Option<u64>>,
}
const STANDARD: Features = Features {
    side_threads: true,
    rewind: false,
    raw_slash_commands: false,
    steer: None,
};

pub struct Spec {
    pub id: &'static str,
    pub label: &'static str,
    pub binary: &'static str,
    /// Directories under the user's home where installers put the binary.
    pub install_dirs: &'static [&'static str],
    /// The name shown when the binary is missing, matching the React harness map.
    pub install_name: &'static str,
    /// The install command React shows in its hint, when it has one.
    pub install_command: Option<&'static str>,
    pub login_hint: &'static str,
    pub family: Family,
    pub login: Option<Login>,
    pub accounts: Option<Accounts>,
    /// Where plan usage and limits come from, when the provider has any.
    pub usage: Option<&'static dyn crate::usage::Source>,
    pub features: Features,
    /// Seconds a model or command probe may take; `None` waits for it.
    pub discovery_timeout: Option<u64>,
}

const CLAUDE: Spec = Spec {
    id: "claude",
    label: "Claude Code",
    binary: "claude",
    install_dirs: &[],
    install_name: "Claude Code CLI",
    install_command: None,
    login_hint: "claude auth login",
    family: Family::Claude,
    login: Some(Login {
        sign_in: &["auth", "login"],
        sign_out: Some(&["auth", "logout"]),
    }),
    accounts: Some(Accounts {
        profile: Some(ProfileHome {
            vars: &["CLAUDE_CONFIG_DIR", "CLAUDE_SECURESTORAGE_CONFIG_DIR"],
            default_dir: ".claude",
            clear: &[
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "CLAUDE_CODE_OAUTH_TOKEN",
            ],
        }),
    }),
    usage: Some(&crate::usage::Claude),
    features: Features {
        steer: Some(None),
        ..STANDARD
    },
    discovery_timeout: Some(60),
};
const CODEX: Spec = Spec {
    id: "codex",
    label: "Codex",
    binary: "codex",
    install_dirs: &[],
    install_name: "Codex CLI",
    install_command: None,
    login_hint: "codex login",
    family: Family::Codex,
    login: Some(Login {
        sign_in: &["login"],
        sign_out: Some(&["logout"]),
    }),
    accounts: Some(Accounts {
        profile: Some(ProfileHome {
            vars: &["CODEX_HOME"],
            default_dir: ".codex",
            clear: &["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"],
        }),
    }),
    usage: Some(&crate::usage::Codex),
    features: Features {
        rewind: true,
        steer: Some(None),
        ..STANDARD
    },
    discovery_timeout: Some(60),
};
const CURSOR: Spec = Spec {
    id: "cursor",
    label: "Cursor",
    binary: "cursor-agent",
    install_dirs: &[],
    install_name: "Cursor CLI",
    install_command: None,
    login_hint: "agent login",
    family: Family::Acp,
    login: Some(Login {
        sign_in: &["login"],
        sign_out: Some(&["logout"]),
    }),
    accounts: Some(Accounts { profile: None }),
    usage: Some(&crate::usage::Cursor),
    features: Features {
        steer: Some(None),
        ..STANDARD
    },
    discovery_timeout: Some(15),
};
const GROK: Spec = Spec {
    id: "grok",
    label: "Grok Build",
    binary: "grok",
    install_dirs: &[".grok/bin"],
    install_name: "Grok Build CLI",
    install_command: Some("curl -fsSL https://x.ai/cli/install.sh | bash"),
    login_hint: "grok login",
    family: Family::Acp,
    login: Some(Login {
        sign_in: &["login", "--oauth"],
        sign_out: None,
    }),
    accounts: None,
    usage: None,
    features: STANDARD,
    discovery_timeout: Some(15),
};
const OPENCODE: Spec = Spec {
    id: "opencode",
    label: "OpenCode",
    binary: "opencode",
    install_dirs: &[".opencode/bin"],
    install_name: "OpenCode CLI",
    install_command: None,
    login_hint: "opencode auth login",
    family: Family::OpenCode,
    login: None,
    accounts: None,
    usage: Some(&crate::usage::OpenCode),
    features: Features {
        rewind: true,
        steer: Some(Some(30)),
        ..STANDARD
    },
    discovery_timeout: Some(60),
};
const ANTIGRAVITY: Spec = Spec {
    id: "antigravity",
    label: "Antigravity",
    binary: "agy_acp_server.par",
    install_dirs: &[".local/share/agy-acp"],
    install_name: "Antigravity ACP server (agy_acp_server.par)",
    install_command: None,
    login_hint: "agy",
    family: Family::Acp,
    login: None,
    accounts: None,
    usage: None,
    features: Features {
        side_threads: false,
        ..STANDARD
    },
    discovery_timeout: None,
};
const PI: Spec = Spec {
    id: "pi",
    label: "Pi",
    binary: "pi",
    install_dirs: &[],
    install_name: "Pi CLI",
    install_command: Some("npm i -g @earendil-works/pi-coding-agent"),
    login_hint: "pi",
    family: Family::Pi,
    login: None,
    accounts: None,
    usage: Some(&crate::usage::Pi),
    features: Features {
        rewind: true,
        steer: Some(Some(15)),
        ..STANDARD
    },
    discovery_timeout: Some(45),
};
const OMP: Spec = Spec {
    id: "omp",
    label: "omp",
    binary: "omp",
    install_dirs: &[],
    install_name: "omp CLI",
    install_command: Some("curl -fsSL https://omp.sh/install | sh"),
    login_hint: "omp",
    family: Family::Pi,
    login: None,
    accounts: None,
    // omp keeps its own sign-ins, which Pi's usage reader does not know.
    usage: None,
    features: Features {
        rewind: true,
        raw_slash_commands: true,
        steer: Some(Some(15)),
        ..STANDARD
    },
    discovery_timeout: Some(45),
};
const FX: Spec = Spec {
    id: "fx",
    label: "fx",
    binary: "fx",
    install_dirs: &[".fx/bin"],
    install_name: "fx CLI",
    install_command: Some("curl -fsSL https://fx.sh/setup.sh | bash"),
    login_hint: "fx login",
    family: Family::Acp,
    login: Some(Login {
        sign_in: &["login", "vercel"],
        sign_out: None,
    }),
    accounts: None,
    usage: None,
    features: Features {
        side_threads: false,
        ..STANDARD
    },
    discovery_timeout: Some(60),
};
const HERMES: Spec = Spec {
    id: "hermes",
    label: "Hermes Agent",
    binary: "hermes",
    install_dirs: &[
        ".hermes/hermes-agent/venv/bin",
        ".hermes/hermes-agent/.venv/bin",
    ],
    install_name: "Hermes Agent CLI",
    install_command: Some("Install from hermes-agent.nousresearch.com, then run hermes model"),
    login_hint: "hermes model",
    family: Family::Acp,
    login: None,
    accounts: None,
    usage: None,
    features: Features {
        side_threads: false,
        steer: Some(Some(20)),
        ..STANDARD
    },
    discovery_timeout: Some(30),
};
const DEVIN: Spec = Spec {
    id: "devin",
    label: "Devin",
    binary: "devin",
    install_dirs: &[],
    install_name: "Devin CLI",
    install_command: Some(if cfg!(windows) {
        "irm https://static.devin.ai/cli/setup.ps1 | iex"
    } else {
        "curl -fsSL https://cli.devin.ai/install.sh | bash"
    }),
    login_hint: "devin auth login",
    family: Family::Acp,
    login: Some(Login {
        sign_in: &["auth", "login"],
        sign_out: Some(&["auth", "logout"]),
    }),
    accounts: Some(Accounts { profile: None }),
    usage: Some(&crate::usage::Devin),
    features: Features {
        raw_slash_commands: true,
        ..STANDARD
    },
    discovery_timeout: Some(60),
};
const KIRO: Spec = Spec {
    id: "kiro",
    label: "Kiro CLI",
    binary: "kiro-cli",
    install_dirs: &[],
    install_name: "Kiro CLI",
    install_command: Some("curl -fsSL https://cli.kiro.dev/install | bash, then kiro-cli login"),
    login_hint: "kiro-cli login",
    family: Family::Acp,
    login: Some(Login {
        sign_in: &["login"],
        sign_out: Some(&["logout"]),
    }),
    accounts: Some(Accounts { profile: None }),
    usage: Some(&crate::usage::Kiro),
    features: STANDARD,
    discovery_timeout: Some(30),
};

impl Kind {
    pub fn spec(self) -> &'static Spec {
        match self {
            Self::Claude => &CLAUDE,
            Self::Codex => &CODEX,
            Self::Cursor => &CURSOR,
            Self::Grok => &GROK,
            Self::OpenCode => &OPENCODE,
            Self::Antigravity => &ANTIGRAVITY,
            Self::Pi => &PI,
            Self::Omp => &OMP,
            Self::Fx => &FX,
            Self::Hermes => &HERMES,
            Self::Devin => &DEVIN,
            Self::Kiro => &KIRO,
        }
    }
    pub fn family(self) -> Family {
        self.spec().family
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_names_its_own_kind() {
        for kind in Kind::ALL {
            assert_eq!(Kind::parse(kind.spec().id), Some(kind));
        }
    }

    #[test]
    fn accounts_with_a_private_home_clear_every_overriding_credential() {
        for kind in Kind::ALL {
            let Some(profile) = kind.spec().accounts.and_then(|a| a.profile) else {
                continue;
            };
            assert!(!profile.vars.is_empty(), "{kind:?}");
            assert!(profile.default_dir.starts_with('.'), "{kind:?}");
            assert!(!profile.clear.is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn features_match_what_each_cli_supports() {
        let with = |pick: fn(&Features) -> bool| -> Vec<Kind> {
            Kind::ALL
                .into_iter()
                .filter(|kind| pick(&kind.spec().features))
                .collect()
        };
        assert_eq!(
            with(|f| f.rewind),
            [Kind::Codex, Kind::OpenCode, Kind::Pi, Kind::Omp]
        );
        assert_eq!(with(|f| f.raw_slash_commands), [Kind::Omp, Kind::Devin]);
        assert_eq!(
            with(|f| f.steer.is_some()),
            [
                Kind::Claude,
                Kind::Codex,
                Kind::Cursor,
                Kind::OpenCode,
                Kind::Pi,
                Kind::Omp,
                Kind::Hermes
            ]
        );
        assert_eq!(
            with(|f| !f.side_threads),
            [Kind::Fx, Kind::Hermes, Kind::Antigravity]
        );
    }

    #[test]
    fn a_kiro_footer_can_offer_its_sign_in() {
        // The footer shows a sign-in panel only for providers with a login.
        let login = Kind::Kiro.spec().login.unwrap();
        assert_eq!(login.sign_in, ["login"]);
        assert_eq!(Kind::Kiro.login_hint(), "kiro-cli login");
    }
}
