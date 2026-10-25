//! The fallback while there is no report intake: save the bundle to
//! Downloads and open GitHub's bug form with the report's fields filled in
//! (`docs/diagnostics/intake-api.md#fallback-when-the-intake-cant-be-reached`).
use super::Bundle;
use serde_json::Value;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const ISSUE_FORM: &str = "https://github.com/0xvasanth/gespan/issues/new";
/// GitHub refuses much longer URLs; the zip holds the full text anyway.
pub const MAX_URL_BYTES: usize = 8_000;
/// Ends a field that was cut to fit the URL.
pub const CUT_NOTE: &str = "… (cut; see the attached zip)";

/// The bug form, prefilled from the bundle's own (scrubbed) `report.json`,
/// so the issue says exactly what the preview showed.
pub fn issue_url(report: &Value) -> String {
    let text = |key: &str| report[key].as_str().unwrap_or_default().to_owned();
    let os = match report["os"].as_str().unwrap_or_default() {
        "macos" => "macOS".to_owned(),
        "linux" => "Linux".to_owned(),
        "windows" => "Windows".to_owned(),
        other => other.to_owned(),
    };
    let mut fields = vec![
        ("template", "bug_report.yml".to_owned()),
        ("title", text("title")),
        ("version", text("version")),
        ("os", os),
        ("os-version", text("os_version")),
        ("what-happened", text("description")),
    ];
    let steps = text("steps");
    if !steps.is_empty() {
        fields.push(("repro", steps));
    }
    // "What happened" is the form's required text: cut the steps first, the
    // title last.
    for key in ["repro", "what-happened", "title"] {
        if encode(&fields).len() <= MAX_URL_BYTES {
            break;
        }
        let Some(ix) = fields.iter().position(|(k, _)| *k == key) else {
            continue;
        };
        let full: Vec<char> = fields[ix].1.chars().collect();
        let fits = |n: usize, fields: &mut Vec<(&str, String)>| {
            fields[ix].1 = full[..n].iter().collect::<String>() + CUT_NOTE;
            encode(fields).len() <= MAX_URL_BYTES
        };
        // The longest prefix that fits, by binary search.
        let (mut low, mut high) = (0, full.len());
        while low < high {
            let mid = (low + high).div_ceil(2);
            if fits(mid, &mut fields) {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        fits(low, &mut fields);
    }
    encode(&fields)
}

fn encode(fields: &[(&str, String)]) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().map(|(k, v)| (*k, v.as_str())))
        .finish();
    format!("{ISSUE_FORM}?{query}")
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
        assert_eq!(pairs["repro"], CUT_NOTE, "the steps go before what happened");
        // A short report is left alone.
        let (_, short) = query(&issue_url(&report("title", "short", None, "macos")));
        assert!(short
            .iter()
            .any(|(k, v)| k == "what-happened" && v == "short"));
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
