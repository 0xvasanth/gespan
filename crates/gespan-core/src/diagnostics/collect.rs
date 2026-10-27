//! Gathers what a report attaches, scrubbed once: the log tail, crash
//! records, installed provider CLIs and facts about this machine.
use super::{crash_dir, default_consent, rotation::scan, Channel, Sources, System, STARTED};
use chrono::{DateTime, Duration, Utc};
use gespan_redact::Redactor;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// At most this many log lines go in a report.
pub const TAIL_LINES: usize = 2_000;

/// What `environment.json` holds: no paths, names or URLs.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    /// `(provider id, version)` per CLI found on this machine.
    pub providers: Vec<(String, Option<String>)>,
    pub projects_count: usize,
    pub sessions_count: usize,
}

/// The last `max_lines` log lines from the last 24 hours, oldest first,
/// unscrubbed. Lines without a readable `ts` (a raw panic message, say) are
/// kept: they belong to the record around them.
pub fn log_tail(log_dir: &Path, now: DateTime<Utc>, max_lines: usize) -> String {
    let since = now - Duration::hours(24);
    let mut files = scan(log_dir).unwrap_or_default();
    files.retain(|(day, _, _)| *day >= since.date_naive());
    files.sort_by_key(|(day, _, _)| *day);
    let mut lines = Vec::new();
    for (_, path, _) in files {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        lines.extend(
            String::from_utf8_lossy(&bytes)
                .lines()
                .filter(|line| !line.is_empty() && !older_than(line, since))
                .map(str::to_owned),
        );
    }
    let skip = lines.len().saturating_sub(max_lines);
    lines[skip..].join("\n")
}

fn older_than(line: &str, since: DateTime<Utc>) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|value| {
            DateTime::parse_from_rfc3339(value.get("ts")?.as_str()?)
                .ok()
                .map(|ts| ts.with_timezone(&Utc) < since)
        })
        .unwrap_or(false)
}

/// Everything a report can attach, scrubbed by `redactor`. The log tail is
/// scrubbed as one text, as the scrubber expects of a tail whose first line
/// may be cut; JSON files are scrubbed field by field.
pub fn collect(
    log_dir: &Path,
    redactor: &Redactor,
    now: DateTime<Utc>,
    environment: Option<Environment>,
    counters: Option<Value>,
) -> Sources {
    Sources {
        logs: redactor.redact_text(&log_tail(log_dir, now, TAIL_LINES)),
        crashes: crash_records(log_dir)
            .into_iter()
            .map(|(name, text)| (name, scrub_json_text(redactor, &text)))
            .collect(),
        counters: counters.map(|value| pretty(redactor.redact_value(value))),
        environment: environment.map(|environment| environment_json(redactor, environment)),
        consent: default_consent(),
        install_id: None,
    }
}

/// `environment.json`: provider ids and versions, and counts.
pub fn environment_json(redactor: &Redactor, environment: Environment) -> String {
    pretty(redactor.redact_value(json!({
        "providers": environment
            .providers
            .into_iter()
            .map(|(id, version)| json!({ "id": id, "version": version }))
            .collect::<Vec<_>>(),
        "projects_count": environment.projects_count,
        "sessions_count": environment.sessions_count,
    })))
}

/// `(file name, contents)` of each crash record, oldest first.
pub(crate) fn crash_records(log_dir: &Path) -> Vec<(String, String)> {
    let mut records: Vec<(String, String)> = std::fs::read_dir(crash_dir(log_dir))
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_str()?.to_owned();
                    if !name.ends_with(".json") {
                        return None;
                    }
                    let bytes = std::fs::read(entry.path()).ok()?;
                    Some((name, String::from_utf8_lossy(&bytes).into_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    records.sort();
    records
}

/// A crash record waiting for the crash notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingCrash {
    /// The file name in `crashes/`.
    pub name: String,
    /// `Crash in <file>:<line>`, never the panic message: the title becomes
    /// public (see `bundle-format.md`).
    pub title: String,
}

/// Unsent crash records, newest first.
pub fn pending_crashes(log_dir: &Path) -> Vec<PendingCrash> {
    let mut pending: Vec<PendingCrash> = crash_records(log_dir)
        .into_iter()
        .map(|(name, text)| PendingCrash {
            title: serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|record| crash_location(&record["location"]))
                .map(|at| format!("Crash in {at}"))
                .unwrap_or_else(|| "Gespan quit unexpectedly".into()),
            name,
        })
        .collect();
    pending.reverse();
    pending
}

/// `file:line` from `{ file, line }` or from a `file:line:column` string.
fn crash_location(location: &Value) -> Option<String> {
    if let Some(file) = location["file"].as_str() {
        return Some(format!("{file}:{}", location["line"].as_u64()?));
    }
    let mut parts = location.as_str()?.rsplitn(3, ':');
    let _column = parts.next()?;
    let line: u64 = parts.next()?.parse().ok()?;
    Some(format!("{}:{line}", parts.next()?))
}

