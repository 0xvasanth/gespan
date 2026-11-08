//! One `Provider` per agent CLI, reached through a single registry. Each
//! provider's facts come from its bundled manifest; its behaviour is trait
//! methods whose defaults forward to the existing `Kind` helpers, so nothing
//! changes for callers yet. Later steps move per-provider code behind these
//! methods instead of `kind == Kind::X` checks.
//!
//! The shape follows Wake's agent adapters (crates/wake-core/src/adapters):
//! one struct per agent, a trait with a small required core and defaulted
//! methods, and one place that builds them all.
use super::manifest::{self, Accounts, Login, Source};
use super::spec::Family;
use super::{Kind, Request};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// How Gespan talks to a provider's process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// Frames over the child's stdin and stdout.
    Stdio,
    /// The CLI serves HTTP and an event stream (OpenCode).
    Http,
}

/// What a launch that fails before the CLI answers may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryPolicy {
    Never,
    /// Launch once more with the fallback arguments (Kiro without its V3
    /// engine flag).
    FallbackOnce,
}

/// One launch of a provider process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attempt {
    /// A model or command probe, not a conversation turn.
    pub probe: bool,
    /// The retry that `RetryPolicy::FallbackOnce` allows.
    pub fallback: bool,
}

/// What a provider can do, read as plain data. Grows as kind checks move here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub side_threads: bool,
    pub rewind: bool,
    pub raw_slash_commands: bool,
    /// The composer can send images and files with a prompt.
    pub attachments: bool,
    /// The composer offers Gespan's permission modes.
    pub permission_modes: bool,
    /// Vetted for one-shot text work, so it may stand in for the preferred
    /// provider.
    pub text_turns: bool,
    /// Follow-ups reach a running turn; the value is how long Gespan waits
    /// for the CLI to acknowledge one, `None` waiting as long as the turn.
    pub steer: Option<Option<u64>>,
}

/// A provider's facts, built once from its manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub id: String,
    pub label: String,
    pub binary: String,
    pub install_dirs: Vec<String>,
    pub install_name: String,
    /// The install hint for this platform.
    pub install_command: Option<String>,
    pub login_hint: String,
    pub family: Family,
    pub login: Option<Login>,
    pub accounts: Option<Accounts>,
    /// The `usage` module that reads this provider's plan usage.
    pub usage: Option<String>,
    pub capabilities: Capabilities,
    pub discovery_timeout: Option<u64>,
    /// What the composer calls the effort control.
    pub effort_label: String,
    /// Model settings a saved choice keeps before the model catalog loads.
    pub settings_before_catalog: Vec<String>,
    pub version: u32,
    /// Where the manifest came from, for diagnostics.
    pub source: Source,
}
impl Descriptor {
    fn from_loaded(loaded: manifest::Loaded) -> Self {
        let m = loaded.manifest;
        Self {
            install_command: m.install_command.as_ref().map(|t| t.here().to_owned()),
            family: m.family.family(),
            capabilities: Capabilities {
                side_threads: m.features.side_threads,
                rewind: m.features.rewind,
                raw_slash_commands: m.features.raw_slash_commands,
                attachments: m.features.attachments,
                permission_modes: m.features.permission_modes,
                text_turns: m.features.text_turns,
                steer: m.features.steer.map(|s| s.ack_timeout),
            },
            id: m.id,
            label: m.label,
            binary: m.binary,
            install_dirs: m.install_dirs,
            install_name: m.install_name,
            login_hint: m.login_hint,
            login: m.login,
            accounts: m.accounts,
            usage: m.usage,
            discovery_timeout: m.discovery_timeout,
            effort_label: m.effort_label.unwrap_or_else(|| "Effort".into()),
            settings_before_catalog: m.settings_before_catalog,
            version: m.version,
            source: loaded.source,
        }
    }
}

