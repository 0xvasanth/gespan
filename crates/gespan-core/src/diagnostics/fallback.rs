//! Gespan never sends a report itself. The user saves the bundle to
//! Downloads and sends it on: by email to the maintainer (private), or in
//! GitHub's bug form with the report's fields filled in (public).
use super::Bundle;
use serde_json::Value;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const ISSUE_FORM: &str = "https://github.com/0xvasanth/gespan/issues/new";
/// Where "Save zip and email" addresses the report.
pub const REPORT_EMAIL: &str = "itparser+gespan@gmail.com";
/// GitHub refuses much longer URLs; the zip holds the full text anyway.
pub const MAX_URL_BYTES: usize = 8_000;
/// Windows hands at most 2,048 characters to the mail app, and some mail
/// apps cut long `mailto:` links, so the email link stays under this.
pub const MAX_MAILTO_BYTES: usize = 1_800;
/// Ends a field that was cut to fit the URL.
pub const CUT_NOTE: &str = "… (cut; see the attached zip)";

/// The bug form, prefilled from the bundle's own (scrubbed) `report.json`,
/// so the issue says exactly what the preview showed.
pub fn issue_url(report: &Value) -> String {
    let text = |key: &str| report[key].as_str().unwrap_or_default().to_owned();
    let mut fields = vec![
        ("template", "bug_report.yml".to_owned()),
        ("title", text("title")),
        ("version", text("version")),
        ("os", os_label(report)),
        ("os-version", text("os_version")),
        ("what-happened", text("description")),
    ];
    let steps = text("steps");
    if !steps.is_empty() {
        fields.push(("repro", steps));
    }
    // "What happened" is the form's required text: cut the steps first, the
    // title last.
    fit(
        &mut fields,
        &["repro", "what-happened", "title"],
        MAX_URL_BYTES,
        issue_query,
    );
    issue_query(&fields)
}

fn issue_query(fields: &[(&str, String)]) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().map(|(k, v)| (*k, v.as_str())))
        .finish();
    format!("{ISSUE_FORM}?{query}")
}

/// A `mailto:` link to [`REPORT_EMAIL`], filled from the bundle's own
/// (scrubbed) `report.json` like [`issue_url`]. A link can't carry the zip,
/// so the body names the saved file for the user to attach.
pub fn mail_url(report: &Value, zip_name: &str) -> String {
    let text = |key: &str| report[key].as_str().unwrap_or_default().to_owned();
    let mut fields = vec![
        ("title", text("title")),
        ("description", text("description")),
        ("steps", text("steps")),
    ];
    // The version table is never cut: without it the report can't be placed.
    let mut facts = format!(
        "Version: {} ({})\r\nOS: {} {} ({})",
        text("version"),
        text("build_channel"),
        os_label(report),
        text("os_version"),
        text("arch"),
    );
    let event_ids: Vec<&str> = report["event_ids"]
        .as_array()
        .map(|ids| ids.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !event_ids.is_empty() {
        facts += &format!("\r\nEvent IDs: {}", event_ids.join(", "));
    }
    let attach = format!("Please attach {zip_name} from your Downloads folder.");
    let render = |fields: &[(&str, String)]| {
        let mut body = format!("What happened:\r\n{}\r\n\r\n", fields[1].1);
        if !fields[2].1.is_empty() {
            body += &format!("Steps to reproduce:\r\n{}\r\n\r\n", fields[2].1);
        }
        body += &format!("{facts}\r\n\r\n{attach}\r\n");
        let subject = format!("[Gespan {}] {}", text("version"), fields[0].1);
        format!(
            "mailto:{}?subject={}&body={}",
            percent_encode(REPORT_EMAIL),
            percent_encode(&subject),
            percent_encode(&body)
        )
    };
    fit(
        &mut fields,
        &["steps", "description", "title"],
        MAX_MAILTO_BYTES,
        render,
    );
    render(&fields)
}

/// RFC 6068 wants spaces as `%20`, not the `+` of form encoding, and `+`
/// itself escaped so the address keeps its `+gespan` tag.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~@".contains(&byte) {
            out.push(byte as char);
        } else {
            out += &format!("%{byte:02X}");
        }
    }
    out
}

