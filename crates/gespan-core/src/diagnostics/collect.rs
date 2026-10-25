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

/// The redactor for this machine: its home folder and the given project roots.
pub fn redactor_for(projects: &[String]) -> Redactor {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|home| !home.is_empty())
        .map(PathBuf::from);
    Redactor::new(gespan_redact::Config {
        home,
        projects: projects.iter().map(PathBuf::from).collect(),
        secrets: Vec::new(),
    })
}

/// Every folder a report should hide behind `<project-N>`: the projects
/// first, then each one's sibling worktree folder, then `others` (archived
/// projects, session folders). Relative paths and repeats are left out.
pub fn report_roots(projects: &[String], others: &[String]) -> Vec<String> {
    let worktrees = projects.iter().map(|project| {
        crate::worktree_management::default_root(Path::new(project))
            .to_string_lossy()
            .into_owned()
    });
    let mut roots: Vec<String> = Vec::new();
    for root in projects
        .iter()
        .cloned()
        .chain(worktrees)
        .chain(others.iter().cloned())
    {
        if Path::new(&root).is_absolute() && !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
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

    /// #72: the scrubber's shared fixtures pass inside the bundle path, from
    /// files on disk to the zip's bytes.
    #[test]
    fn every_redaction_fixture_passes_through_the_bundle() {
        use gespan_redact::fixtures::{self, Kind};
        let file = fixtures::load();
        let now = at("2026-10-25T10:00:00Z");
        let mut failures = Vec::new();
        for case in &file.cases {
            let dir = tempfile::tempdir().unwrap();
            match case.kind {
                Kind::Text => write_day(dir.path(), "2026-10-25", case.input.as_str().unwrap()),
                Kind::Json => {
                    let crashes = crash_dir(dir.path());
                    std::fs::create_dir_all(&crashes).unwrap();
                    std::fs::write(crashes.join("case.json"), case.input.to_string()).unwrap();
                }
            }
            let sources = collect(dir.path(), &file.redactor(&case.context), now, None, None);
            let bundle = build(
                &Report::default(),
                &system(),
                &sources,
                &Selection::default(),
                Destination::Fallback,
                &file.redactor(&case.context),
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
        let roots = report_roots(
            &["/Users/alice/Developer/acme".into(), "/w/app".into()],
            &[
                "/Users/alice/Developer/acme-worktrees/feature-x".into(),
                "/w/app".into(),
                "/Users/alice/old".into(),
                "relative/dir".into(),
                "".into(),
            ],
        );
        assert_eq!(
            roots,
            [
                "/Users/alice/Developer/acme",
                "/w/app",
                "/Users/alice/Developer/acme-worktrees",
                "/w/app-worktrees",
                "/Users/alice/Developer/acme-worktrees/feature-x",
                "/Users/alice/old",
            ]
        );
    }

    #[test]
    fn worktree_and_branch_names_are_scrubbed_from_logs() {
        let dir = tempfile::tempdir().unwrap();
        write_day(
            dir.path(),
            "2026-10-25",
            &[
                r#"{"message":"git failed in /Users/alice/Developer/acme-worktrees/feature-x/src"}"#,
                r#"{"message":"created /Users/alice/Developer/acme-worktrees/fix-y"}"#,
            ]
            .join("\n"),
        );
        let roots = report_roots(
            &["/Users/alice/Developer/acme".into()],
            &["/Users/alice/Developer/acme-worktrees/feature-x".into()],
        );
        let redactor = gespan_redact::Redactor::new(gespan_redact::Config {
            home: Some("/Users/alice".into()),
            projects: roots.iter().map(PathBuf::from).collect(),
            secrets: Vec::new(),
        });
        let logs = collect(
            dir.path(),
            &redactor,
            at("2026-10-25T10:00:00Z"),
            None,
            None,
        )
        .logs;
        assert!(
            !logs.contains("acme") && !logs.contains("feature-x"),
            "{logs}"
        );
        assert!(logs.contains("<project-3>/src"), "{logs}");
    }

    #[test]
    fn build_channel_follows_data_and_version() {
        assert_eq!(build_channel("0.1.0", Channel::Release), "release");
        assert_eq!(
            build_channel("0.1.0-preview.2", Channel::Release),
            "preview"
        );
        assert_eq!(build_channel("0.1.0-preview.2", Channel::Dev), "dev");
    }
}
