//! Brings a new worktree the ignored files its project lists in
//! `.worktreeinclude`, sharing disk blocks where the filesystem can. The copy
//! rules are ported from worktrunk (see NOTICE).
//!
//! A file has to match `.worktreeinclude` and be ignored by Git, so a tracked
//! file is never copied over the checkout's version.
use crate::git::git;
use ignore::gitignore::GitignoreBuilder;
use rayon::prelude::*;
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        LazyLock,
    },
};

const INCLUDE_FILE: &str = ".worktreeinclude";

/// Four threads: this runs beside the app, and it mostly waits on the disk.
static COPY_POOL: LazyLock<Result<rayon::ThreadPool, rayon::ThreadPoolBuildError>> =
    LazyLock::new(|| rayon::ThreadPoolBuilder::new().num_threads(4).build());

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Copied {
    pub files: usize,
    pub bytes: u64,
    /// Of `files`, how many share their blocks with the source.
    pub shared: usize,
    /// Symlinks that lead out of the checkout, and files that vanished.
    pub skipped: usize,
}

/// Git lists entries with `/`, which a canonical Windows path (`\\?\...`) takes
/// literally. So files are reached through the paths as given, and the real
/// paths are only for checking where something leads.
struct Roots<'a> {
    main: &'a Path,
    worktree: &'a Path,
    real_main: PathBuf,
    real_worktree: PathBuf,
}

#[derive(Default)]
struct Tally {
    files: AtomicUsize,
    bytes: AtomicU64,
    shared: AtomicUsize,
    skipped: AtomicUsize,
}

/// Copies `main`'s listed ignored files into the fresh `worktree`. Does nothing
/// for a project without `.worktreeinclude`.
pub fn copy_included(main: &Path, worktree: &Path) -> Result<Copied, String> {
    let include = main.join(INCLUDE_FILE);
    if !include.is_file() {
        return Ok(Copied::default());
    }
    let mut builder = GitignoreBuilder::new(main);
    if let Some(error) = builder.add(&include) {
        return Err(format!("Could not read {INCLUDE_FILE}: {error}"));
    }
    let matcher = builder.build().map_err(|error| error.to_string())?;
    let listed = git(
        main,
        &[
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
        ],
    )?;
    let entries: Vec<&str> = listed
        .split('\0')
        // Git names a nested repository with a trailing slash; it isn't a file.
        .filter(|entry| !entry.is_empty() && !entry.ends_with('/'))
        .filter(|entry| !is_in_git_folder(Path::new(entry)))
        .filter(|entry| {
            matcher
                .matched_path_or_any_parents(entry, false)
                .is_ignore()
        })
        .collect();
    let roots = Roots {
        main,
        worktree,
        real_main: main.canonicalize().map_err(|error| error.to_string())?,
        real_worktree: worktree.canonicalize().map_err(|error| error.to_string())?,
    };
    let tally = Tally::default();
    let copy = || {
        entries.par_iter().try_for_each(|entry| {
            copy_entry(&roots, Path::new(entry), &tally)
                .map_err(|error| format!("Could not copy {entry}: {error}"))
        })
    };
    match &*COPY_POOL {
        Ok(pool) => pool.install(copy)?,
        Err(_) => copy()?,
    }
    Ok(Copied {
        files: tally.files.into_inner(),
        bytes: tally.bytes.into_inner(),
        shared: tally.shared.into_inner(),
        skipped: tally.skipped.into_inner(),
    })
}

fn is_in_git_folder(path: &Path) -> bool {
    path.components()
        .any(|part| matches!(part, Component::Normal(name) if name == ".git"))
}