/// One agent CLI. Only `kind` and `descriptor` are required; the rest
/// default to today's shared behaviour and are overridden per provider.
pub trait Provider: Send + Sync + 'static {
    /// The stable id used in persistence and settings.
    fn kind(&self) -> Kind;
    fn descriptor(&self) -> &Descriptor;

    fn id(&self) -> &str {
        &self.descriptor().id
    }
    fn label(&self) -> &str {
        &self.descriptor().label
    }
    fn capabilities(&self) -> Capabilities {
        self.descriptor().capabilities
    }
    /// The CLI on this machine, honouring a configured or env override.
    fn resolve_binary(&self) -> Option<PathBuf> {
        super::resolve(self.kind())
    }
    /// What the user sees when the CLI cannot be found at launch.
    fn missing_binary_message(&self) -> String {
        self.kind().missing_binary_message()
    }
    /// The picker's hint when the CLI is not installed.
    fn unavailable_hint(&self) -> String {
        self.kind().unavailable_hint()
    }

    /// The account can bank limit resets and spend them early; this comes
    /// from the provider's usage reader, which also spends them.
    fn banked_resets(&self) -> bool {
        self.kind().spec().usage.is_some_and(|u| u.banked_resets())
    }
    /// Spends one banked reset. Blocking.
    fn consume_reset(
        &self,
        profile: &crate::accounts::Profile,
        credit: Option<&str>,
    ) -> Result<String, String> {
        match self.kind().spec().usage {
            Some(usage) => usage.consume_reset(profile, credit),
            None => Err("This provider has no banked resets.".into()),
        }
    }

    /// This provider's CLI id for a model id the phone or the old desktop
    /// app sent. Most providers use the same id.
    fn native_model_id(&self, wire: &str) -> String {
        wire.into()
    }
    /// The reverse: the id the phone expects for a CLI model id.
    fn wire_model_id<'a>(&self, native: &'a str) -> &'a str {
        native
    }

    fn transport(&self) -> Transport {
        Transport::Stdio
    }
    fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy::Never
    }
    /// The folder the CLI process starts in.
    fn working_dir<'a>(&self, request: &'a Request) -> &'a Path {
        &request.cwd
    }
    /// The id a new conversation starts with, when Gespan picks it rather
    /// than the CLI.
    fn new_session_id(&self, _request: &Request) -> Option<String> {
        None
    }
    /// The CLI's launch arguments and environment. `new_session` is what
    /// `new_session_id` returned for this turn.
    fn configure(
        &self,
        command: &mut Command,
        request: &Request,
        attempt: Attempt,
        new_session: Option<&str>,
    ) -> Result<(), String> {
        super::configure_by_family(
            command,
            request,
            attempt.probe,
            attempt.fallback,
            new_session,
        )
    }
}

macro_rules! builtin {
    ($($name:ident => $kind:ident $({ $($method:item)* })?),* $(,)?) => {
        $(
            pub struct $name {
                descriptor: Descriptor,
            }
            impl Provider for $name {
                fn kind(&self) -> Kind {
                    Kind::$kind
                }
                fn descriptor(&self) -> &Descriptor {
                    &self.descriptor
                }
                $($($method)*)?
            }
        )*
        fn build(kind: Kind, descriptor: Descriptor) -> Box<dyn Provider> {
            match kind {
                $(Kind::$kind => Box::new($name { descriptor }),)*
            }
        }
    };
}
builtin! {
    ClaudeProvider => Claude {
        /// A new conversation starts with an id Gespan picks; resumes and
        /// one-shot text turns get none.
        fn new_session_id(&self, request: &Request) -> Option<String> {
            (request.provider_id.is_none() && !request.settings.text_only)
                .then(super::lifecycle::claude_session_id)
        }
        /// The phone and the old desktop app wrote versioned models without
        /// the `claude-` prefix, and some with dots (`opus-4.6`).
        fn native_model_id(&self, wire: &str) -> String {
            if !wire.starts_with("claude-") && wire.chars().any(|c| c.is_ascii_digit()) {
                format!("claude-{}", wire.replace('.', "-"))
            } else {
                wire.into()
            }
        }
        fn wire_model_id<'a>(&self, native: &'a str) -> &'a str {
            match native {
                "claude-opus-4-6" => "opus-4.6",
                "claude-sonnet-4-6" => "sonnet-4.6",
                "claude-opus-4-5" => "opus-4.5",
                "claude-haiku-4-5" => "haiku-4.5",
                _ => native.strip_prefix("claude-").unwrap_or(native),
            }
        }
    },
    CodexProvider => Codex,
    CursorProvider => Cursor,
    GrokProvider => Grok,
    OpenCodeProvider => OpenCode {
        fn transport(&self) -> Transport {
            Transport::Http
        }
    },
    AntigravityProvider => Antigravity {
        /// The ACP server starts in its own install folder, not the project.
        fn working_dir<'a>(&self, request: &'a Request) -> &'a Path {
            request.binary.parent().unwrap_or(&request.cwd)
        }
    },
    PiProvider => Pi,
    OmpProvider => Omp,
    FxProvider => Fx,
    HermesProvider => Hermes,
    DevinProvider => Devin,
    KiroProvider => Kiro {
        fn retry_policy(&self) -> RetryPolicy {
            RetryPolicy::FallbackOnce
        }
    },
}

