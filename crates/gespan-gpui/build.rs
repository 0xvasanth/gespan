use std::{env, fs, path::PathBuf};

fn main() {
    for name in [
        "GESPAN_NATIVE_VERSION",
        "GESPAN_NATIVE_UPDATER_ENDPOINT",
        "GESPAN_NATIVE_UPDATER_PUBKEY",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let version = env::var("GESPAN_NATIVE_VERSION")
        .unwrap_or_else(|_| env::var("CARGO_PKG_VERSION").unwrap());
    println!("cargo:rustc-env=GESPAN_NATIVE_VERSION={version}");
    println!("cargo:rustc-env=GESPAN_BUILD_COMMIT={}", build_commit());
    let lockfile = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("Cargo.lock");
    println!("cargo:rerun-if-changed={}", lockfile.display());
    let lock = fs::read_to_string(&lockfile).expect("The native workspace requires Cargo.lock");
    // Cargo writes canonical TOML. Check package names rather than license metadata:
    // zed-scap advertises MIT but its published license has additional restrictions.
    assert!(
        !lock
            .lines()
            .any(|line| line.trim() == "name = \"zed-scap\""),
        "zed-scap is prohibited in the native dependency graph, including optional dependencies. \
         Restore the GPUI patches that remove it, or replace it with a dependency whose \
         license has been reviewed. Do not suppress this check based on its MIT metadata."
    );
}

/// The commit this build came from, for the diagnostics log: a short SHA,
/// `-dirty` with uncommitted changes, or `GESPAN_BUILD_COMMIT` when CI sets it.
/// Empty outside a Git checkout.
fn build_commit() -> String {
    println!("cargo:rerun-if-env-changed=GESPAN_BUILD_COMMIT");
    if let Ok(commit) = env::var("GESPAN_BUILD_COMMIT") {
        return commit;
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    // HEAD's reflog changes on every commit and checkout, worktrees included.
    for path in ["HEAD", "logs/HEAD", "index"] {
        if let Some(file) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    let Some(sha) = git(&["rev-parse", "--short=12", "HEAD"]) else {
        return String::new();
    };
    // Without optional locks, status leaves the index alone, so it cannot
    // retrigger this script through the rerun check above.
    let dirty = git(&[
        "--no-optional-locks",
        "status",
        "--porcelain",
        "--untracked-files=no",
    ])
    .is_some_and(|status| !status.is_empty());
    if dirty { format!("{sha}-dirty") } else { sha }
}
