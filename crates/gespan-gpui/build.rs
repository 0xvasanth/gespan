use std::{env, fs, path::PathBuf};

mod dependency_guard;

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
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_icon(&manifest_dir);
    }
    dependency_guard::check(&manifest_dir);
}

/// GPUI loads the window and taskbar icon from resource ID 1 of the executable,
/// which Explorer and the installer shortcuts also show.
fn embed_windows_icon(manifest_dir: &std::path::Path) {
    let icon = manifest_dir.join("packaging/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    assert!(
        icon.is_file(),
        "The Windows build requires packaging/icon.ico"
    );
    // Not canonicalize: its \\?\ verbatim prefix is rejected by rc.exe.
    let icon = icon.to_string_lossy().replace('\\', "/");
    let rc = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("gespan.rc");
    fs::write(&rc, format!("1 ICON \"{icon}\"\n")).unwrap();
    embed_resource::compile(&rc, embed_resource::NONE)
        .manifest_optional()
        .unwrap();
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
