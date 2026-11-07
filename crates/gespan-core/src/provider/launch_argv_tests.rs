//! Every provider's launch command, for a fixed set of settings, against a
//! recorded snapshot. A change to any provider's arguments or environment
//! shows up here as a diff to review. Re-record after an intended change with
//! `GESPAN_BLESS_LAUNCH_ARGV=1`.
use super::*;
use crate::agent::AccessMode;

const SNAPSHOT: &str = include_str!("fixtures/launch_argv.txt");

#[derive(Clone, Copy)]
enum Case {
    Supervised,
    ReadOnly,
    AutoAcceptEdits,
    FullAccess,
    Plan,
    ModelAndEffort,
    Resume,
    TextOnly,
    Probe,
    Fallback,
}
const CASES: [(Case, &str); 10] = [
    (Case::Supervised, "supervised"),
    (Case::ReadOnly, "read-only"),
    (Case::AutoAcceptEdits, "auto-accept-edits"),
    (Case::FullAccess, "full-access"),
    (Case::Plan, "plan"),
    (Case::ModelAndEffort, "model+effort"),
    (Case::Resume, "resume"),
    (Case::TextOnly, "text-only"),
    (Case::Probe, "probe"),
    (Case::Fallback, "fallback"),
];

fn request(kind: Kind, case: Case) -> Request {
    let mut request = Request {
        kind,
        binary: PathBuf::from("/opt/agents").join(kind.binary_name()),
        cwd: PathBuf::from("/work/project"),
        prompt: "hello".into(),
        provider_id: None,
        model: None,
        settings: Default::default(),
    };
    let settings = &mut request.settings;
    match case {
        Case::Supervised | Case::Probe | Case::Fallback => {}
        Case::ReadOnly => settings.access = AccessMode::ReadOnly,
        Case::AutoAcceptEdits => settings.access = AccessMode::AutoAcceptEdits,
        Case::FullAccess => settings.access = AccessMode::FullAccess,
        Case::Plan => settings.plan = true,
        Case::ModelAndEffort => {
            request.model = Some("model-x".into());
            request.settings.effort = Some("high".into());
        }
        Case::Resume => request.provider_id = Some("previous-session".into()),
        Case::TextOnly => settings.text_only = true,
    }
    request
}

/// Devin's supervised mode writes a config file from the user's own Devin
/// settings, so its launch depends on the machine; its other modes do not.
fn skipped(kind: Kind, request: &Request) -> bool {
    kind == Kind::Devin && request.settings.access == AccessMode::Supervised
}

/// Antigravity's server takes an empty `--uid=` on Linux only. Check it is
/// there, then drop it so one snapshot holds on every platform.
fn linux_neutral(kind: Kind, command: Command) -> Command {
    if !(cfg!(target_os = "linux") && kind == Kind::Antigravity) {
        return command;
    }
    let mut args = command.get_args();
    assert_eq!(args.next().and_then(|a| a.to_str()), Some("--uid="));
    let mut neutral = Command::new(command.get_program());
    neutral.args(args);
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => neutral.env(key, value),
            None => neutral.env_remove(key),
        };
    }
    neutral
}

fn render(command: &Command) -> String {
    let args: Vec<String> = command
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let mut envs: Vec<String> = command
        .get_envs()
        .map(|(k, v)| {
            format!(
                "{}={}",
                k.to_string_lossy(),
                v.map(|v| v.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "<removed>".into())
            )
        })
        .collect();
    envs.sort();
    let mut line = args.join(" ");
    if !envs.is_empty() {
        line.push_str(" | env ");
        line.push_str(&envs.join(" "));
    }
    line
}

fn snapshot(launch: impl Fn(&Request, Case) -> Result<Command, String>) -> String {
    let mut out = String::new();
    for kind in Kind::ALL {
        for (case, name) in CASES {
            let request = request(kind, case);
            if skipped(kind, &request) {
                continue;
            }
            let line = match launch(&request, case) {
                Ok(command) => render(&linux_neutral(kind, command)),
                Err(error) => format!("error: {error}"),
            };
            out.push_str(&format!("{} {name}: {line}\n", kind.id()));
        }
    }
    out
}

fn assert_snapshot(actual: String) {
    if std::env::var_os("GESPAN_BLESS_LAUNCH_ARGV").is_some() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/provider/fixtures/launch_argv.txt"
        );
        std::fs::write(path, &actual).unwrap();
        return;
    }
    if actual != SNAPSHOT {
        let diff: Vec<String> = SNAPSHOT
            .lines()
            .zip(actual.lines())
            .filter(|(want, got)| want != got)
            .map(|(want, got)| format!("- {want}\n+ {got}"))
            .collect();
        panic!(
            "launch commands changed ({} of {} lines; line counts {} vs {}):\n{}",
            diff.len(),
            SNAPSHOT.lines().count(),
            SNAPSHOT.lines().count(),
            actual.lines().count(),
            diff.join("\n")
        );
    }
}

#[test]
fn every_provider_launches_with_its_recorded_command() {
    // Recorded from the per-family match `launch()` used before the trait;
    // the trait's launch must still produce exactly that.
    assert_snapshot(snapshot(|request, case| {
        let provider = api::ProviderRegistry::builtin().get(request.kind);
        let mut command = Command::new(&request.binary);
        let session = provider.new_session_id(request).map(|_| "new-session-id");
        provider.configure(
            &mut command,
            request,
            api::Attempt {
                probe: matches!(case, Case::Probe),
                fallback: matches!(case, Case::Fallback),
            },
            session,
        )?;
        Ok(command)
    }));
}
