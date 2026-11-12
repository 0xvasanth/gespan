//! Importing the preferences a user already keeps in harness files.
use crate::decisions::files::{replace_section, BEGIN, END};
use crate::decisions::import::{candidates, global_files, import_once};
use crate::decisions::{Config, DecisionStore, Kind, Scope, Status};
use crate::provider::Kind as Harness;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CLAUDE_MD: &str = "# Personal Rules\n\n\
## 1. Commits\n\n\
- Never add a Co-Authored-By trailer.\n\
- Keep messages short:\n  - imperative mood\n  - no trailing period\n\n\
## 2. Testing\n\n\
1. Add a test with every fix.\n\
2. Run the suite before committing.\n\n\
## 3. Style\n\n\
Some prose that is not a rule.\n\n\
* Prefer small functions.\n";

fn store(dir: &TempDir) -> DecisionStore {
    DecisionStore::open(
        &dir.path().join("gespan.db"),
        Config {
            redact: crate::pilot::memory::redact,
        },
    )
    .unwrap()
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn splits_bullets_under_headings_into_preferences() {
    let file = Path::new("CLAUDE.md");
    let found = candidates(CLAUDE_MD, file);
    let texts: Vec<&str> = found.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Never add a Co-Authored-By trailer.",
            "Keep messages short:\n- imperative mood\n- no trailing period",
            "Add a test with every fix.",
            "Run the suite before committing.",
            "Prefer small functions.",
        ]
    );
    assert!(found
        .iter()
        .all(|e| e.kind == Kind::Preference && e.scope == Scope::Global));
    assert!(found.iter().all(|e| e.why == "Imported from CLAUDE.md"));
    assert_eq!(
        found[0].title,
        "1. Commits: Never add a Co-Authored-By trailer."
    );
    assert_eq!(found[4].title, "3. Style: Prefer small functions.");
}

#[test]
fn bullets_inside_code_fences_are_not_rules() {
    let text = "# Rules\n\n- Real rule\n\n```\n- not a rule\n```\n";
    let found = candidates(text, Path::new("x.md"));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].text, "Real rule\n```\n- not a rule\n```");
}

#[test]
fn ignores_gespan_section() {
    let text = replace_section(CLAUDE_MD, "- Gespan wrote this");
    assert!(text.contains(BEGIN) && text.contains(END));
    let found = candidates(&text, Path::new("CLAUDE.md"));
    assert_eq!(found.len(), 5);
    assert!(found.iter().all(|e| !e.text.contains("Gespan wrote")));
}

#[test]
fn imported_entries_are_pending_with_import_source() {
    let dir = TempDir::new().unwrap();
    let log = store(&dir);
    let file = write(dir.path(), "CLAUDE.md", CLAUDE_MD);
    assert_eq!(
        import_once(&log, std::slice::from_ref(&file), 10).unwrap(),
        5
    );
    let pending = log.pending().unwrap();
    assert_eq!(pending.len(), 5);
    for entry in &pending {
        assert_eq!(entry.status, Status::Pending);
        assert_eq!(entry.kind, Kind::Preference);
        assert_eq!(entry.source.import.as_deref(), file.to_str());
        assert_eq!(entry.why, format!("Imported from {}", file.display()));
    }
    // No user source, so other sessions do not see them as the user's words.
    assert!(log.active(&Scope::Global).unwrap().is_empty());
}

#[test]
fn import_once_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let log = store(&dir);
    let file = write(dir.path(), "CLAUDE.md", CLAUDE_MD);
    assert_eq!(
        import_once(&log, std::slice::from_ref(&file), 10).unwrap(),
        5
    );
    assert_eq!(
        import_once(&log, std::slice::from_ref(&file), 20).unwrap(),
        0
    );
    assert_eq!(log.pending().unwrap().len(), 5);
}

#[test]
fn rejected_and_archived_imports_do_not_come_back() {
    let dir = TempDir::new().unwrap();
    let log = store(&dir);
    let file = write(dir.path(), "CLAUDE.md", CLAUDE_MD);
    import_once(&log, std::slice::from_ref(&file), 10).unwrap();
    let pending = log.pending().unwrap();
    log.reject(pending[0].id, 15).unwrap();
    let approved = log.approve(pending[1].id, None, false, &[], 15).unwrap();
    log.archive(approved.id, 16).unwrap();
    assert_eq!(import_once(&log, &[file], 20).unwrap(), 0);
    assert_eq!(log.pending().unwrap().len(), 3);
}

#[test]
fn a_rule_added_later_is_imported_but_old_ones_are_not() {
    let dir = TempDir::new().unwrap();
    let log = store(&dir);
    let file = write(dir.path(), "CLAUDE.md", CLAUDE_MD);
    import_once(&log, std::slice::from_ref(&file), 10).unwrap();
    write(
        dir.path(),
        "CLAUDE.md",
        &format!("{CLAUDE_MD}- Brand new rule\n"),
    );
    assert_eq!(import_once(&log, &[file], 20).unwrap(), 1);
}

#[test]
fn secrets_in_a_rule_are_stored_redacted() {
    let dir = TempDir::new().unwrap();
    let log = store(&dir);
    let secret = "sk-abcdefghijklmnopqrstuvwx";
    let text = format!("# Keys\n\n- Call the API with {secret} always\n");
    let file = write(dir.path(), "CLAUDE.md", &text);
    assert_eq!(
        import_once(&log, std::slice::from_ref(&file), 10).unwrap(),
        1
    );
    let entry = &log.pending().unwrap()[0];
    for field in [&entry.title, &entry.text] {
        assert!(!field.contains(secret), "{field}");
        assert!(field.contains("redacted"), "{field}");
    }
    // The redacted text is what is matched, so it is not imported twice.
    assert_eq!(import_once(&log, &[file], 20).unwrap(), 0);
}

