//! The shared fixtures in `fixtures/redaction.json`: every case must produce
//! its expected output exactly, and scrubbing that output again changes nothing.

use gespan_redact::fixtures;

#[test]
fn every_fixture_matches() {
    let file = fixtures::load();
    let mut failures = Vec::new();
    for case in &file.cases {
        let actual = case.run(&file.redactor(&case.context));
        if actual != case.expected {
            failures.push(format!(
                "{}\n  expected: {}\n  actual:   {}",
                case.name, case.expected, actual
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(
        file.cases.len() >= 25,
        "only {} fixture cases",
        file.cases.len()
    );
    for context in ["macos", "linux", "windows"] {
        assert!(
            file.cases.iter().any(|case| case.context == context),
            "no {context} case"
        );
    }
}

#[test]
fn scrubbing_is_idempotent() {
    let file = fixtures::load();
    for case in &file.cases {
        let again = case.run_on(&file.redactor(&case.context), &case.expected);
        assert_eq!(
            again, case.expected,
            "{} changed on a second pass",
            case.name
        );
    }
}