fn copy_entry(roots: &Roots, entry: &Path, tally: &Tally) -> io::Result<()> {
    let source = roots.main.join(entry);
    let dest = roots.worktree.join(entry);
    let metadata = match fs::symlink_metadata(&source) {
        Ok(metadata) => metadata,
        // A running build can delete files behind Git's listing.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            tally.skipped.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    if dest.symlink_metadata().is_ok() {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
        // A tracked symlink in the worktree must not carry the copy elsewhere.
        if !parent.canonicalize()?.starts_with(&roots.real_worktree) {
            tally.skipped.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
    }
    if metadata.is_symlink() {
        let copied = copy_symlink(&roots.real_main, &source, &dest)?;
        let counter = if copied { &tally.files } else { &tally.skipped };
        counter.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }
    let wrote = match reflink_copy::reflink_or_copy(&source, &dest) {
        Ok(wrote) => wrote,
        Err(error) if error.kind() == io::ErrorKind::NotFound && !source.exists() => {
            tally.skipped.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    keep_modified_time(&dest, &metadata);
    if wrote.is_none() {
        tally.shared.fetch_add(1, Ordering::Relaxed);
    }
    tally.files.fetch_add(1, Ordering::Relaxed);
    tally.bytes.fetch_add(metadata.len(), Ordering::Relaxed);
    Ok(())
}

/// Clones and byte copies can both write the time of the copy, and files copied
/// in parallel would then be ordered by scheduling. Build tools compare these.
/// Best effort: a read-only copy can't be opened to set it.
fn keep_modified_time(dest: &Path, source: &fs::Metadata) {
    let Ok(modified) = source.modified() else {
        return;
    };
    if let Ok(file) = fs::File::options().write(true).open(dest) {
        let _ = file.set_modified(modified);
    }
}

/// Keeps a relative link whose target stays inside the checkout, so it means the
/// same thing in the worktree. Anything else would reach outside it.
fn copy_symlink(main: &Path, source: &Path, dest: &Path) -> io::Result<bool> {
    let target = fs::read_link(source)?;
    if target.is_absolute() {
        return Ok(false);
    }
    let Some(parent) = source.parent() else {
        return Ok(false);
    };
    if !stays_inside(main, &parent.join(&target)) {
        return Ok(false);
    }
    make_symlink(source, &target, dest)
}

/// Whether `path` resolves under `root`, without needing it to exist. Each real
/// part is resolved, so a link in the middle can't hide an escape.
fn stays_inside(root: &Path, path: &Path) -> bool {
    let mut resolved = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            part => {
                resolved.push(part);
                if let Ok(real) = resolved.canonicalize() {
                    resolved = real;
                }
            }
        }
    }
    resolved.starts_with(root)
}

#[cfg(unix)]
fn make_symlink(_: &Path, target: &Path, dest: &Path) -> io::Result<bool> {
    std::os::unix::fs::symlink(target, dest).map(|_| true)
}
#[cfg(windows)]
fn make_symlink(source: &Path, target: &Path, dest: &Path) -> io::Result<bool> {
    use std::os::windows::fs::{symlink_dir, symlink_file};
    let linked = if source.is_dir() {
        symlink_dir(target, dest)
    } else {
        symlink_file(target, dest)
    };
    // Creating links needs a privilege most Windows accounts don't have.
    match linked {
        Err(error) if error.raw_os_error() == Some(1314) => Ok(false),
        linked => linked.map(|_| true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) -> String {
        git(dir, args).unwrap_or_else(|error| panic!("git {args:?}: {error}"))
    }
    fn write(root: &Path, file: &str, text: &str) {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    /// A repository with a linked worktree, and `ignored` in `.gitignore`.
    fn repo(ignored: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        fs::create_dir(&main).unwrap();
        run(&main, &["init", "-qb", "main"]);
        run(&main, &["config", "user.name", "Test"]);
        run(&main, &["config", "user.email", "test@example.invalid"]);
        run(&main, &["config", "commit.gpgsign", "false"]);
        write(&main, ".gitignore", ignored);
        write(&main, "tracked.txt", "from git\n");
        run(&main, &["add", "."]);
        run(&main, &["commit", "-qm", "Start"]);
        let tree = dir.path().join("tree");
        run(
            &main,
            &["worktree", "add", "-q", "-b", "w", tree.to_str().unwrap()],
        );
        (dir, main, tree)
    }

    #[test]
    fn nothing_happens_without_an_include_file() {
        let (_dir, main, tree) = repo(".env\n");
        write(&main, ".env", "secret");
        assert_eq!(copy_included(&main, &tree).unwrap(), Copied::default());
        assert!(!tree.join(".env").exists());
    }
    #[test]
    fn a_file_must_be_listed_and_ignored() {
        let (_dir, main, tree) = repo(".env\nother.log\n");
        write(
            &main,
            ".worktreeinclude",
            ".env\ntracked.txt\nuntracked.txt\n",
        );
        write(&main, ".env", "secret");
        write(&main, "other.log", "not listed");
        write(&main, "untracked.txt", "not ignored");
        write(&main, "tracked.txt", "edited in main\n");
        let copied = copy_included(&main, &tree).unwrap();
        assert_eq!(copied.files, 1);
        assert_eq!(fs::read_to_string(tree.join(".env")).unwrap(), "secret");
        assert!(!tree.join("other.log").exists());
        assert!(!tree.join("untracked.txt").exists());
        assert_eq!(
            fs::read_to_string(tree.join("tracked.txt")).unwrap(),
            "from git\n"
        );
    }
    #[test]
    fn patterns_cover_comments_negation_and_folders() {
        let (_dir, main, tree) = repo("config/\ncache/\n*.key\n");
        write(
            &main,
            ".worktreeinclude",
            "# a comment\n\nconfig/\n*.key\n!private.key\n",
        );
        write(&main, "config/nested/app.toml", "a");
        write(&main, "cache/skip.bin", "b");
        write(&main, "public.key", "c");
        write(&main, "private.key", "d");
        write(&main, "# a comment", "e");
        copy_included(&main, &tree).unwrap();
        assert!(tree.join("config/nested/app.toml").exists());
        assert!(tree.join("public.key").exists());
        assert!(!tree.join("cache/skip.bin").exists());
        assert!(!tree.join("private.key").exists());
    }
    #[test]
    fn names_with_spaces_and_non_ascii_characters_copy() {
        let (_dir, main, tree) = repo("local/\n");
        write(&main, ".worktreeinclude", "local/\n");
        write(&main, "local/my notes é中.txt", "text");
        assert_eq!(copy_included(&main, &tree).unwrap().files, 1);
        assert!(tree.join("local/my notes é中.txt").exists());
    }
    #[test]
    fn an_existing_file_is_not_replaced() {
        let (_dir, main, tree) = repo(".env\n");
        write(&main, ".worktreeinclude", ".env\n");
        write(&main, ".env", "from main");
        write(&tree, ".env", "already here");
        assert_eq!(copy_included(&main, &tree).unwrap().files, 0);
        assert_eq!(
            fs::read_to_string(tree.join(".env")).unwrap(),
            "already here"
        );
    }
    #[test]
    fn a_nested_repository_is_not_copied() {
        let (_dir, main, tree) = repo("vendored/\n");
        write(&main, ".worktreeinclude", "vendored/\n");
        let nested = main.join("vendored");
        fs::create_dir(&nested).unwrap();
        run(&nested, &["init", "-qb", "main"]);
        write(&nested, "file.txt", "x");
        copy_included(&main, &tree).unwrap();
        assert!(!tree.join("vendored/.git").exists());
    }
    #[test]
    fn modified_times_carry_over() {
        let (_dir, main, tree) = repo("out.bin\n");
        write(&main, ".worktreeinclude", "out.bin\n");
        write(&main, "out.bin", "built");
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let file = fs::File::options()
            .write(true)
            .open(main.join("out.bin"))
            .unwrap();
        file.set_modified(old).unwrap();
        drop(file);
        copy_included(&main, &tree).unwrap();
        let copied = fs::metadata(tree.join("out.bin")).unwrap();
        assert_eq!(copied.modified().unwrap(), old);
    }
    #[test]
    fn writing_a_copy_leaves_the_source_alone() {
        let (_dir, main, tree) = repo("big.bin\n");
        write(&main, ".worktreeinclude", "big.bin\n");
        write(&main, "big.bin", &"x".repeat(1 << 20));
        let copied = copy_included(&main, &tree).unwrap();
        assert_eq!((copied.files, copied.bytes), (1, 1 << 20));
        // CI's disks cover both paths: macOS clones, Linux and Windows write.
        #[cfg(target_os = "macos")]
        assert_eq!(copied.shared, 1);
        fs::write(tree.join("big.bin"), "changed").unwrap();
        assert_eq!(fs::metadata(main.join("big.bin")).unwrap().len(), 1 << 20);
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_stay_inside_the_checkout() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (dir, main, tree) = repo("local/\n");
        write(&main, ".worktreeinclude", "local/\n");
        write(dir.path(), "outside/secret.txt", "secret");
        write(&main, "local/real.txt", "real");
        fs::set_permissions(
            main.join("local/real.txt"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        symlink("real.txt", main.join("local/inside")).unwrap();
        symlink("../../outside/secret.txt", main.join("local/escape")).unwrap();
        symlink(
            dir.path().join("outside/secret.txt"),
            main.join("local/absolute"),
        )
        .unwrap();
        symlink("missing", main.join("local/broken")).unwrap();
        let copied = copy_included(&main, &tree).unwrap();
        assert_eq!(copied.skipped, 2, "{copied:?}");
        assert_eq!(
            fs::read_link(tree.join("local/inside")).unwrap(),
            Path::new("real.txt")
        );
        assert!(tree.join("local/broken").symlink_metadata().is_ok());
        assert!(tree.join("local/escape").symlink_metadata().is_err());
        assert!(tree.join("local/absolute").symlink_metadata().is_err());
        let mode = fs::metadata(tree.join("local/real.txt")).unwrap();
        assert_eq!(mode.permissions().mode() & 0o777, 0o755);
    }
    #[cfg(unix)]
    #[test]
    fn a_link_in_the_worktree_cannot_carry_a_copy_out() {
        use std::os::unix::fs::symlink;
        let (dir, main, tree) = repo("local/\n");
        write(&main, ".worktreeinclude", "local/\n");
        write(&main, "local/file.txt", "x");
        fs::create_dir(dir.path().join("outside")).unwrap();
        symlink(dir.path().join("outside"), tree.join("local")).unwrap();
        assert_eq!(copy_included(&main, &tree).unwrap().skipped, 1);
        assert!(!dir.path().join("outside/file.txt").exists());
    }
    #[test]
    fn a_malformed_pattern_is_reported_and_copies_nothing() {
        let (_dir, main, tree) = repo(".env\n");
        write(&main, ".worktreeinclude", "[z-a]\n.env\n");
        write(&main, ".env", "x");
        let error = copy_included(&main, &tree).unwrap_err();
        assert!(error.contains(".worktreeinclude"), "{error}");
        assert!(!tree.join(".env").exists());
    }
}
