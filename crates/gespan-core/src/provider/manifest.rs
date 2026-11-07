//! What each provider is, as data: one bundled TOML manifest per provider.
//! They mirror the [`Spec`](super::Spec) table field for field and nothing
//! reads them yet; the provider registry will. A manifest in an override
//! folder shadows the bundled one, so a CLI change can be fixed without a
//! release.
//!
//! The bundle, validation and override fallback follow herdr's agent
//! manifests (src/detect/manifest.rs at ee5bfb50, Apache-2.0).
use super::spec::Family;
use super::Kind;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The manifest format this build reads. A manifest that needs a newer one
/// is ignored in favour of the bundled copy.
pub const ENGINE_VERSION: u32 = 1;

const BUNDLED: [(Kind, &str); 12] = [
    (Kind::Claude, include_str!("manifests/claude.toml")),
    (Kind::Codex, include_str!("manifests/codex.toml")),
    (Kind::Cursor, include_str!("manifests/cursor.toml")),
    (Kind::Grok, include_str!("manifests/grok.toml")),
    (Kind::OpenCode, include_str!("manifests/opencode.toml")),
    (
        Kind::Antigravity,
        include_str!("manifests/antigravity.toml"),
    ),
    (Kind::Pi, include_str!("manifests/pi.toml")),
    (Kind::Omp, include_str!("manifests/omp.toml")),
    (Kind::Fx, include_str!("manifests/fx.toml")),
    (Kind::Hermes, include_str!("manifests/hermes.toml")),
    (Kind::Devin, include_str!("manifests/devin.toml")),
    (Kind::Kiro, include_str!("manifests/kiro.toml")),
];

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    /// Bumped whenever the manifest's content changes.
    pub version: u32,
    pub min_engine_version: u32,
    pub label: String,
    pub binary: String,
    #[serde(default)]
    pub install_dirs: Vec<String>,
    pub install_name: String,
    #[serde(default)]
    pub install_command: Option<PlatformText>,
    pub login_hint: String,
    pub family: FamilyId,
    #[serde(default)]
    pub login: Option<Login>,
    #[serde(default)]
    pub accounts: Option<Accounts>,
    /// The `usage` module that reads this provider's plan usage.
    #[serde(default)]
    pub usage: Option<String>,
    pub features: Features,
    /// Seconds a model or command probe may take; absent waits for it.
    #[serde(default)]
    pub discovery_timeout: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FamilyId {
    Codex,
    Claude,
    Pi,
    Acp,
    OpenCode,
}
impl FamilyId {
    pub fn family(self) -> Family {
        match self {
            Self::Codex => Family::Codex,
            Self::Claude => Family::Claude,
            Self::Pi => Family::Pi,
            Self::Acp => Family::Acp,
            Self::OpenCode => Family::OpenCode,
        }
    }
}

