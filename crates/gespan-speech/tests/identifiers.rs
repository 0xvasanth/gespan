use std::fs;
use std::path::Path;
use std::process::Command;

use gespan_speech::{local_branches, pick_terms, postprocess, VocabularySources, MAX_TERMS};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    terms: Vec<String>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    input: String,
    expected: String,
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

#[test]
fn identifier_phrases_fixture() {
    let raw = include_str!("fixtures/identifier_phrases.json");
    let fixture: Fixture = serde_json::from_str(raw).unwrap();
    assert!(fixture.cases.len() >= 25);
    for case in &fixture.cases {
        assert_eq!(
            postprocess(&case.input, &fixture.terms),
            case.expected,
            "input: {:?}",
            case.input
        );
    }
}

#[test]
fn pick_terms_filters_short_and_dedupes() {
    let user = strings(&["Tauri", "abc", "2024", "tauri"]);
    let branches = strings(&["fix/Voice-Input", "main"]);
    let known = strings(&["TAURI", "voice-input", "gpt"]);
    let terms = pick_terms(&VocabularySources {
        project_root: None,
        branches: &branches,
        known_names: &known,
        user_words: &user,
    });
    assert_eq!(terms, strings(&["Tauri", "Voice-Input", "main"]));
}

#[test]
fn pick_terms_caps_at_230_keeping_user_words() {
    let user = strings(&["Zebracorn", "Quokkaflow"]);
    let known: Vec<String> = (0..400).map(|i| format!("model{i}")).collect();
    let terms = pick_terms(&VocabularySources {
        project_root: None,
        branches: &[],
        known_names: &known,
        user_words: &user,
    });
    assert_eq!(terms.len(), MAX_TERMS);
    assert_eq!(&terms[..2], &user[..]);
    assert_eq!(terms[2], "model0");
}

#[test]
fn pick_terms_reads_top_folders_and_git_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("workspace/src")).unwrap();
    fs::create_dir_all(root.join(".hidden")).unwrap();
    fs::write(root.join("workspace/src/useInboxUnseen.ts"), "x").unwrap();
    fs::write(root.join("workspace/Cargo.toml"), "x").unwrap();
    fs::write(root.join("untracked_notes.txt"), "x").unwrap();
    git(root, &["init", "-q", "-b", "trunk"]);
    git(root, &["add", "workspace"]);
    git(root, &["commit", "-q", "-m", "init"]);

    let branches = local_branches(root);
    assert_eq!(branches, strings(&["trunk"]));

    let terms = pick_terms(&VocabularySources {
        project_root: Some(root),
        branches: &branches,
        known_names: &[],
        user_words: &[],
    });
    assert_eq!(
        terms,
        strings(&["workspace", "Cargo", "useInboxUnseen", "trunk"])
    );
}

#[test]
fn non_repo_contributes_no_file_terms() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("plain_folder")).unwrap();
    assert!(local_branches(dir.path()).is_empty());
    let terms = pick_terms(&VocabularySources {
        project_root: Some(dir.path()),
        branches: &[],
        known_names: &[],
        user_words: &[],
    });
    assert_eq!(terms, strings(&["plain_folder"]));
}