/// A crash file that isn't JSON (cut short by the crash) is still scrubbed.
fn scrub_json_text(redactor: &Redactor, text: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(value) => pretty(redactor.redact_value(value)),
        Err(_) => redactor.redact_text(text),
    }
}

fn pretty(value: Value) -> String {
    serde_json::to_string_pretty(&value).expect("JSON serializes")
}

/// The provider CLIs on this machine and their versions. Runs each CLI's
/// `--version`, so call it off the UI thread.
pub fn installed_providers() -> Vec<(String, Option<String>)> {
    use crate::provider::Kind;
    let cwd = crate::scm::shell::dirs_home()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    std::thread::scope(|scope| {
        let jobs: Vec<_> = Kind::ALL
            .into_iter()
            .filter_map(|kind| Some((kind, crate::provider::resolve(kind)?)))
            .map(|(kind, path)| {
                let cwd = &cwd;
                scope.spawn(move || {
                    let version = crate::cli::output(
                        &path,
                        &["--version".into()],
                        cwd,
                        std::time::Duration::from_secs(10),
                    )
                    .ok()
                    .and_then(|text| crate::harness_updates::parse_version(&text));
                    (kind.id().to_owned(), version)
                })
            })
            .collect();
        jobs.into_iter().filter_map(|job| job.join().ok()).collect()
    })
}

/// Where a report's scrubber learns this machine's secrets.
#[derive(Clone, Debug, Default)]
pub struct Machine {
    pub home: Option<PathBuf>,
    /// Gespan's data folder: tracker credentials are keyed by it and account
    /// profiles live under it.
    pub data_dir: Option<PathBuf>,
    /// The default Codex home (`$CODEX_HOME` or `~/.codex`).
    pub codex_home: Option<PathBuf>,
}

impl Machine {
    /// This machine, for the app whose data folder is `data_dir`.
    pub fn this(data_dir: Option<PathBuf>) -> Self {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|home| !home.is_empty())
            .map(PathBuf::from);
        let codex_home = std::env::var_os("CODEX_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".codex")));
        Self {
            home,
            data_dir,
            codex_home,
        }
    }
}

/// The trackers whose credentials Gespan keeps in the keychain.
const TRACKERS: [gespan_tracker_secrets::Tracker; 5] = {
    use gespan_tracker_secrets::Tracker::*;
    [Linear, Jira, Gitlab, AzureDevOps, Plane]
};

/// Every secret value Gespan reads on this machine: tracker credentials
/// from the keychain, and the tokens in each Codex sign-in's `auth.json`
/// (the default one and every account profile). Reads the keychain and the
/// disk: call it off the UI thread.
pub fn known_secrets(machine: &Machine) -> Vec<String> {
    let mut secrets = Vec::new();
    if let Some(data_dir) = &machine.data_dir {
        for tracker in TRACKERS {
            if let Ok(Some(value)) = gespan_tracker_secrets::read(tracker, data_dir) {
                // Linear keeps the bare token; the others a JSON config.
                match serde_json::from_str::<Value>(&value) {
                    Ok(config @ Value::Object(_)) => credential_fields(&config, &mut secrets),
                    _ => secrets.push(value.trim().to_owned()),
                }
            }
        }
    }
    let profiles = machine.data_dir.iter().flat_map(|dir| {
        std::fs::read_dir(dir.join("provider-accounts").join("codex"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
    });
    for home in machine.codex_home.iter().cloned().chain(profiles) {
        if let Some(auth) = std::fs::read_to_string(home.join("auth.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        {
            credential_fields(&auth, &mut secrets);
        }
    }
    secrets.retain(|secret| !secret.is_empty());
    secrets.sort();
    secrets.dedup();
    secrets
}

/// The string values under credential-named keys (`token`, `id_token`,
/// `OPENAI_API_KEY`, `password`…), at any depth. Sites, URLs and account
/// ids aren't secrets, and taking them would scrub ordinary log text.
fn credential_fields(value: &Value, secrets: &mut Vec<String>) {
    fn credential(key: &str) -> bool {
        let key = key.to_ascii_lowercase();
        ["token", "secret", "password", "key", "pat"]
            .iter()
            .any(|word| key == *word || key.ends_with(&format!("_{word}")))
            || key.ends_with("token")
            || key.ends_with("apikey")
    }
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                // Everything under a credential-named key is a credential.
                if credential(key) {
                    strings(value, secrets);
                } else {
                    credential_fields(value, secrets);
                }
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| credential_fields(item, secrets)),
        _ => {}
    }
}

fn strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.trim().to_owned()),
        Value::Object(map) => map.values().for_each(|value| strings(value, out)),
        Value::Array(items) => items.iter().for_each(|item| strings(item, out)),
        _ => {}
    }
}

/// The redactor a report uses: this machine's home folder, the given
/// folders (see [`report_roots`]) and every [`known_secrets`] value.
pub fn redactor_for(machine: &Machine, roots: &[String]) -> Redactor {
    Redactor::new(gespan_redact::Config {
        home: machine.home.clone(),
        projects: roots.iter().map(PathBuf::from).collect(),
        secrets: known_secrets(machine),
    })
}