/// Text that is the same everywhere, or differs on Windows.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PlatformText {
    Same(String),
    PerOs { default: String, windows: String },
}
impl PlatformText {
    pub fn for_os(&self, windows: bool) -> &str {
        match self {
            Self::Same(text) => text,
            Self::PerOs { windows: text, .. } if windows => text,
            Self::PerOs { default, .. } => default,
        }
    }
    pub fn here(&self) -> &str {
        self.for_os(cfg!(windows))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Login {
    pub sign_in: Vec<String>,
    #[serde(default)]
    pub sign_out: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Accounts {
    #[serde(default)]
    pub profile: Option<ProfileHome>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileHome {
    pub vars: Vec<String>,
    pub default_dir: String,
    pub clear: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Features {
    pub side_threads: bool,
    pub rewind: bool,
    pub raw_slash_commands: bool,
    /// Present when follow-ups reach a running turn.
    #[serde(default)]
    pub steer: Option<Steer>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Steer {
    /// Seconds to wait for the CLI to acknowledge a follow-up; absent waits
    /// as long as the turn.
    #[serde(default)]
    pub ack_timeout: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Bundled,
    Override(PathBuf),
}

#[derive(Clone, Debug)]
pub struct Loaded {
    pub manifest: Manifest,
    pub source: Source,
    /// Why an override was ignored.
    pub warning: Option<String>,
}

pub fn parse(content: &str) -> Result<Manifest, String> {
    let manifest = toml::from_str::<Manifest>(content).map_err(|e| e.to_string())?;
    validate(&manifest)?;
    Ok(manifest)
}

fn validate(manifest: &Manifest) -> Result<(), String> {
    if manifest.min_engine_version > ENGINE_VERSION {
        return Err(format!(
            "manifest requires engine {}, this build reads {ENGINE_VERSION}",
            manifest.min_engine_version
        ));
    }
    for (field, value) in [
        ("id", &manifest.id),
        ("label", &manifest.label),
        ("binary", &manifest.binary),
    ] {
        if value.trim().is_empty() {
            return Err(format!("manifest {field} must not be empty"));
        }
    }
    if manifest
        .login
        .as_ref()
        .is_some_and(|l| l.sign_in.is_empty())
    {
        return Err("manifest login.sign_in must not be empty".into());
    }
    Ok(())
}

/// The manifest shipped in this build. Bundled manifests are checked by the
/// tests, so a bad one never reaches a release.
pub fn bundled(kind: Kind) -> Manifest {
    let (_, content) = BUNDLED
        .iter()
        .find(|(bundled, _)| *bundled == kind)
        .unwrap_or_else(|| panic!("no bundled manifest for {kind:?}"));
    parse(content).unwrap_or_else(|e| panic!("bundled {} manifest is invalid: {e}", kind.id()))
}

/// `<dir>/<id>.toml` when it is valid and names this provider, else the
/// bundled manifest with the reason the override was skipped.
pub fn load(kind: Kind, override_dir: Option<&Path>) -> Loaded {
    let fallback = |warning: Option<String>| Loaded {
        manifest: bundled(kind),
        source: Source::Bundled,
        warning,
    };
    let Some(dir) = override_dir else {
        return fallback(None);
    };
    let path = dir.join(format!("{}.toml", kind.id()));
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return fallback(None),
        Err(e) => {
            return fallback(Some(format!(
                "ignored override {} because it could not be read: {e}",
                path.display()
            )));
        }
    };
    match parse(&content) {
        Ok(manifest) if manifest.id == kind.id() => Loaded {
            manifest,
            source: Source::Override(path),
            warning: None,
        },
        Ok(manifest) => fallback(Some(format!(
            "ignored override {} because manifest id {} does not match {}",
            path.display(),
            manifest.id,
            kind.id()
        ))),
        Err(e) => fallback(Some(format!(
            "ignored override {} because it could not be loaded: {e}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    /// Field for field against the `Spec` row, so the two cannot drift while
    /// both exist.
    fn assert_mirrors_spec(kind: Kind) {
        let manifest = bundled(kind);
        let spec = kind.spec();
        assert_eq!(manifest.id, spec.id, "{kind:?} id");
        assert_eq!(manifest.label, spec.label, "{kind:?} label");
        assert_eq!(manifest.binary, spec.binary, "{kind:?} binary");
        assert_eq!(
            manifest.install_dirs,
            strings(spec.install_dirs),
            "{kind:?} install_dirs"
        );
        assert_eq!(
            manifest.install_name, spec.install_name,
            "{kind:?} install_name"
        );
        assert_eq!(
            manifest.install_command.as_ref().map(PlatformText::here),
            spec.install_command,
            "{kind:?} install_command"
        );
        assert_eq!(manifest.login_hint, spec.login_hint, "{kind:?} login_hint");
        assert_eq!(manifest.family.family(), spec.family, "{kind:?} family");
        assert_eq!(
            manifest.login.map(|l| (l.sign_in, l.sign_out)),
            spec.login
                .map(|l| (strings(l.sign_in), l.sign_out.map(strings))),
            "{kind:?} login"
        );
        assert_eq!(
            manifest.accounts.map(|a| a.profile),
            spec.accounts.map(|a| a.profile.map(|p| ProfileHome {
                vars: strings(p.vars),
                default_dir: p.default_dir.to_owned(),
                clear: strings(p.clear),
            })),
            "{kind:?} accounts"
        );
        // Every usage source today is the module named after its provider.
        assert_eq!(
            manifest.usage.as_deref(),
            spec.usage.map(|_| spec.id),
            "{kind:?} usage"
        );
        let features = &spec.features;
        assert_eq!(
            (
                manifest.features.side_threads,
                manifest.features.rewind,
                manifest.features.raw_slash_commands,
                manifest.features.steer.map(|s| s.ack_timeout),
            ),
            (
                features.side_threads,
                features.rewind,
                features.raw_slash_commands,
                features.steer,
            ),
            "{kind:?} features"
        );
        assert_eq!(
            manifest.discovery_timeout, spec.discovery_timeout,
            "{kind:?} discovery_timeout"
        );
    }

    #[test]
    fn every_provider_has_exactly_one_bundled_manifest() {
        for kind in Kind::ALL {
            let count = BUNDLED.iter().filter(|(k, _)| *k == kind).count();
            assert_eq!(count, 1, "{kind:?}");
            assert_eq!(bundled(kind).id, kind.id());
            assert!(bundled(kind).min_engine_version <= ENGINE_VERSION);
        }
    }

    #[test]
    fn bundled_manifests_mirror_the_spec_table() {
        for kind in Kind::ALL {
            assert_mirrors_spec(kind);
        }
    }

    #[test]
    fn devin_install_hint_follows_the_platform() {
        let command = bundled(Kind::Devin).install_command.unwrap();
        assert_eq!(
            command.for_os(true),
            "irm https://static.devin.ai/cli/setup.ps1 | iex"
        );
        assert_eq!(
            command.for_os(false),
            "curl -fsSL https://cli.devin.ai/install.sh | bash"
        );
        // A single string is the same on every platform.
        let same = bundled(Kind::Grok).install_command.unwrap();
        assert_eq!(same.for_os(true), same.for_os(false));
    }

    #[test]
    fn unknown_fields_are_rejected_rather_than_ignored() {
        let content = include_str!("manifests/kiro.toml").replace(
            "label = \"Kiro CLI\"",
            "label = \"Kiro CLI\"\nlable = \"typo\"",
        );
        let error = parse(&content).unwrap_err();
        assert!(error.contains("lable"), "{error}");
        let nested = include_str!("manifests/kiro.toml")
            .replace("rewind = false", "rewind = false\nrewnd = true");
        assert!(parse(&nested).unwrap_err().contains("rewnd"));
    }

    #[test]
    fn manifests_that_need_a_newer_engine_or_lack_a_binary_are_invalid() {
        let base = include_str!("manifests/kiro.toml");
        let newer = base.replace(
            "min_engine_version = 1",
            &format!("min_engine_version = {}", ENGINE_VERSION + 1),
        );
        assert!(parse(&newer).unwrap_err().contains("requires engine"));
        let blank = base.replace("binary = \"kiro-cli\"", "binary = \" \"");
        assert!(parse(&blank).unwrap_err().contains("binary"));
        let no_sign_in = base.replace("sign_in = [\"login\"]", "sign_in = []");
        assert!(parse(&no_sign_in).unwrap_err().contains("sign_in"));
    }

    #[test]
    fn without_an_override_folder_or_file_the_bundled_manifest_loads() {
        let loaded = load(Kind::Claude, None);
        assert_eq!(loaded.source, Source::Bundled);
        assert!(loaded.warning.is_none());

        let dir = tempfile::tempdir().unwrap();
        let loaded = load(Kind::Claude, Some(dir.path()));
        assert_eq!(loaded.source, Source::Bundled);
        assert!(loaded.warning.is_none());
        assert_eq!(loaded.manifest, bundled(Kind::Claude));
    }

    #[test]
    fn a_valid_override_shadows_the_bundled_manifest_for_its_provider_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grok.toml");
        let content = include_str!("manifests/grok.toml")
            .replace("binary = \"grok\"", "binary = \"grok-nightly\"");
        std::fs::write(&path, content).unwrap();

        let loaded = load(Kind::Grok, Some(dir.path()));
        assert_eq!(loaded.source, Source::Override(path));
        assert_eq!(loaded.manifest.binary, "grok-nightly");
        assert!(loaded.warning.is_none());
        assert_eq!(load(Kind::Claude, Some(dir.path())).source, Source::Bundled);
    }

    #[test]
    fn a_broken_override_falls_back_to_the_bundled_manifest_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("codex.toml");

        std::fs::write(&path, "id = \"codex\"\nlabel = ").unwrap();
        let loaded = load(Kind::Codex, Some(dir.path()));
        assert_eq!(loaded.source, Source::Bundled);
        assert_eq!(loaded.manifest, bundled(Kind::Codex));
        assert!(loaded.warning.unwrap().contains("could not be loaded"));

        // Another provider's manifest under this provider's file name.
        std::fs::write(&path, include_str!("manifests/claude.toml")).unwrap();
        let loaded = load(Kind::Codex, Some(dir.path()));
        assert_eq!(loaded.source, Source::Bundled);
        assert!(loaded.warning.unwrap().contains("does not match codex"));

        let newer = include_str!("manifests/codex.toml").replace(
            "min_engine_version = 1",
            &format!("min_engine_version = {}", ENGINE_VERSION + 1),
        );
        std::fs::write(&path, newer).unwrap();
        let loaded = load(Kind::Codex, Some(dir.path()));
        assert_eq!(loaded.source, Source::Bundled);
        assert!(loaded.warning.unwrap().contains("requires engine"));
    }
}
