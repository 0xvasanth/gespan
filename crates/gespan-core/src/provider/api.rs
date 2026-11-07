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
use super::Kind;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// What a provider can do, read as plain data. Grows as kind checks move here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub side_threads: bool,
    pub rewind: bool,
    pub raw_slash_commands: bool,
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
}

macro_rules! builtin {
    ($($name:ident => $kind:ident),* $(,)?) => {
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
    ClaudeProvider => Claude,
    CodexProvider => Codex,
    CursorProvider => Cursor,
    GrokProvider => Grok,
    OpenCodeProvider => OpenCode,
    AntigravityProvider => Antigravity,
    PiProvider => Pi,
    OmpProvider => Omp,
    FxProvider => Fx,
    HermesProvider => Hermes,
    DevinProvider => Devin,
    KiroProvider => Kiro,
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
}