fn os_label(report: &Value) -> String {
    match report["os"].as_str().unwrap_or_default() {
        "macos" => "macOS".to_owned(),
        "linux" => "Linux".to_owned(),
        "windows" => "Windows".to_owned(),
        other => other.to_owned(),
    }
}

/// Cuts the named fields, in order, to the longest prefix (on a character
/// boundary, ending in [`CUT_NOTE`]) that keeps `render(fields)` within
/// `max` bytes. Fields that already fit are left alone.
fn fit(
    fields: &mut [(&str, String)],
    order: &[&str],
    max: usize,
    render: impl Fn(&[(&str, String)]) -> String,
) {
    for key in order {
        if render(fields).len() <= max {
            return;
        }
        let Some(ix) = fields.iter().position(|(k, _)| k == key) else {
            continue;
        };
        if fields[ix].1.is_empty() {
            continue;
        }
        let full: Vec<char> = fields[ix].1.chars().collect();
        let fits = |n: usize, fields: &mut [(&str, String)]| {
            fields[ix].1 = full[..n].iter().collect::<String>() + CUT_NOTE;
            render(fields).len() <= max
        };
        // The longest prefix that fits, by binary search.
        let (mut low, mut high) = (0, full.len());
        while low < high {
            let mid = (low + high).div_ceil(2);
            if fits(mid, fields) {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        fits(low, fields);
    }
}

/// The user's Downloads folder.
pub fn downloads_dir() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|home| !home.is_empty())?;
    Some(PathBuf::from(home).join("Downloads"))
}

/// Writes [`Bundle::zip`] into `dir` under the bundle's name, as `-1`, `-2`…
/// when that name is taken. Never overwrites a file.
pub fn save_bundle(bundle: &Bundle, dir: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stem = bundle
        .file_name
        .strip_suffix(".zip")
        .unwrap_or(&bundle.file_name);
    for attempt in 0.. {
        let path = dir.join(if attempt == 0 {
            format!("{stem}.zip")
        } else {
            format!("{stem}-{attempt}.zip")
        });
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        file.write_all(bundle.zip())?;
        file.sync_all()?;
        return Ok(path);
    }
    unreachable!("an unbounded range always yields another attempt")
}