/// Every folder a report should hide behind `<project-N>`: the projects
/// first, then every worktree git lists for each one (the repository's own
/// folder too, which a project in a subfolder doesn't cover), each one's
/// `-worktrees` folder and the folders in it, then `others` (archived
/// projects, session and session worktree folders). Each folder is followed
/// by its real path when that differs (a symlink, or `/tmp` → `/private/tmp`
/// on macOS), since git and the OS report real paths. A `remote://` project
/// is hidden by its key and by its path on the other machine. Relative paths
/// and repeats are left out. Runs git and reads the disk: call it off the UI
/// thread.
pub fn report_roots(projects: &[String], others: &[String]) -> Vec<String> {
    let local: Vec<&String> = projects
        .iter()
        .filter(|project| !crate::files::remote::is_remote(Path::new(project)))
        .collect();
    let listed: Vec<String> = local
        .iter()
        .flat_map(|project| crate::git::worktrees(Path::new(project.as_str())).unwrap_or_default())
        .map(|tree| tree.path)
        .collect();
    // Before git knew of them, or for a project that isn't a repository.
    let worktree_dirs: Vec<PathBuf> = local
        .iter()
        .map(|project| Path::new(project.as_str()))
        .chain(listed.iter().map(Path::new))
        .flat_map(|path| {
            let real = real_path(path).map(PathBuf::from);
            std::iter::once(path.to_path_buf()).chain(real)
        })
        .map(|path| crate::worktree_management::default_root(&path))
        .collect();
    let found: Vec<String> = worktree_dirs
        .iter()
        .flat_map(|dir| {
            let mut trees: Vec<String> = std::fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter(|entry| entry.path().is_dir())
                        .map(|entry| entry.path().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            trees.sort();
            trees
        })
        .collect();
    let mut candidates: Vec<String> = Vec::new();
    for project in projects {
        candidates.push(project.clone());
        if let Some(host) = remote_host_path(project) {
            candidates.push(host);
        }
    }
    candidates.extend(listed);
    candidates.extend(
        worktree_dirs
            .iter()
            .map(|dir| dir.to_string_lossy().into_owned()),
    );
    candidates.extend(others.iter().cloned());
    candidates.extend(found);
    let spellings: Vec<(PathBuf, PathBuf)> = local
        .iter()
        .filter_map(|project| spelling(Path::new(project.as_str())))
        .collect();
    let mut roots: Vec<String> = Vec::new();
    let mut add = |root: String| {
        if !root.is_empty() && !roots.contains(&root) {
            roots.push(root);
        }
    };
    for root in candidates {
        let path = Path::new(&root);
        if crate::files::remote::is_remote(path) {
            add(root);
        // `/home/bob/acme` from another machine isn't absolute on Windows.
        } else if path.is_absolute() || root.starts_with('/') {
            let mut also: Vec<String> = real_path(path).into_iter().collect();
            also.extend(private_twin(&root));
            for (one, other) in &spellings {
                for (from, to) in [(one, other), (other, one)] {
                    if let Ok(rest) = path.strip_prefix(from) {
                        also.push(to.join(rest).to_string_lossy().into_owned());
                    }
                }
            }
            add(root);
            also.into_iter().for_each(&mut add);
        }
    }
    roots
}

/// The folders where a project's own spelling and its real path part, as
/// `(spelled, real)`: a linked parent folder, Windows' short `RUNNER~1`
/// names, or macOS's `/var` for `/private/var`. Git reports real paths, but
/// the app may log a worktree in the project's spelling, or the other way
/// round.
fn spelling(project: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut spelled = project.to_path_buf();
    let mut real = PathBuf::from(real_path(project)?);
    while spelled.file_name().is_some() && spelled.file_name() == real.file_name() {
        if !(spelled.pop() && real.pop()) {
            return None;
        }
    }
    (spelled != real && spelled.file_name().is_some() && real.file_name().is_some())
        .then_some((spelled, real))
}

/// The resolved path, without Windows' `\\?\` prefix, which logs never use.
fn real_path(path: &Path) -> Option<String> {
    let real = std::fs::canonicalize(path).ok()?;
    let real = real.to_string_lossy();
    Some(real.strip_prefix(r"\\?\").unwrap_or(&real).to_owned())
}

/// macOS's other spelling of a folder under `/var`, `/tmp` or `/etc`, which
/// are links into `/private`: git reports one and the app may log the other.
fn private_twin(root: &str) -> Option<String> {
    ["/var/", "/tmp/", "/etc/"].iter().find_map(|top| {
        if let Some(rest) = root.strip_prefix("/private") {
            rest.starts_with(top).then(|| rest.to_owned())
        } else {
            root.starts_with(top).then(|| format!("/private{root}"))
        }
    })
}

/// `/home/bob/acme` from `remote://devbox/home/bob/acme`.
fn remote_host_path(project: &str) -> Option<String> {
    let rest = project.strip_prefix("remote://")?;
    let path = &rest[rest.find('/')?..];
    (path.len() > 1).then(|| path.to_owned())
}

/// The running app's log folder, once diagnostics have started.
pub fn running_log_dir() -> Option<PathBuf> {
    STARTED.get().map(|config| config.log_dir.clone())
}

/// The running app's version and data channel, once diagnostics have started.
pub fn running_config() -> Option<(String, Channel)> {
    STARTED
        .get()
        .map(|config| (config.version.clone(), config.channel))
}

/// `release`, `preview` or `dev`, as `report.json` names them.
pub fn build_channel(version: &str, channel: Channel) -> &'static str {
    match channel {
        Channel::Dev => "dev",
        Channel::Release if version.contains("-preview") => "preview",
        Channel::Release if version.contains("-beta") => "beta",
        Channel::Release => "release",
    }
}

/// The app and this machine. Runs `sw_vers` or similar, so call it off the
/// UI thread.
pub fn system(version: &str, channel: Channel) -> System {
    let apple = if cfg!(target_os = "macos") {
        command_output("defaults", &["read", "-g", "AppleLocale"])
    } else {
        None
    };
    System {
        version: version.into(),
        build_channel: build_channel(version, channel),
        os: std::env::consts::OS.into(),
        os_version: os_version().unwrap_or_else(|| "unknown".into()),
        arch: std::env::consts::ARCH.into(),
        locale: locale_from(|name| std::env::var(name).ok(), apple),
    }
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}

fn os_version() -> Option<String> {
    if cfg!(target_os = "macos") {
        command_output("sw_vers", &["-productVersion"])
    } else if cfg!(windows) {
        // "Microsoft Windows [Version 10.0.22631.4317]"
        let text = command_output("cmd", &["/c", "ver"])?;
        let start = text.find("Version ")? + "Version ".len();
        Some(text[start..].trim_end_matches(']').trim().to_owned())
    } else {
        let release = std::fs::read_to_string("/etc/os-release").ok()?;
        release.lines().find_map(|line| {
            Some(
                line.strip_prefix("VERSION_ID=")?
                    .trim_matches('"')
                    .to_owned(),
            )
        })
    }
}

/// A BCP 47 tag from the POSIX locale variables (`en_GB.UTF-8` → `en-GB`),
/// then macOS's `AppleLocale`; `und` when neither names a language.
pub fn locale_from(env: impl Fn(&str) -> Option<String>, apple: Option<String>) -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(env)
        .chain(apple)
        .find_map(|raw| bcp47(&raw))
        .unwrap_or_else(|| "und".into())
}

fn bcp47(raw: &str) -> Option<String> {
    let tag = raw.split(['.', '@']).next()?.trim();
    if tag.is_empty() || tag == "C" || tag == "POSIX" {
        return None;
    }
    let tag = tag.replace('_', "-");
    tag.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
        .then_some(tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::bundle::tests::{stamp, system, unzip};
    use crate::diagnostics::{build, Destination, Report, Selection, MAX_BUNDLE_BYTES};
    use chrono::NaiveDate;
    use std::path::PathBuf;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn write_day(dir: &Path, day: &str, text: &str) {
        let day = NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
        std::fs::write(dir.join(super::super::file_name(day)), text).unwrap();
    }

    fn line(ts: &str, n: usize) -> String {
        format!("{{\"ts\":\"{ts}\",\"level\":\"info\",\"message\":\"m{n}\"}}")
    }

    #[test]
    fn tail_takes_the_last_lines_across_days() {
        let dir = tempfile::tempdir().unwrap();
        write_day(
            dir.path(),
            "2026-10-24",
            &format!(
                "{}\n{}\n",
                line("2026-10-24T20:00:00.000Z", 1),
                line("2026-10-24T21:00:00.000Z", 2)
            ),
        );
        write_day(
            dir.path(),
            "2026-10-25",
            &format!(
                "{}\n{}\n",
                line("2026-10-25T08:00:00.000Z", 3),
                line("2026-10-25T09:00:00.000Z", 4)
            ),
        );
        let tail = log_tail(dir.path(), at("2026-10-25T10:00:00Z"), 3);
        assert_eq!(
            tail,
            [
                line("2026-10-24T21:00:00.000Z", 2),
                line("2026-10-25T08:00:00.000Z", 3),
                line("2026-10-25T09:00:00.000Z", 4)
            ]
            .join("\n")
        );
    }

    #[test]
    fn tail_drops_lines_older_than_24_hours() {
        let dir = tempfile::tempdir().unwrap();
        write_day(
            dir.path(),
            "2026-10-23",
            &line("2026-10-23T23:00:00.000Z", 0),
        );
        write_day(
            dir.path(),
            "2026-10-24",
            &format!(
                "{}\n{}",
                line("2026-10-24T09:00:00.000Z", 1),
                line("2026-10-24T11:00:00.000Z", 2)
            ),
        );
        let tail = log_tail(dir.path(), at("2026-10-25T10:00:00Z"), TAIL_LINES);
        assert_eq!(tail, line("2026-10-24T11:00:00.000Z", 2));
    }

    #[test]
    fn tail_keeps_lines_without_a_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        write_day(
            dir.path(),
            "2026-10-25",
            &format!(
                "{}\nthread 'main' panicked at src/a.rs:1:1\n{{\"ts\":\"yesterday\"}}\n{}\n",
                line("2026-10-25T08:00:00.000Z", 1),
                line("2026-10-25T09:00:00.000Z", 2)
            ),
        );
        let tail = log_tail(dir.path(), at("2026-10-25T10:00:00Z"), TAIL_LINES);
        assert_eq!(tail.lines().count(), 4, "{tail}");
        assert!(tail.contains("panicked at src/a.rs"));
        assert!(tail.contains("{\"ts\":\"yesterday\"}"));
    }

    #[test]
    fn missing_log_folder_is_an_empty_tail() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            log_tail(&dir.path().join("none"), Utc::now(), TAIL_LINES),
            ""
        );
    }

    /// Puts each secret in the (in-memory) keychain as a tracker's saved
    /// credentials, the way the Inbox saves them.
    fn save_tracker_secrets(data_dir: &Path, secrets: &[String]) {
        use gespan_tracker_secrets::{self as trackers, testing, Tracker};
        testing::use_memory_keychain();
        let trackers = [
            Tracker::Linear,
            Tracker::Jira,
            Tracker::Gitlab,
            Tracker::AzureDevOps,
        ];
        assert!(
            secrets.len() <= trackers.len(),
            "more secrets than trackers"
        );
        for (tracker, secret) in trackers.into_iter().zip(secrets) {
            let value = match tracker {
                Tracker::Linear => secret.clone(),
                _ => json!({ "url": "https://example.com", "token": secret }).to_string(),
            };
            trackers::write(tracker, data_dir, &value).unwrap();
        }
    }

    /// #72: the scrubber's shared fixtures pass inside the bundle path, from
    /// files on disk to the zip's bytes. The redactor is the one the app
    /// builds: known secrets come from the keychain, not from the fixture.
    #[test]
    fn every_redaction_fixture_passes_through_the_bundle() {
        use gespan_redact::fixtures::{self, Kind};
        let file = fixtures::load();
        let now = at("2026-10-25T10:00:00Z");
        let mut failures = Vec::new();
        for case in &file.cases {
            let context = &file.contexts[&case.context];
            let data = tempfile::tempdir().unwrap();
            save_tracker_secrets(data.path(), &context.secrets);
            let redactor = redactor_for(
                &Machine {
                    home: context.home.as_ref().map(PathBuf::from),
                    data_dir: Some(data.path().to_owned()),
                    codex_home: None,
                },
                &context.projects,
            );
            let dir = tempfile::tempdir().unwrap();
            match case.kind {
                Kind::Text => write_day(dir.path(), "2026-10-25", case.input.as_str().unwrap()),
                Kind::Json => {
                    let crashes = crash_dir(dir.path());
                    std::fs::create_dir_all(&crashes).unwrap();
                    std::fs::write(crashes.join("case.json"), case.input.to_string()).unwrap();
                }
            }
            let sources = collect(dir.path(), &redactor, now, None, None);
            let bundle = build(
                &Report::default(),
                &system(),
                &sources,
                &Selection::default(),
                Destination::Fallback,
                &redactor,
                &stamp(),
                MAX_BUNDLE_BYTES,
            )
            .unwrap();
            let zipped = unzip(bundle.zip());
            let actual = match case.kind {
                Kind::Text => zipped
                    .iter()
                    .find(|entry| entry.name == "logs.jsonl")
                    .map(|entry| String::from_utf8(entry.bytes.clone()).unwrap().into())
                    .unwrap_or(serde_json::Value::Null),
                Kind::Json => zipped
                    .iter()
                    .find(|entry| entry.name == "crashes/case.json")
                    .map(|entry| serde_json::from_slice(&entry.bytes).unwrap())
                    .unwrap_or(serde_json::Value::Null),
            };
            let expected = match case.kind {
                // A bundle entry ends in exactly one newline.
                Kind::Text => {
                    let text = case.expected.as_str().unwrap().replace("\r\n", "\n");
                    format!("{}\n", text.strip_suffix('\n').unwrap_or(&text)).into()
                }
                Kind::Json => case.expected.clone(),
            };
            if actual != expected {
                failures.push(format!(
                    "{}\n  expected: {expected}\n  actual:   {actual}",
                    case.name
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn unparsable_crash_is_scrubbed_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let crashes = crash_dir(dir.path());
        std::fs::create_dir_all(&crashes).unwrap();
        std::fs::write(
            crashes.join("b.json"),
            "{\"message\": \"cut at /Users/alice/x",
        )
        .unwrap();
        std::fs::write(crashes.join("a.json"), "{\"message\": \"/Users/alice/y\"}").unwrap();
        std::fs::write(crashes.join("notes.txt"), "not a crash").unwrap();
        let redactor = gespan_redact::Redactor::new(gespan_redact::Config {
            home: Some("/Users/alice".into()),
            ..Default::default()
        });
        let sources = collect(dir.path(), &redactor, Utc::now(), None, None);
        let names: Vec<_> = sources
            .crashes
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["a.json", "b.json"]);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&sources.crashes[0].1).unwrap(),
            serde_json::json!({"message": "~/y"})
        );
        assert_eq!(sources.crashes[1].1, "{\"message\": \"cut at ~/x");
    }

    #[test]
    fn environment_json_has_only_ids_versions_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let environment = Environment {
            providers: vec![
                ("claude".into(), Some("2.1.4".into())),
                ("cursor".into(), None),
            ],
            projects_count: 4,
            sessions_count: 57,
        };
        let sources = collect(
            dir.path(),
            &gespan_redact::Redactor::new(Default::default()),
            Utc::now(),
            Some(environment),
            None,
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(sources.environment.as_deref().unwrap())
                .unwrap(),
            serde_json::json!({
                "providers": [
                    { "id": "claude", "version": "2.1.4" },
                    { "id": "cursor", "version": null }
                ],
                "projects_count": 4,
                "sessions_count": 57
            })
        );
        assert_eq!(sources.consent, crate::diagnostics::default_consent());
        assert!(sources.install_id.is_none());
        assert!(sources.counters.is_none());
    }

    #[test]
    fn locale_is_a_bcp_47_tag() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert_eq!(locale_from(env(&[("LANG", "en_GB.UTF-8")]), None), "en-GB");
        assert_eq!(
            locale_from(
                env(&[("LC_ALL", "de_DE@euro"), ("LANG", "en_GB.UTF-8")]),
                None
            ),
            "de-DE"
        );
        assert_eq!(
            locale_from(env(&[("LANG", "C.UTF-8")]), Some("fr_CA".into())),
            "fr-CA"
        );
        assert_eq!(locale_from(env(&[("LANG", "POSIX")]), None), "und");
        assert_eq!(locale_from(env(&[]), None), "und");
        assert_eq!(locale_from(env(&[("LANG", "ja")]), None), "ja");
    }

    #[test]
    fn pending_crashes_are_titled_by_location() {
        let dir = tempfile::tempdir().unwrap();
        let crashes = crash_dir(dir.path());
        std::fs::create_dir_all(&crashes).unwrap();
        std::fs::write(
            crashes.join("2026-10-24T08-00-00.000Z.json"),
            r#"{"message":"secret text","location":"src/a.rs:12:5"}"#,
        )
        .unwrap();
        std::fs::write(
            crashes.join("2026-10-25T08-00-00.000Z.json"),
            r#"{"location":{"file":"src/board.rs","line":212,"column":9}}"#,
        )
        .unwrap();
        std::fs::write(crashes.join("2026-10-25T09-00-00.000Z.json"), "{cut").unwrap();
        let pending = pending_crashes(dir.path());
        let titles: Vec<_> = pending.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Gespan quit unexpectedly",
                "Crash in src/board.rs:212",
                "Crash in src/a.rs:12"
            ]
        );
        assert_eq!(pending[2].name, "2026-10-24T08-00-00.000Z.json");
        assert!(pending_crashes(&dir.path().join("none")).is_empty());
    }

    #[test]
    fn report_roots_add_worktree_folders_and_skip_repeats() {
        // Absolute on every platform: `/Users/…` is not absolute on Windows.
        let base = std::env::temp_dir().join("gespan-report-roots-none");
        // One component at a time, so Windows gets `\` throughout.
        let path = |rel: &str| {
            rel.split('/')
                .fold(base.clone(), |path, part| path.join(part))
                .to_string_lossy()
                .into_owned()
        };
        let roots = report_roots(
            &[path("alice/Developer/acme"), path("w/app")],
            &[
                path("alice/Developer/acme-worktrees/feature-x"),
                path("w/app"),
                path("alice/old"),
                "relative/dir".into(),
                "".into(),
            ],
        );
        let expected: Vec<String> = [
            path("alice/Developer/acme"),
            path("w/app"),
            path("alice/Developer/acme-worktrees"),
            path("w/app-worktrees"),
            path("alice/Developer/acme-worktrees/feature-x"),
            path("alice/old"),
        ]
        .into_iter()
        // On macOS the temp folder is under `/var`, also spelled `/private/var`.
        .flat_map(|root| {
            let twin = private_twin(&root);
            std::iter::once(root).chain(twin)
        })
        .collect();
        assert_eq!(roots, expected);
    }

    /// A worktree with no session is still on disk; its branch name must
    /// not reach a report as `<project-N>/fix-y`.
    #[test]
    fn worktree_and_branch_names_are_scrubbed_from_logs() {
        let machine = tempfile::tempdir().unwrap();
        let home = machine.path().join("alice");
        let project = home.join("Developer/acme");
        let worktrees = home.join("Developer/acme-worktrees");
        for dir in [
            &project,
            &worktrees.join("feature-x"),
            &worktrees.join("fix-y"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let p = |path: &Path| path.to_string_lossy().into_owned();
        let dir = tempfile::tempdir().unwrap();
        write_day(
            dir.path(),
            "2026-10-25",
            &[
                format!(
                    r#"{{"message":"git failed in {}/src"}}"#,
                    p(&worktrees.join("feature-x"))
                ),
                format!(r#"{{"message":"created {}"}}"#, p(&worktrees.join("fix-y"))),
            ]
            .join("\n"),
        );
        // Only the project is known to the app; neither worktree has a session.
        let roots = report_roots(&[p(&project)], &[]);
        let redactor = redactor_for(
            &Machine {
                home: Some(home.clone()),
                ..Machine::default()
            },
            &roots,
        );
        let logs = collect(
            dir.path(),
            &redactor,
            at("2026-10-25T10:00:00Z"),
            None,
            None,
        )
        .logs;
        for name in ["acme", "feature-x", "fix-y"] {
            assert!(!logs.contains(name), "{name} in {logs}");
        }
        assert!(logs.contains("/src"), "{logs}");
    }

    fn scrub_logs(lines: &[String], redactor: &Redactor) -> String {
        let dir = tempfile::tempdir().unwrap();
        write_day(dir.path(), "2026-10-25", &lines.join("\n"));
        collect(dir.path(), redactor, at("2026-10-25T10:00:00Z"), None, None).logs
    }

    /// The keychain's tracker credentials and the Codex sign-ins Gespan
    /// reads are scrubbed, wherever they show up in a log.
    #[test]
    fn keychain_and_account_tokens_are_scrubbed_from_logs() {
        let data = tempfile::tempdir().unwrap();
        save_tracker_secrets(
            data.path(),
            &[
                "lin_api_LinearSecret01".into(),
                "jira-Secret-Token-02".into(),
                "glpat-GitlabSecret03".into(),
            ],
        );
        let codex = tempfile::tempdir().unwrap();
        std::fs::write(
            codex.path().join("auth.json"),
            json!({
                "OPENAI_API_KEY": "sk-proj-DefaultKey04",
                "tokens": {
                    "id_token": "eyJhbGciOi.defaultIdToken05.sig",
                    "access_token": "codex-access-Token06",
                    "account_id": "acct-not-a-secret"
                }
            })
            .to_string(),
        )
        .unwrap();
        let profile = data.path().join("provider-accounts/codex/work");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(
            profile.join("auth.json"),
            json!({ "tokens": { "refresh_token": "codex-refresh-Token07" } }).to_string(),
        )
        .unwrap();
        let machine = Machine {
            home: None,
            data_dir: Some(data.path().to_owned()),
            codex_home: Some(codex.path().to_owned()),
        };
        let secrets = [
            "lin_api_LinearSecret01",
            "jira-Secret-Token-02",
            "glpat-GitlabSecret03",
            "sk-proj-DefaultKey04",
            "eyJhbGciOi.defaultIdToken05.sig",
            "codex-access-Token06",
            "codex-refresh-Token07",
        ];
        // As plain words, so only the known-secret rule can catch them.
        let lines: Vec<String> = secrets
            .iter()
            .map(|secret| format!(r#"{{"message":"saw {secret} here"}}"#))
            .collect();
        let logs = scrub_logs(&lines, &redactor_for(&machine, &[]));
        for secret in secrets {
            assert!(!logs.contains(secret), "{secret} in {logs}");
        }
        assert_eq!(logs.matches("<redacted>").count(), secrets.len(), "{logs}");
        assert!(
            known_secrets(&machine)
                .iter()
                .all(|secret| secret != "acct-not-a-secret" && secret != "https://example.com"),
            "only credential fields are taken"
        );
    }

    #[test]
    fn no_data_folder_or_codex_home_means_no_known_secrets() {
        assert!(known_secrets(&Machine::default()).is_empty());
        let empty = tempfile::tempdir().unwrap();
        assert!(known_secrets(&Machine {
            home: None,
            data_dir: Some(empty.path().to_owned()),
            codex_home: Some(empty.path().join("none")),
        })
        .is_empty());
    }

    fn git(cwd: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
    }

    /// A repository with a commit, so it can have worktrees.
    fn repo(path: &Path) {
        std::fs::create_dir_all(path.join("app")).unwrap();
        std::fs::write(path.join("app/README"), "x").unwrap();
        git(path, &["init", "-q"]);
        git(path, &["add", "."]);
        git(path, &["commit", "-qm", "init"]);
    }

    fn assert_hidden(logs: &str, names: &[&str]) {
        for name in names {
            assert!(!logs.contains(name), "{name} in {logs}");
        }
        assert!(logs.contains("/src/lib.rs"), "{logs}");
    }

    /// Git puts worktrees beside the repository, not beside a project added
    /// as one of its subfolders; and it can put them anywhere.
    #[test]
    fn worktrees_of_a_project_in_a_repo_subfolder_are_hidden() {
        let machine = tempfile::tempdir().unwrap();
        let root = machine.path().join("acme-repo");
        repo(&root);
        let beside = machine.path().join("acme-repo-worktrees").join("feature-x");
        let elsewhere = machine.path().join("scratch").join("fix-y");
        let p = |path: &Path| path.to_string_lossy().into_owned();
        git(
            &root,
            &["worktree", "add", "-q", "-b", "feature-x", &p(&beside)],
        );
        git(
            &root,
            &["worktree", "add", "-q", "-b", "fix-y", &p(&elsewhere)],
        );
        let project = root.join("app");
        let roots = report_roots(&[p(&project)], &[]);
        let logs = scrub_logs(
            &[
                format!(r#"{{"message":"git failed in {}/src/lib.rs"}}"#, p(&beside)),
                format!(r#"{{"message":"created {}/src/lib.rs"}}"#, p(&elsewhere)),
                format!(
                    r#"{{"message":"read {}/src/lib.rs"}}"#,
                    p(&root.join("docs"))
                ),
            ],
            &redactor_for(&Machine::default(), &roots),
        );
        assert_hidden(&logs, &["acme-repo", "feature-x", "fix-y"]);
    }

    /// Git reports real paths, so worktrees of a project added through a
    /// symlink must be hidden by their real path too (and on macOS, temp
    /// folders resolve under `/private`).
    #[cfg(unix)]
    #[test]
    fn worktrees_of_a_project_added_through_a_symlink_are_hidden() {
        let machine = tempfile::tempdir().unwrap();
        let real = machine.path().join("real").join("acme-repo");
        repo(&real);
        let link = machine.path().join("acme-link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let p = |path: &Path| path.to_string_lossy().into_owned();
        let feature = machine
            .path()
            .join("real")
            .join("acme-repo-worktrees")
            .join("feature-x");
        git(
            &link,
            &["worktree", "add", "-q", "-b", "feature-x", &p(&feature)],
        );
        let roots = report_roots(&[p(&link)], &[]);
        let real = std::fs::canonicalize(&real).unwrap();
        let feature = std::fs::canonicalize(&feature).unwrap();
        let logs = scrub_logs(
            &[
                format!(
                    r#"{{"message":"git failed in {}/src/lib.rs"}}"#,
                    p(&feature)
                ),
                format!(r#"{{"message":"opened {}/src/lib.rs"}}"#, p(&real)),
            ],
            &redactor_for(&Machine::default(), &roots),
        );
        assert_hidden(&logs, &["acme-repo", "feature-x"]);
    }

    /// Git reports real paths, but the app may log a worktree in the
    /// project's own spelling: here a linked parent folder, as Windows' short
    /// `RUNNER~1` names or macOS's `/var` for `/private/var` are.
    #[cfg(unix)]
    #[test]
    fn worktrees_are_hidden_in_the_projects_own_spelling() {
        let machine = tempfile::tempdir().unwrap();
        let real_parent = machine.path().join("long-name");
        let repo_dir = real_parent.join("acme-repo");
        repo(&repo_dir);
        let short = machine.path().join("short");
        std::os::unix::fs::symlink(&real_parent, &short).unwrap();
        let p = |path: &Path| path.to_string_lossy().into_owned();
        let beside = real_parent.join("acme-repo-worktrees").join("feature-x");
        git(
            &repo_dir,
            &["worktree", "add", "-q", "-b", "feature-x", &p(&beside)],
        );
        let project = short.join("acme-repo").join("app");
        let roots = report_roots(&[p(&project)], &[]);
        let spelled = short.join("acme-repo-worktrees").join("feature-x");
        let logs = scrub_logs(
            &[
                format!(
                    r#"{{"message":"git failed in {}/src/lib.rs"}}"#,
                    p(&spelled)
                ),
                format!(
                    r#"{{"message":"read {}/src/lib.rs"}}"#,
                    p(&short.join("acme-repo"))
                ),
            ],
            &redactor_for(&Machine::default(), &roots),
        );
        assert_hidden(&logs, &["acme-repo", "feature-x"]);
    }

    /// A project on another machine is hidden by its `remote://` key and by
    /// its path there; neither is ever run through local git.
    #[test]
    fn remote_projects_are_hidden() {
        let roots = report_roots(&["remote://devbox/home/bob/acme".into()], &[]);
        assert_eq!(roots, ["remote://devbox/home/bob/acme", "/home/bob/acme"]);
        let logs = scrub_logs(
            &[
                r#"{"message":"list remote://devbox/home/bob/acme/src/lib.rs"}"#.into(),
                r#"{"message":"ssh: no /home/bob/acme/src/lib.rs"}"#.into(),
            ],
            &redactor_for(&Machine::default(), &roots),
        );
        assert_hidden(&logs, &["acme", "bob"]);
    }

    #[test]
    fn build_channel_follows_data_and_version() {
        assert_eq!(build_channel("0.1.0", Channel::Release), "release");
        assert_eq!(
            build_channel("0.1.0-preview.2", Channel::Release),
            "preview"
        );
        assert_eq!(build_channel("0.1.0-preview.2", Channel::Dev), "dev");
        assert_eq!(build_channel("0.0.1-beta", Channel::Release), "beta");
        assert_eq!(build_channel("0.0.1-beta.2", Channel::Dev), "dev");
    }
}
