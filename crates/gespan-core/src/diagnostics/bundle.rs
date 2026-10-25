//! The diagnostics bundle (`docs/diagnostics/bundle-format.md`). The report
//! sheet previews [`Bundle::entries`] and saves [`Bundle::zip`], which is
//! written from those same entries, so the preview is exactly what is sent.
use chrono::{DateTime, Datelike, SecondsFormat, Timelike, Utc};
use gespan_redact::Redactor;
use serde_json::{json, Value};
use std::{collections::BTreeSet, io::Write};

/// The largest zip the intake accepts.
pub const MAX_BUNDLE_BYTES: usize = 5 * 1024 * 1024;

/// Where the bundle goes. A fallback bundle is attached to a public GitHub
/// issue, so it leaves out the contact email and the install ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Intake,
    Fallback,
}

/// What the user wrote in the report sheet.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub title: String,
    pub description: String,
    pub steps: String,
    pub contact_email: Option<String>,
    pub event_ids: Vec<String>,
}

/// The app and the machine, for `report.json`.
#[derive(Clone, Debug)]
pub struct System {
    pub version: String,
    pub build_channel: &'static str,
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub locale: String,
}

/// Everything besides the user's text, already scrubbed.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    /// The log tail, oldest line first, one line per `\n`.
    pub logs: String,
    /// `(file name, body)` per crash record, oldest first.
    pub crashes: Vec<(String, String)>,
    pub counters: Option<String>,
    pub environment: Option<String>,
    pub consent: Value,
    pub install_id: Option<String>,
}

/// What the user chose to include.
#[derive(Clone, Debug)]
pub struct Selection {
    pub attach: bool,
    pub logs: bool,
    /// Crash file names (without `crashes/`) the user removed.
    pub crashes_excluded: BTreeSet<String>,
    pub counters: bool,
    pub environment: bool,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            attach: true,
            logs: true,
            crashes_excluded: BTreeSet::new(),
            counters: true,
            environment: true,
        }
    }
}

/// When and as what this bundle was made. The sheet makes one when it opens
/// and keeps it, so rebuilding after an edit doesn't change the identity.
#[derive(Clone, Debug)]
pub struct Stamp {
    pub submission_id: String,
    pub created_at: DateTime<Utc>,
}