#[test]
fn unreadable_files_are_skipped() {
    let dir = TempDir::new().unwrap();
    let log = store(&dir);
    let big = write(
        dir.path(),
        "big.md",
        &format!("- rule\n{}", "x".repeat(300 * 1024)),
    );
    let folder = dir.path().join("AGENTS.md");
    std::fs::create_dir(&folder).unwrap();
    let missing = dir.path().join("nope.md");
    assert_eq!(import_once(&log, &[big, folder, missing], 10).unwrap(), 0);
    assert!(log.pending().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn a_symlink_out_of_its_folder_is_not_read() {
    let dir = TempDir::new().unwrap();
    let log = store(&dir);
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let outside = write(dir.path(), "elsewhere.md", "- secret plans\n");
    std::os::unix::fs::symlink(&outside, home.join("CLAUDE.md")).unwrap();
    let inside = write(&home, "real.md", "- inside rule\n");
    std::os::unix::fs::symlink(&inside, home.join("AGENTS.md")).unwrap();
    let files = [home.join("CLAUDE.md"), home.join("AGENTS.md")];
    assert_eq!(import_once(&log, &files, 10).unwrap(), 1);
    assert_eq!(log.pending().unwrap()[0].text, "inside rule");
}

#[test]
fn global_files_follow_the_injected_homes() {
    let dir = TempDir::new().unwrap();
    let claude = dir.path().join("claude");
    let codex = dir.path().join("codex");
    let opencode = dir.path().join("opencode");
    for d in [&claude, &codex, &opencode] {
        std::fs::create_dir(d).unwrap();
    }
    write(&claude, "CLAUDE.md", "- a\n");
    write(&codex, "AGENTS.md", "- b\n");
    // OpenCode has no AGENTS.md yet, so it is not listed.
    let homes = |kind: Harness| match kind {
        Harness::Claude => Some(claude.clone()),
        Harness::Codex => Some(codex.clone()),
        Harness::OpenCode => Some(opencode.clone()),
        _ => None,
    };
    assert_eq!(
        global_files(&homes),
        [claude.join("CLAUDE.md"), codex.join("AGENTS.md")]
    );
    write(&opencode, "AGENTS.md", "- c\n");
    assert_eq!(global_files(&homes).len(), 3);
}

#[test]
fn concurrent_imports_insert_each_rule_once() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("gespan.db");
    let config = || Config {
        redact: crate::pilot::memory::redact,
    };
    let first = DecisionStore::open(&path, config()).unwrap();
    let second = DecisionStore::open(&path, config()).unwrap();
    let file = write(dir.path(), "CLAUDE.md", CLAUDE_MD);
    let files = [file];
    let barrier = std::sync::Barrier::new(2);
    let (a, b) = std::thread::scope(|s| {
        let (barrier, files) = (&barrier, &files);
        let run = move |log: &DecisionStore| {
            barrier.wait();
            import_once(log, files, 10).unwrap()
        };
        let (one, two) = (&first, &second);
        let a = s.spawn(move || run(one));
        let b = s.spawn(move || run(two));
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!(a + b, 5);
    assert_eq!(first.pending().unwrap().len(), 5);
}

#[test]
fn a_file_over_the_cap_imports_only_its_first_rules() {
    use crate::decisions::import::MAX_RULES_PER_FILE;
    let dir = TempDir::new().unwrap();
    let log = store(&dir);
    let text: String = (0..250).map(|i| format!("- rule number {i}\n")).collect();
    let file = write(dir.path(), "CLAUDE.md", &text);
    assert_eq!(MAX_RULES_PER_FILE, 200);
    assert_eq!(import_once(&log, &[file], 10).unwrap(), 200);
    let texts: Vec<String> = log.pending().unwrap().into_iter().map(|e| e.text).collect();
    assert!(texts.contains(&"rule number 199".to_string()));
    assert!(!texts.contains(&"rule number 200".to_string()));
}

#[test]
fn stored_fields_name_the_home_as_a_tilde_not_its_path() {
    use crate::decisions::import::import_once_from;
    let dir = TempDir::new().unwrap();
    let log = store(&dir);
    let home = dir.path().join("people").join("jane");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    let file = write(&home.join(".claude"), "CLAUDE.md", "- Keep it short\n");
    assert_eq!(
        import_once_from(&log, std::slice::from_ref(&file), 10, Some(&home)).unwrap(),
        1
    );
    // Stable: a second run with the same home matches the stored source.
    assert_eq!(import_once_from(&log, &[file], 20, Some(&home)).unwrap(), 0);
    let entry = &log.pending().unwrap()[0];
    let home_text = home.display().to_string();
    let source = entry.source.import.clone().unwrap();
    for field in [&entry.title, &entry.text, &entry.why, &source] {
        assert!(!field.contains(&home_text), "{field}");
    }
    assert!(source.starts_with('~'), "{source}");
    assert!(entry.why.starts_with("Imported from ~"), "{}", entry.why);
}

#[test]
fn indentation_decides_between_a_rule_and_a_nested_item() {
    let text = "# Rules\n\n - one space\n - sibling\n   - nested under it\n    - also nested\n\n## More\n\n   - three spaces\n";
    let found = candidates(text, Path::new("x.md"));
    let texts: Vec<&str> = found.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "one space",
            "sibling\n- nested under it\n- also nested",
            "three spaces"
        ]
    );
}

#[test]
fn horizontal_rules_and_year_lines_are_not_rules() {
    let text = "# Rules\n\n- Real\n\n* * *\n\n- - -\n\n- Also real\n\n2024. A year\n";
    let found = candidates(text, Path::new("x.md"));
    let texts: Vec<&str> = found.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texts, ["Real", "Also real"]);
}
