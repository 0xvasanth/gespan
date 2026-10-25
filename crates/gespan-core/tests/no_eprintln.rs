//! Errors go through `tracing`, so they reach the log file in a packaged app.
//! Plain `eprintln!` output is lost when the app is launched from Finder.
use std::path::Path;

/// `path:line` for every line that prints to stderr outside a comment.
fn offenders(path: &str, source: &str) -> Vec<String> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let code = line.split("//").next().unwrap_or_default();
            code.contains("eprintln!") || code.contains("eprint!")
        })
        .map(|(index, _)| format!("{path}:{}", index + 1))
        .collect()
}

fn scan(root: &Path, dir: &Path, found: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(root, &path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            found.extend(offenders(&name, &source));
        }
    }
}

#[test]
fn offenders_ignore_comments_only() {
    let source = "// as eprintln! did\n    /// eprint!\nlet x = 1; eprintln!(\"a\");\neprint!(\"b\") // note\n";
    assert_eq!(offenders("f.rs", source), ["f.rs:3", "f.rs:4"]);
}

#[test]
fn gespan_core_has_no_eprintln() {
    let mut found = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    scan(root, &root.join("src"), &mut found);
    assert!(
        found.is_empty(),
        "use tracing::error!/warn!/debug! instead of eprintln! at:\n{}",
        found.join("\n")
    );
}