impl Kind {
    /// This kind's provider in the process-wide registry.
    pub fn provider(self) -> &'static dyn Provider {
        ProviderRegistry::builtin().get(self)
    }
}

/// An id that names no provider. Callers decide what to do instead of
/// silently running Codex, as `Kind::from_id` does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownProvider(pub String);
impl fmt::Display for UnknownProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unknown provider \"{}\".", self.0)
    }
}
impl std::error::Error for UnknownProvider {}

/// Every provider, built in one place. Lookups index an array by `Kind`, so
/// they cost no allocation, lock or hashing.
pub struct ProviderRegistry {
    /// In `Kind` declaration order, so `kind as usize` is the slot.
    slots: Vec<Box<dyn Provider>>,
    /// Overrides that were skipped, with the reason.
    warnings: Vec<String>,
}
impl ProviderRegistry {
    /// The bundled providers, built once for the life of the process.
    pub fn builtin() -> &'static Self {
        static REGISTRY: OnceLock<ProviderRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| Self::load(None))
    }

    /// Bundled manifests, each shadowed by `<dir>/<id>.toml` when valid.
    pub fn load(override_dir: Option<&Path>) -> Self {
        let mut kinds = Kind::ALL;
        kinds.sort_by_key(|kind| *kind as usize);
        let mut warnings = Vec::new();
        let slots = kinds
            .into_iter()
            .map(|kind| {
                let mut loaded = manifest::load(kind, override_dir);
                warnings.extend(loaded.warning.take());
                build(kind, Descriptor::from_loaded(loaded))
            })
            .collect();
        Self { slots, warnings }
    }

    pub fn get(&self, kind: Kind) -> &dyn Provider {
        self.slots[kind as usize].as_ref()
    }

    pub fn parse(&self, id: &str) -> Result<&dyn Provider, UnknownProvider> {
        Kind::parse(id)
            .map(|kind| self.get(kind))
            .ok_or_else(|| UnknownProvider(id.to_owned()))
    }

    /// In `Kind::ALL` order, the order pickers show providers in.
    pub fn iter(&self) -> impl Iterator<Item = &dyn Provider> {
        Kind::ALL.into_iter().map(|kind| self.get(kind))
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn every_kind_has_exactly_one_provider_in_its_own_slot() {
        let registry = ProviderRegistry::builtin();
        assert_eq!(registry.slots.len(), Kind::ALL.len());
        for kind in Kind::ALL {
            assert_eq!(registry.get(kind).kind(), kind, "{kind:?}");
        }
        assert!(registry.warnings().is_empty());
    }

    #[test]
    fn the_builtin_registry_is_built_once() {
        assert!(std::ptr::eq(
            ProviderRegistry::builtin(),
            ProviderRegistry::builtin()
        ));
    }

    #[test]
    fn iteration_keeps_the_picker_order_of_kind_all() {
        let kinds: Vec<Kind> = ProviderRegistry::builtin()
            .iter()
            .map(|p| p.kind())
            .collect();
        assert_eq!(kinds, Kind::ALL);
    }

    #[test]
    fn ids_round_trip_and_unknown_ids_are_an_error_not_codex() {
        let registry = ProviderRegistry::builtin();
        for kind in Kind::ALL {
            assert_eq!(registry.parse(kind.id()).unwrap().kind(), kind);
        }
        let error = registry.parse("gemini").err().unwrap();
        assert_eq!(error, UnknownProvider("gemini".into()));
        assert_eq!(error.to_string(), "Unknown provider \"gemini\".");
        // The old lookup quietly picks Codex; the registry does not.
        assert_eq!(Kind::from_id("gemini"), Kind::Codex);
        assert!(registry.parse("").is_err());
        assert!(registry.parse("Claude").is_err());
    }

    /// The trait answers exactly what the `Kind` and `Spec` helpers answer
    /// today, for every provider.
    #[test]
    fn every_provider_answers_what_the_old_helpers_answer() {
        for provider in ProviderRegistry::builtin().iter() {
            let kind = provider.kind();
            let spec = kind.spec();
            let d = provider.descriptor();
            assert_eq!(provider.id(), kind.id(), "{kind:?}");
            assert_eq!(provider.label(), kind.label(), "{kind:?}");
            assert_eq!(d.binary, kind.binary_name(), "{kind:?}");
            assert_eq!(d.install_dirs, strings(kind.install_dirs()), "{kind:?}");
            assert_eq!(d.install_name, spec.install_name, "{kind:?}");
            assert_eq!(
                d.install_command.as_deref(),
                spec.install_command,
                "{kind:?}"
            );
            assert_eq!(d.login_hint, kind.login_hint(), "{kind:?}");
            assert_eq!(d.family, kind.family(), "{kind:?}");
            assert_eq!(
                d.login.clone().map(|l| (l.sign_in, l.sign_out)),
                spec.login
                    .map(|l| (strings(l.sign_in), l.sign_out.map(strings))),
                "{kind:?}"
            );
            assert_eq!(
                d.accounts.clone().map(|a| a.profile.map(|p| p.vars)),
                spec.accounts.map(|a| a.profile.map(|p| strings(p.vars))),
                "{kind:?}"
            );
            assert_eq!(d.usage.is_some(), spec.usage.is_some(), "{kind:?}");
            assert_eq!(d.discovery_timeout, spec.discovery_timeout, "{kind:?}");
            assert_eq!(d.source, Source::Bundled, "{kind:?}");
            let f = spec.features;
            assert_eq!(
                provider.capabilities(),
                Capabilities {
                    side_threads: f.side_threads,
                    rewind: f.rewind,
                    raw_slash_commands: f.raw_slash_commands,
                    // No `Spec` field yet: these were lists in the UI.
                    attachments: d.capabilities.attachments,
                    permission_modes: d.capabilities.permission_modes,
                    text_turns: d.capabilities.text_turns,
                    steer: f.steer,
                },
                "{kind:?}"
            );
            assert_eq!(
                provider.missing_binary_message(),
                kind.missing_binary_message(),
                "{kind:?}"
            );
            assert_eq!(
                provider.unavailable_hint(),
                kind.unavailable_hint(),
                "{kind:?}"
            );
            assert_eq!(
                provider.resolve_binary(),
                super::super::resolve(kind),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn kiro_keeps_its_own_install_guidance_through_the_trait() {
        let kiro = ProviderRegistry::builtin().get(Kind::Kiro);
        assert!(kiro
            .missing_binary_message()
            .contains("cli.kiro.dev/install"));
        let grok = ProviderRegistry::builtin().get(Kind::Grok);
        assert_eq!(
            grok.missing_binary_message(),
            "Grok Build CLI not found. Install it and sign in, or set GESPAN_GROK_BINARY."
        );
    }

    #[test]
    fn an_override_reaches_only_its_provider_and_a_bad_one_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let grok = include_str!("manifests/grok.toml")
            .replace("binary = \"grok\"", "binary = \"grok-nightly\"");
        std::fs::write(dir.path().join("grok.toml"), grok).unwrap();
        std::fs::write(dir.path().join("pi.toml"), "id = \"pi\"\nlabel =").unwrap();

        let registry = ProviderRegistry::load(Some(dir.path()));
        let grok = registry.get(Kind::Grok).descriptor();
        assert_eq!(grok.binary, "grok-nightly");
        assert_eq!(grok.source, Source::Override(dir.path().join("grok.toml")));
        // The broken Pi override leaves Pi on its bundled manifest.
        assert_eq!(registry.get(Kind::Pi).descriptor().source, Source::Bundled);
        assert_eq!(registry.get(Kind::Pi).descriptor().binary, "pi");
        assert_eq!(registry.warnings().len(), 1);
        assert!(registry.warnings()[0].contains("pi.toml"));
        // The process-wide registry never reads an override folder.
        assert_eq!(
            ProviderRegistry::builtin()
                .get(Kind::Grok)
                .descriptor()
                .binary,
            "grok"
        );
    }

    fn request(kind: Kind) -> Request {
        Request {
            kind,
            binary: PathBuf::from("/opt/agy-acp").join(kind.binary_name()),
            cwd: PathBuf::from("/work/project"),
            prompt: "hello".into(),
            provider_id: None,
            model: None,
            settings: Default::default(),
        }
    }

    fn kinds_where(pick: impl Fn(&dyn Provider) -> bool) -> Vec<Kind> {
        ProviderRegistry::builtin()
            .iter()
            .filter(|p| pick(*p))
            .map(|p| p.kind())
            .collect()
    }

    #[test]
    fn only_opencode_is_reached_over_http() {
        assert_eq!(
            kinds_where(|p| p.transport() == Transport::Http),
            [Kind::OpenCode]
        );
    }

    #[test]
    fn only_kiro_retries_a_launch_with_its_fallback_arguments() {
        assert_eq!(
            kinds_where(|p| p.retry_policy() == RetryPolicy::FallbackOnce),
            [Kind::Kiro]
        );
    }

    #[test]
    fn only_antigravity_starts_outside_the_project_folder() {
        let registry = ProviderRegistry::builtin();
        for provider in registry.iter() {
            let request = request(provider.kind());
            let expected = if provider.kind() == Kind::Antigravity {
                Path::new("/opt/agy-acp")
            } else {
                Path::new("/work/project")
            };
            assert_eq!(
                provider.working_dir(&request),
                expected,
                "{:?}",
                provider.kind()
            );
        }
        // A binary path with no parent keeps the project folder.
        let mut bare = request(Kind::Antigravity);
        bare.binary = PathBuf::from("/");
        assert_eq!(
            registry.get(Kind::Antigravity).working_dir(&bare),
            Path::new("/work/project")
        );
    }

    #[test]
    fn only_a_new_claude_conversation_gets_an_id_from_gespan() {
        let registry = ProviderRegistry::builtin();
        assert_eq!(
            kinds_where(|p| p.new_session_id(&request(p.kind())).is_some()),
            [Kind::Claude]
        );
        let claude = registry.get(Kind::Claude);
        let id = claude.new_session_id(&request(Kind::Claude)).unwrap();
        assert_eq!(id.len(), 36, "{id}");
        assert_eq!(id.chars().nth(14), Some('4'), "a v4 UUID: {id}");
        assert_ne!(claude.new_session_id(&request(Kind::Claude)), Some(id));

        let mut resume = request(Kind::Claude);
        resume.provider_id = Some("previous-session".into());
        assert_eq!(claude.new_session_id(&resume), None);
        let mut text = request(Kind::Claude);
        text.settings.text_only = true;
        assert_eq!(claude.new_session_id(&text), None);
    }

    // The sets below are what the UI hard-coded before it read capabilities;
    // each test pins one, so a manifest edit that changes it is deliberate.

    #[test]
    fn only_fx_cannot_take_attachments() {
        assert_eq!(kinds_where(|p| !p.capabilities().attachments), [Kind::Fx]);
    }

    #[test]
    fn the_vetted_text_providers_keep_their_fallback_order() {
        assert_eq!(
            kinds_where(|p| p.capabilities().text_turns),
            [
                Kind::Claude,
                Kind::Codex,
                Kind::Cursor,
                Kind::Grok,
                Kind::OpenCode,
                Kind::Kiro
            ]
        );
    }

    #[test]
    fn side_questions_are_off_for_fx_hermes_and_antigravity() {
        assert_eq!(
            kinds_where(|p| !p.capabilities().side_threads),
            [Kind::Fx, Kind::Hermes, Kind::Antigravity]
        );
    }

    #[test]
    fn only_claude_and_codex_keep_accounts_in_their_own_config_homes() {
        assert_eq!(
            kinds_where(|p| p
                .descriptor()
                .accounts
                .as_ref()
                .is_some_and(|a| a.profile.is_some())),
            [Kind::Claude, Kind::Codex]
        );
    }

    #[test]
    fn pi_and_omp_are_the_pi_family() {
        assert_eq!(
            kinds_where(|p| p.descriptor().family == Family::Pi),
            [Kind::Pi, Kind::Omp]
        );
    }

    #[test]
    fn only_fx_has_no_permission_modes_to_offer() {
        assert_eq!(
            kinds_where(|p| !p.capabilities().permission_modes),
            [Kind::Fx]
        );
    }

    #[test]
    fn codex_calls_its_effort_control_reasoning_and_the_rest_effort() {
        for provider in ProviderRegistry::builtin().iter() {
            let want = if provider.kind() == Kind::Codex {
                "Reasoning"
            } else {
                "Effort"
            };
            assert_eq!(
                provider.descriptor().effort_label,
                want,
                "{:?}",
                provider.kind()
            );
        }
    }

    #[test]
    fn only_claude_and_codex_keep_saved_settings_before_their_catalog_loads() {
        for provider in ProviderRegistry::builtin().iter() {
            let want: &[&str] = match provider.kind() {
                Kind::Claude => &["fast", "thinking", "context"],
                Kind::Codex => &["serviceTier"],
                _ => &[],
            };
            assert_eq!(
                provider.descriptor().settings_before_catalog,
                want,
                "{:?}",
                provider.kind()
            );
        }
    }

    #[test]
    fn claude_model_ids_cross_the_wire_without_their_prefix() {
        let claude = Kind::Claude.provider();
        for (wire, native) in [
            ("opus-4.6", "claude-opus-4-6"),
            ("sonnet-4.6", "claude-sonnet-4-6"),
            ("opus-4.5", "claude-opus-4-5"),
            ("haiku-4.5", "claude-haiku-4-5"),
            ("opus-5-5", "claude-opus-5-5"),
        ] {
            assert_eq!(claude.native_model_id(wire), native);
            assert_eq!(claude.wire_model_id(native), wire);
        }
        // Aliases have no version and stay as the CLI takes them.
        assert_eq!(claude.native_model_id("opus"), "opus");
        assert_eq!(claude.wire_model_id("opus"), "opus");
        assert_eq!(claude.native_model_id("claude-opus-5-5"), "claude-opus-5-5");
    }

    #[test]
    fn other_providers_send_model_ids_unchanged() {
        for provider in ProviderRegistry::builtin().iter() {
            if provider.kind() == Kind::Claude {
                continue;
            }
            for id in [
                "opus-4.6",
                "gpt-6.1-sol",
                "claude-opus-4-6",
                "provider/model",
            ] {
                assert_eq!(provider.native_model_id(id), id, "{:?}", provider.kind());
                assert_eq!(provider.wire_model_id(id), id, "{:?}", provider.kind());
            }
        }
    }

    #[test]
    fn only_codex_banks_limit_resets() {
        assert_eq!(kinds_where(|p| p.banked_resets()), [Kind::Codex]);
    }

    #[test]
    fn spending_a_reset_on_a_provider_without_them_fails_without_a_request() {
        let profile = crate::accounts::Profile::default();
        for provider in ProviderRegistry::builtin().iter() {
            if provider.banked_resets() {
                continue;
            }
            assert_eq!(
                provider.consume_reset(&profile, None),
                Err("This provider has no banked resets.".into()),
                "{:?}",
                provider.kind()
            );
        }
    }
}