/// Deletes the crash records a saved bundle carried. `names` are zip entry
/// names; anything that isn't a plain `crashes/<file>.json` is ignored.
pub fn remove_crashes(log_dir: &Path, names: &[String]) {
    let crashes = super::crash_dir(log_dir);
    for name in names {
        let Some(file) = name.strip_prefix("crashes/") else {
            continue;
        };
        let plain = Path::new(file)
            .file_name()
            .is_some_and(|only| only == std::ffi::OsStr::new(file));
        if plain && file.ends_with(".json") {
            let _ = std::fs::remove_file(crashes.join(file));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::bundle::tests::{stamp, system};
    use crate::diagnostics::{
        build, default_consent, Destination, Report, Selection, Sources, MAX_BUNDLE_BYTES,
    };
    use serde_json::json;
    use std::collections::HashMap;

    fn query(url: &str) -> (String, Vec<(String, String)>) {
        let url = url::Url::parse(url).unwrap();
        let base = format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().unwrap(),
            url.path()
        );
        (base, url.query_pairs().into_owned().collect())
    }

    fn report(title: &str, description: &str, steps: Option<&str>, os: &str) -> Value {
        let mut report = json!({
            "title": title,
            "description": description,
            "version": "0.1.0",
            "os": os,
            "os_version": "15.1",
        });
        if let Some(steps) = steps {
            report["steps"] = steps.into();
        }
        report
    }

    #[test]
    fn url_fills_every_field() {
        let (base, pairs) = query(&issue_url(&report(
            "Worktree fails",
            "Nothing happens.",
            Some("1. Click"),
            "macos",
        )));
        assert_eq!(base, "https://github.com/0xvasanth/gespan/issues/new");
        let expect: Vec<(String, String)> = [
            ("template", "bug_report.yml"),
            ("title", "Worktree fails"),
            ("version", "0.1.0"),
            ("os", "macOS"),
            ("os-version", "15.1"),
            ("what-happened", "Nothing happens."),
            ("repro", "1. Click"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        assert_eq!(pairs, expect);
        let (_, no_steps) = query(&issue_url(&report("t", "d", None, "macos")));
        assert!(no_steps.iter().all(|(key, _)| key != "repro"));
    }

    #[test]
    fn os_takes_the_form_label() {
        for (os, label) in [
            ("macos", "macOS"),
            ("linux", "Linux"),
            ("windows", "Windows"),
        ] {
            let (_, pairs) = query(&issue_url(&report("t", "d", None, os)));
            let pairs: HashMap<_, _> = pairs.into_iter().collect();
            assert_eq!(pairs["os"], label);
        }
    }

    #[test]
    fn url_uses_scrubbed_report_fields() {
        let redactor = gespan_redact::Redactor::new(gespan_redact::Config {
            home: Some("/Users/alice".into()),
            ..Default::default()
        });
        let typed = Report {
            title: "fails in /Users/alice/code".into(),
            description: "key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789 from bob@x.io"
                .into(),
            ..Report::default()
        };
        let sources = Sources {
            consent: default_consent(),
            ..Sources::default()
        };
        let bundle = build(
            &typed,
            &system(),
            &sources,
            &Selection::default(),
            Destination::Fallback,
            &redactor,
            &stamp(),
            MAX_BUNDLE_BYTES,
        )
        .unwrap();
        let url = issue_url(&bundle.report());
        assert!(!url.contains("alice") && !url.contains("sk-ant") && !url.contains("bob"));
        let (_, pairs) = query(&url);
        let pairs: HashMap<_, _> = pairs.into_iter().collect();
        assert_eq!(pairs["title"], "fails in ~/code");
        assert_eq!(pairs["what-happened"], bundle.report()["description"]);
    }

    #[test]
    fn url_round_trips_special_characters() {
        let title = "a & b #3 = 50% + ünïcødé 漢字 / ?";
        let description = "line one\nline two\ttab \"quoted\"";
        let (_, pairs) = query(&issue_url(&report(title, description, None, "linux")));
        let pairs: HashMap<_, _> = pairs.into_iter().collect();
        assert_eq!(pairs["title"], title);
        assert_eq!(pairs["what-happened"], description);
    }

    #[test]
    fn long_text_is_cut_to_fit() {
        let long = "é".repeat(20_000);
        let url = issue_url(&report("title", &long, Some(&"s".repeat(20_000)), "macos"));
        assert!(url.len() <= MAX_URL_BYTES, "{}", url.len());
        let (_, pairs) = query(&url);
        let pairs: HashMap<_, _> = pairs.into_iter().collect();
        assert_eq!(pairs["title"], "title");
        assert!(pairs["what-happened"].ends_with(CUT_NOTE));
        assert!(pairs["what-happened"].starts_with("éé"));
        assert_eq!(
            pairs["repro"], CUT_NOTE,
            "the steps go before what happened"
        );
        // A short report is left alone.
        let (_, short) = query(&issue_url(&report("title", "short", None, "macos")));
        assert!(short
            .iter()
            .any(|(k, v)| k == "what-happened" && v == "short"));
    }

    /// The decoded address, subject and body of a `mailto:` link.
    fn mail_parts(url: &str) -> (String, String, String) {
        let url = url::Url::parse(url).unwrap();
        assert_eq!(url.scheme(), "mailto");
        let to = percent_decode(url.path());
        let mut subject = None;
        let mut body = None;
        for pair in url.query().unwrap().split('&') {
            let (key, value) = pair.split_once('=').unwrap();
            match key {
                "subject" => subject = Some(percent_decode(value)),
                "body" => body = Some(percent_decode(value)),
                other => panic!("unexpected field {other}"),
            }
        }
        (to, subject.unwrap(), body.unwrap())
    }

    /// Decodes `%XX` only: a mail app reads a raw `+` as a plus, so a
    /// decoder that turned it into a space would hide encoding bugs.
    fn percent_decode(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                out.push(u8::from_str_radix(&text[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    fn mail_report(title: &str, description: &str, steps: Option<&str>) -> Value {
        let mut report = report(title, description, steps, "macos");
        report["build_channel"] = "release".into();
        report["arch"] = "aarch64".into();
        report["event_ids"] = json!(["0b7f1c2e-5a4d-4e8b-9f3a-2c6d1e0a9b47"]);
        report
    }

    const ZIP: &str = "gespan-diagnostics-native-0.1.0-20261025T091403Z.zip";

    #[test]
    fn mail_goes_to_the_tagged_address_with_the_report() {
        let url = mail_url(
            &mail_report("Worktree fails", "Nothing happens.", Some("1. Click")),
            ZIP,
        );
        assert!(
            url.starts_with("mailto:itparser%2Bgespan@gmail.com?"),
            "the + tag must be escaped so it isn't read as a space: {url}"
        );
        assert!(!url.contains('+') && !url.contains(' '), "{url}");
        let (to, subject, body) = mail_parts(&url);
        assert_eq!(to, REPORT_EMAIL);
        assert_eq!(subject, "[Gespan 0.1.0] Worktree fails");
        assert_eq!(
            body,
            "What happened:\r\nNothing happens.\r\n\r\n\
             Steps to reproduce:\r\n1. Click\r\n\r\n\
             Version: 0.1.0 (release)\r\nOS: macOS 15.1 (aarch64)\r\n\
             Event IDs: 0b7f1c2e-5a4d-4e8b-9f3a-2c6d1e0a9b47\r\n\r\n\
             Please attach gespan-diagnostics-native-0.1.0-20261025T091403Z.zip from your Downloads folder.\r\n"
        );
    }

    #[test]
    fn mail_leaves_out_empty_steps_and_event_ids() {
        let mut report = mail_report("t", "d", None);
        report["event_ids"] = json!([]);
        let (_, _, body) = mail_parts(&mail_url(&report, ZIP));
        assert!(!body.contains("Steps to reproduce"), "{body}");
        assert!(!body.contains("Event IDs"), "{body}");
        assert!(body.contains("OS: macOS 15.1 (aarch64)"));
    }

    #[test]
    fn mail_round_trips_special_characters() {
        let title = "a & b #3 = 50% + ünïcødé 漢字 / ?";
        let description = "line one\nline two\ttab \"quoted\" & body=x";
        let (_, subject, body) = mail_parts(&mail_url(&mail_report(title, description, None), ZIP));
        assert_eq!(subject, format!("[Gespan 0.1.0] {title}"));
        assert!(body.starts_with(&format!("What happened:\r\n{description}\r\n")));
    }

    #[test]
    fn long_mail_is_cut_but_keeps_the_version_and_the_zip_name() {
        let long = "é".repeat(5_000);
        let url = mail_url(
            &mail_report(&"t".repeat(3_000), &long, Some(&"s".repeat(5_000))),
            ZIP,
        );
        assert!(url.len() <= MAX_MAILTO_BYTES, "{}", url.len());
        let (_, subject, body) = mail_parts(&url);
        assert!(body.contains("Version: 0.1.0 (release)"), "{body}");
        assert!(body.contains(ZIP), "{body}");
        assert!(
            body.contains(&format!("Steps to reproduce:\r\n{CUT_NOTE}")),
            "the steps go first: {body}"
        );
        // Each cut ends on a character boundary, so the é's decode whole.
        assert!(subject.starts_with("[Gespan 0.1.0] t"));
        // A short report is left alone.
        let (_, _, short) = mail_parts(&mail_url(&mail_report("t", "short", None), ZIP));
        assert!(!short.contains(CUT_NOTE));
    }

    #[test]
    fn mail_uses_scrubbed_report_fields() {
        let redactor = gespan_redact::Redactor::new(gespan_redact::Config {
            home: Some("/Users/alice".into()),
            ..Default::default()
        });
        let typed = Report {
            title: "fails in /Users/alice/code".into(),
            description: "key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789 from bob@x.io"
                .into(),
            steps: "open /Users/alice/code/.env".into(),
            ..Report::default()
        };
        let sources = Sources {
            consent: default_consent(),
            ..Sources::default()
        };
        let bundle = build(
            &typed,
            &system(),
            &sources,
            &Selection::default(),
            Destination::Fallback,
            &redactor,
            &stamp(),
            MAX_BUNDLE_BYTES,
        )
        .unwrap();
        let (_, subject, body) = mail_parts(&mail_url(&bundle.report(), &bundle.file_name));
        for text in [&subject, &body] {
            assert!(
                !text.contains("alice") && !text.contains("sk-ant") && !text.contains("bob"),
                "{text}"
            );
        }
        assert!(subject.ends_with("fails in ~/code"), "{subject}");
        assert!(
            body.contains("Steps to reproduce:\r\nopen ~/code/.env"),
            "{body}"
        );
    }

    fn sample() -> crate::diagnostics::Bundle {
        build(
            &Report {
                title: "t".into(),
                description: "d".into(),
                ..Report::default()
            },
            &system(),
            &Sources {
                consent: default_consent(),
                ..Sources::default()
            },
            &Selection::default(),
            Destination::Fallback,
            &gespan_redact::Redactor::new(Default::default()),
            &stamp(),
            MAX_BUNDLE_BYTES,
        )
        .unwrap()
    }

    #[test]
    fn saved_bytes_are_the_bundle_zip() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = dir.path().join("Downloads");
        let bundle = sample();
        let path = save_bundle(&bundle, &downloads).unwrap();
        assert_eq!(path, downloads.join(&bundle.file_name));
        assert_eq!(std::fs::read(&path).unwrap(), bundle.zip());
    }

    #[test]
    fn save_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = sample();
        std::fs::write(dir.path().join(&bundle.file_name), b"mine").unwrap();
        let second = save_bundle(&bundle, dir.path()).unwrap();
        let third = save_bundle(&bundle, dir.path()).unwrap();
        assert_eq!(
            second.file_name().unwrap(),
            "gespan-diagnostics-native-0.1.0-20261025T091403Z-1.zip"
        );
        assert_eq!(
            third.file_name().unwrap(),
            "gespan-diagnostics-native-0.1.0-20261025T091403Z-2.zip"
        );
        assert_eq!(
            std::fs::read(dir.path().join(&bundle.file_name)).unwrap(),
            b"mine"
        );
    }

    #[test]
    fn remove_crashes_only_touches_named_files() {
        let dir = tempfile::tempdir().unwrap();
        let crashes = crate::diagnostics::crash_dir(dir.path());
        std::fs::create_dir_all(&crashes).unwrap();
        for name in ["a.json", "b.json"] {
            std::fs::write(crashes.join(name), "{}").unwrap();
        }
        std::fs::write(dir.path().join("keep.json"), "{}").unwrap();
        remove_crashes(
            dir.path(),
            &[
                "crashes/a.json".into(),
                "crashes/../keep.json".into(),
                "logs.jsonl".into(),
                "crashes/missing.json".into(),
            ],
        );
        assert!(!crashes.join("a.json").exists());
        assert!(crashes.join("b.json").exists());
        assert!(dir.path().join("keep.json").exists());
    }
}
