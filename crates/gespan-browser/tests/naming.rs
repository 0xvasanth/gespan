//! The crate was imported from an upstream fork. Nothing of that product's
//! name may remain in what ships or in what agents read.

use std::fs;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn no_upstream_names_remain() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);
    files.push(root.join("instructions.md"));
    files.push(root.join("Cargo.toml"));
    assert!(
        files.len() > 30,
        "the walk found only {} files",
        files.len()
    );

    let mut hits = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).unwrap();
        if text.to_lowercase().contains("monocode") || text.contains("Mono ") {
            hits.push(path.strip_prefix(root).unwrap().display().to_string());
        }
    }
    assert_eq!(hits, Vec::<String>::new());
}