impl Stamp {
    pub fn now() -> Self {
        Self {
            submission_id: uuid::Uuid::new_v4().to_string(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Bundle {
    pub file_name: String,
    pub entries: Vec<Entry>,
    zip: Vec<u8>,
}

impl Bundle {
    /// The zip, written from [`Bundle::entries`].
    pub fn zip(&self) -> &[u8] {
        &self.zip
    }

    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    pub fn report(&self) -> Value {
        self.entry(REPORT)
            .and_then(|entry| serde_json::from_slice(&entry.bytes).ok())
            .unwrap_or(Value::Null)
    }
}

const REPORT: &str = "report.json";
const LOGS: &str = "logs.jsonl";
const COUNTERS: &str = "counters.json";
const ENVIRONMENT: &str = "environment.json";

/// The `consent` field of `report.json`: always this default, because
/// Gespan has no sending choices (`docs/diagnostics/bundle-format.md`).
pub fn default_consent() -> Value {
    json!({
        "consent_version": 1,
        "crash_reports": "ask",
        "metrics": false,
        "include_logs": true,
        "decided_at": null,
    })
}

/// Builds the bundle, dropping the oldest log lines and then the oldest
/// crash records until the zip fits in `cap`. `report.json` always stays.
#[allow(clippy::too_many_arguments)]
pub fn build(
    report: &Report,
    system: &System,
    sources: &Sources,
    selection: &Selection,
    destination: Destination,
    redactor: &Redactor,
    stamp: &Stamp,
    cap: usize,
) -> Result<Bundle, String> {
    let attach = selection.attach;
    let mut lines: Vec<&str> = if attach && selection.logs {
        sources.logs.lines().collect()
    } else {
        Vec::new()
    };
    let mut crashes: Vec<&(String, String)> = if attach {
        sources
            .crashes
            .iter()
            .filter(|(name, _)| !selection.crashes_excluded.contains(name))
            .collect()
    } else {
        Vec::new()
    };
    let counters = sources
        .counters
        .as_ref()
        .filter(|_| attach && selection.counters);
    let environment = sources
        .environment
        .as_ref()
        .filter(|_| attach && selection.environment);
    let user = UserText::scrubbed(report, redactor);
    loop {
        let mut others = Vec::new();
        if !lines.is_empty() {
            others.push(entry(LOGS, lines.join("\n")));
        }
        for (name, body) in &crashes {
            others.push(entry(&format!("crashes/{name}"), body.clone()));
        }
        if let Some(counters) = counters {
            others.push(entry(COUNTERS, counters.clone()));
        }
        if let Some(environment) = environment {
            others.push(entry(ENVIRONMENT, environment.clone()));
        }
        let files: Vec<&str> = others.iter().map(|entry| entry.name.as_str()).collect();
        let body = report_json(&user, report, system, sources, destination, stamp, &files);
        let mut entries = vec![entry(REPORT, body)];
        entries.extend(others);
        let zip = write_zip(&entries, stamp.created_at)?;
        if zip.len() <= cap || (lines.is_empty() && crashes.is_empty()) {
            return Ok(Bundle {
                file_name: file_name(&system.version, stamp.created_at),
                entries,
                zip,
            });
        }
        if lines.is_empty() {
            crashes.remove(0);
        } else {
            let drop = lines.len().div_ceil(10);
            lines.drain(..drop);
        }
    }
}

/// `gespan-diagnostics-native-<version>-<YYYYMMDDTHHMMSSZ>.zip`.
fn file_name(version: &str, at: DateTime<Utc>) -> String {
    format!(
        "gespan-diagnostics-native-{version}-{}.zip",
        at.format("%Y%m%dT%H%M%SZ")
    )
}

/// UTF-8 text with `\n` line endings, ending in one newline.
fn entry(name: &str, text: String) -> Entry {
    let mut text = text.replace("\r\n", "\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Entry {
        name: name.into(),
        bytes: text.into_bytes(),
    }
}

/// The user's fields after scrubbing. The contact email is the user's own and
/// stays as typed: it never reaches the public issue.
struct UserText {
    title: String,
    description: String,
    steps: String,
}

impl UserText {
    fn scrubbed(report: &Report, redactor: &Redactor) -> Self {
        let scrub = |text: &str| redactor.redact_text(text.trim());
        Self {
            title: scrub(&report.title),
            description: scrub(&report.description),
            steps: scrub(&report.steps),
        }
    }
}

fn report_json(
    user: &UserText,
    report: &Report,
    system: &System,
    sources: &Sources,
    destination: Destination,
    stamp: &Stamp,
    files: &[&str],
) -> String {
    let mut body = json!({
        "bundle_version": 1,
        "submission_id": stamp.submission_id,
        "kind": "user",
        "created_at": stamp.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        "app": "native",
        "version": system.version,
        "build_channel": system.build_channel,
        "os": system.os,
        "os_version": system.os_version,
        "arch": system.arch,
        "locale": system.locale,
        "title": user.title,
        "description": user.description,
        "event_ids": report.event_ids,
        "consent": sources.consent,
        "files": files,
    });
    if !user.steps.is_empty() {
        body["steps"] = user.steps.clone().into();
    }
    if destination == Destination::Intake {
        if let Some(email) = report
            .contact_email
            .as_deref()
            .map(str::trim)
            .filter(|email| !email.is_empty())
        {
            body["contact_email"] = email.into();
        }
        if let Some(id) = &sources.install_id {
            body["install_id"] = id.clone().into();
        }
    }
    serde_json::to_string_pretty(&body).expect("report.json serializes")
}

/// Deflated entries in order, all stamped with the bundle's time, so the
/// same entries always give the same bytes.
fn write_zip(entries: &[Entry], at: DateTime<Utc>) -> Result<Vec<u8>, String> {
    let time = zip::DateTime::from_date_and_time(
        at.year().clamp(1980, 2107) as u16,
        at.month() as u8,
        at.day() as u8,
        at.hour() as u8,
        at.minute() as u8,
        at.second() as u8,
    )
    .map_err(|error| error.to_string())?;
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(time)
        .unix_permissions(0o644);
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for entry in entries {
        writer
            .start_file(entry.name.as_str(), options)
            .map_err(|error| error.to_string())?;
        writer
            .write_all(&entry.bytes)
            .map_err(|error| error.to_string())?;
    }
    writer
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Read;

    /// The zip's entries, in order.
    pub(crate) fn unzip(bytes: &[u8]) -> Vec<Entry> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut file = archive.by_index(i).unwrap();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                Entry {
                    name: file.name().to_owned(),
                    bytes,
                }
            })
            .collect()
    }

    pub(crate) fn system() -> System {
        System {
            version: "0.1.0".into(),
            build_channel: "release",
            os: "macos".into(),
            os_version: "15.1".into(),
            arch: "aarch64".into(),
            locale: "en-GB".into(),
        }
    }

    pub(crate) fn stamp() -> Stamp {
        Stamp {
            submission_id: "3f9b2c71-8d4e-4a6f-b1c5-0e7d9a2f6b38".into(),
            created_at: DateTime::parse_from_rfc3339("2026-10-25T09:14:03Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    fn typed() -> Report {
        Report {
            title: "New worktree fails".into(),
            description: "Nothing happens.".into(),
            steps: String::new(),
            contact_email: Some("me@example.com".into()),
            event_ids: vec![],
        }
    }

    fn sources() -> Sources {
        Sources {
            logs: "{\"n\":1}\n{\"n\":2}".into(),
            crashes: vec![("a.json".into(), "{\"message\":\"boom\"}".into())],
            counters: None,
            environment: Some("{\"providers\":[]}".into()),
            consent: default_consent(),
            install_id: Some("c2d8e4a1-7b3f-4e9c-a6d0-5f1b8e2c9a74".into()),
        }
    }

    fn redactor() -> Redactor {
        Redactor::new(gespan_redact::Config {
            home: Some("/Users/alice".into()),
            ..Default::default()
        })
    }

    fn make(report: &Report, sources: &Sources, selection: &Selection, to: Destination) -> Bundle {
        build(
            report,
            &system(),
            sources,
            selection,
            to,
            &redactor(),
            &stamp(),
            MAX_BUNDLE_BYTES,
        )
        .unwrap()
    }

    fn fallback() -> Bundle {
        make(
            &typed(),
            &sources(),
            &Selection::default(),
            Destination::Fallback,
        )
    }

    #[test]
    fn zip_holds_exactly_the_preview_entries() {
        let bundle = fallback();
        assert_eq!(unzip(bundle.zip()), bundle.entries);
        let names: Vec<_> = bundle.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "report.json",
                "logs.jsonl",
                "crashes/a.json",
                "environment.json"
            ]
        );
        assert_eq!(
            bundle.report()["files"],
            json!(["logs.jsonl", "crashes/a.json", "environment.json"])
        );
        assert_eq!(
            bundle.entry("logs.jsonl").unwrap().bytes,
            b"{\"n\":1}\n{\"n\":2}\n"
        );
    }

    #[test]
    fn unticked_attach_sends_only_report_json() {
        let selection = Selection {
            attach: false,
            ..Selection::default()
        };
        let bundle = make(&typed(), &sources(), &selection, Destination::Fallback);
        assert_eq!(bundle.entries.len(), 1);
        assert_eq!(bundle.report()["files"], json!([]));
        assert_eq!(unzip(bundle.zip()).len(), 1);
    }

    #[test]
    fn fallback_leaves_out_contact_email_and_install_id() {
        let report = fallback().report();
        assert!(report.get("contact_email").is_none());
        assert!(report.get("install_id").is_none());
    }

    #[test]
    fn intake_keeps_contact_email_unscrubbed() {
        let report = make(
            &typed(),
            &sources(),
            &Selection::default(),
            Destination::Intake,
        )
        .report();
        assert_eq!(report["contact_email"], "me@example.com");
        assert_eq!(report["install_id"], "c2d8e4a1-7b3f-4e9c-a6d0-5f1b8e2c9a74");
    }

    #[test]
    fn report_json_matches_the_contract() {
        let bundle = fallback();
        let report = bundle.report();
        assert_eq!(report["bundle_version"], 1);
        assert_eq!(report["kind"], "user");
        assert_eq!(report["app"], "native");
        assert_eq!(
            report["submission_id"],
            "3f9b2c71-8d4e-4a6f-b1c5-0e7d9a2f6b38"
        );
        assert_eq!(report["created_at"], "2026-10-25T09:14:03.000Z");
        assert_eq!(report["version"], "0.1.0");
        assert_eq!(report["build_channel"], "release");
        assert_eq!(report["os"], "macos");
        assert_eq!(report["os_version"], "15.1");
        assert_eq!(report["arch"], "aarch64");
        assert_eq!(report["locale"], "en-GB");
        assert_eq!(report["title"], "New worktree fails");
        assert_eq!(report["description"], "Nothing happens.");
        assert!(report.get("steps").is_none(), "empty steps are omitted");
        assert_eq!(report["event_ids"], json!([]));
        assert_eq!(report["consent"], default_consent());
        assert_eq!(
            bundle.file_name,
            "gespan-diagnostics-native-0.1.0-20261025T091403Z.zip"
        );
        let with_steps = Report {
            steps: "1. Open\n2. Click".into(),
            event_ids: vec!["0b7f1c2e-5a4d-4e8b-9f3a-2c6d1e0a9b47".into()],
            ..typed()
        };
        let report = make(
            &with_steps,
            &sources(),
            &Selection::default(),
            Destination::Fallback,
        )
        .report();
        assert_eq!(report["steps"], "1. Open\n2. Click");
        assert_eq!(
            report["event_ids"],
            json!(["0b7f1c2e-5a4d-4e8b-9f3a-2c6d1e0a9b47"])
        );
    }

    #[test]
    fn user_text_is_scrubbed() {
        let typed = Report {
            title: "crash in /Users/alice/x".into(),
            description: "token ghp_16C7e42F292c6912E7710c838347Ae178B4a, mail bob@x.io".into(),
            steps: "open /Users/alice/code".into(),
            ..typed()
        };
        let report = make(
            &typed,
            &sources(),
            &Selection::default(),
            Destination::Fallback,
        )
        .report();
        assert_eq!(report["title"], "crash in ~/x");
        assert_eq!(report["description"], "token <redacted>, mail <email>");
        assert_eq!(report["steps"], "open ~/code");
    }

    #[test]
    fn excluded_entries_are_left_out() {
        let mut sources = sources();
        sources
            .crashes
            .push(("b.json".into(), "{\"message\":\"two\"}".into()));
        sources.counters = Some("{\"counters\":[]}".into());
        let selection = Selection {
            logs: false,
            crashes_excluded: ["a.json".to_string()].into(),
            counters: false,
            ..Selection::default()
        };
        let bundle = make(&typed(), &sources, &selection, Destination::Fallback);
        assert_eq!(
            bundle.report()["files"],
            json!(["crashes/b.json", "environment.json"])
        );
        assert_eq!(unzip(bundle.zip()), bundle.entries);
    }

    /// Text deflate can't shrink, so a byte cap means a line count.
    fn noise(seed: u64, len: usize) -> String {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                char::from(b'a' + (state >> 59) as u8 % 26)
            })
            .collect()
    }

    #[test]
    fn over_the_cap_drops_oldest_logs_then_oldest_crashes() {
        let lines: Vec<String> = (0..200)
            .map(|i| format!("{i:03} {}", noise(i, 200)))
            .collect();
        let sources = Sources {
            logs: lines.join("\n"),
            crashes: vec![
                ("old.json".into(), noise(1000, 4000)),
                ("new.json".into(), noise(1001, 4000)),
            ],
            environment: None,
            ..sources()
        };
        let build_with = |cap, selection: &Selection| {
            build(
                &typed(),
                &system(),
                &sources,
                selection,
                Destination::Fallback,
                &redactor(),
                &stamp(),
                cap,
            )
            .unwrap()
        };
        let all = Selection::default();
        let full = build_with(usize::MAX, &all);
        let cap = full.zip().len() - 10_000;
        let trimmed = build_with(cap, &all);
        assert!(trimmed.zip().len() <= cap);
        let logs = String::from_utf8(trimmed.entry("logs.jsonl").unwrap().bytes.clone()).unwrap();
        assert!(logs.starts_with(&lines[lines.len() - logs.lines().count()]));
        assert!(
            logs.ends_with(&format!("{}\n", lines[199])),
            "newest line stays"
        );
        assert!(!logs.contains(&lines[0]), "oldest line goes");
        assert_eq!(
            trimmed.entries.len(),
            4,
            "both crashes stay while logs can go"
        );

        // Room for the report and one crash only.
        let only_new = Selection {
            logs: false,
            crashes_excluded: ["old.json".to_string()].into(),
            ..Selection::default()
        };
        let cap = build_with(usize::MAX, &only_new).zip().len() + 100;
        let one_crash = build_with(cap, &all);
        assert_eq!(
            one_crash.report()["files"],
            json!(["crashes/new.json"]),
            "logs go first, then the oldest crash"
        );
        assert!(one_crash.zip().len() <= cap);

        let nothing_fits = build_with(10, &all);
        assert_eq!(nothing_fits.entries.len(), 1);
        assert_eq!(nothing_fits.entries[0].name, "report.json");
    }

    #[test]
    fn same_input_same_bytes() {
        assert_eq!(fallback().zip(), fallback().zip());
    }
}
