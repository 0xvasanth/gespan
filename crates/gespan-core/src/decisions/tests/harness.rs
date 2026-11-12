use crate::accounts::{Account, Book};
use crate::decisions::files::{BEGIN, END};
use crate::decisions::harness::{
    hold_writing, preference_targets, project_targets, regenerate, remove_all, Mode, Report,
    BEFORE_WRITE, MAX_FILE_BYTES,
};
use crate::decisions::scope::{git_common_dir, scope_for, shared_file};
use crate::decisions::service::{self, isolated, Isolated};
use crate::provider::Kind;
use gespan_memory::{Config, Kind as EntryKind, NewEntry, Scope, Source, Store, START_BUDGET};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// A temp home, data dir and git repository, and a store of their own.
struct Fixture {
    store: Store,
    _log: Isolated,
    _tmp: TempDir,
    home: PathBuf,
    data: PathBuf,
    repo: PathBuf,
}

fn fixture() -> Fixture {
    // Serializes with every test of the process-wide switch, and turns it on.
    let log = isolated();
    let tmp = TempDir::new().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (home, data, repo) = (root.join("home"), root.join("data"), root.join("repo"));
    for dir in [&home, &data, &repo] {
        fs::create_dir_all(dir).unwrap();
    }
    init_repo(&repo);
    let store = Store::open(
        &root.join("memory.db"),
        Config {
            redact: crate::pilot::memory::redact,
        },
    )
    .unwrap();
    Fixture {
        store,
        _log: log,
        _tmp: tmp,
        home,
        data,
        repo,
    }
}

impl Fixture {
    /// Installs Claude Code and Codex (their default homes exist).
    fn install_claude_and_codex(&self) {
        fs::create_dir_all(self.home.join(".claude")).unwrap();
        fs::create_dir_all(self.home.join(".codex")).unwrap();
    }

    fn propose(&self, kind: EntryKind, scope: Scope, title: &str) -> i64 {
        self.store
            .propose(
                NewEntry {
                    kind,
                    scope,
                    title: title.into(),
                    text: format!("{title} text"),
                    why: format!("{title} why"),
                    user_answer: None,
                    source: Source {
                        session: Some(3),
                        question_ref: Some("q-1".into()),
                        ..Source::default()
                    },
                },
                10,
            )
            .unwrap()
            .id
    }

    fn approve(&self, kind: EntryKind, scope: Scope, title: &str) -> i64 {
        let id = self.propose(kind, scope, title);
        self.store.approve(id, None, false, &[], 20).unwrap();
        id
    }

    /// Each harness's default home, all under the temp home.
    fn homes(&self) -> impl Fn(Kind) -> Option<PathBuf> + '_ {
        move |kind| match kind {
            Kind::Claude => Some(self.home.join(".claude")),
            Kind::Codex => Some(self.home.join(".codex")),
            Kind::OpenCode => Some(self.home.join(".config").join("opencode")),
            Kind::Kiro => Some(self.home.join(".kiro")),
            _ => None,
        }
    }

    fn regenerate(&self, book: &Book, workspaces: &[PathBuf]) -> Report {
        regenerate(&self.store, book, &self.data, &self.homes(), workspaces)
    }

    /// The off switch: the log is turned off, then everything is removed.
    fn remove_all(&self, book: &Book, workspaces: &[PathBuf]) -> Report {
        service::set_enabled(false);
        let report = remove_all(book, &self.data, &self.homes(), workspaces);
        service::set_enabled(true);
        report
    }
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn init_repo(dir: &Path) {
    git_out(dir, &["init", "-q"]);
    git_out(dir, &["config", "user.name", "Test"]);
    git_out(dir, &["config", "user.email", "test@example.com"]);
    git_out(dir, &["config", "commit.gpgsign", "false"]);
    fs::write(dir.join("README.md"), "readme\n").unwrap();
    git_out(dir, &["add", "README.md"]);
    git_out(dir, &["commit", "-q", "-m", "init"]);
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The text between Gespan's markers.
fn section_of(text: &str) -> String {
    let begin = text.find(BEGIN).expect("a Gespan section") + BEGIN.len();
    let end = text[begin..].find(END).expect("an end marker") + begin;
    text[begin..end].trim_matches(['\r', '\n']).to_owned()
}

fn extra_profiles() -> Book {
    Book {
        accounts: [Kind::Claude, Kind::Codex]
            .into_iter()
            .map(|kind| Account {
                id: "work".into(),
                provider: kind.id().into(),
                label: "Work".into(),
            })
            .collect(),
        ..Book::default()
    }
}

fn profile_dir(data: &Path, kind: Kind, id: &str) -> PathBuf {
    data.join("provider-accounts").join(kind.id()).join(id)
}

#[test]
fn preferences_go_to_every_claude_and_codex_profile() {
    let f = fixture();
    f.install_claude_and_codex();
    let book = extra_profiles();
    for kind in [Kind::Claude, Kind::Codex] {
        fs::create_dir_all(profile_dir(&f.data, kind, "work")).unwrap();
    }
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    f.propose(EntryKind::Preference, Scope::Global, "Unreviewed wish");

    let report = f.regenerate(&book, &[]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let files = [
        f.home.join(".claude").join("CLAUDE.md"),
        f.home.join(".codex").join("AGENTS.md"),
        profile_dir(&f.data, Kind::Claude, "work").join("CLAUDE.md"),
        profile_dir(&f.data, Kind::Codex, "work").join("AGENTS.md"),
    ];
    for file in &files {
        let body = section_of(&read(file));
        assert!(body.contains("Answer tersely text"), "{}", file.display());
        assert!(!body.contains("Unreviewed wish"), "{}", file.display());
        assert!(report.written.contains(file), "{}", file.display());
    }
    // Harnesses that are not installed get nothing created for them.
    assert!(!f.home.join(".kiro").exists());
    assert!(!f.home.join(".config").exists());
    // The targets say the same, and global files are sections, not owned.
    let targets = preference_targets(&book, &f.data, &f.homes());
    for file in &files {
        let target = targets.iter().find(|t| &t.path == file).unwrap();
        assert!(matches!(target.mode, Mode::Section));
    }
}

#[test]
fn opencode_and_kiro_preferences_only_when_installed() {
    let f = fixture();
    let opencode = f.home.join(".config").join("opencode");
    fs::create_dir_all(&opencode).unwrap();
    // OpenCode reads ~/.claude/CLAUDE.md only while its own AGENTS.md is
    // missing, so Gespan writes into its file only when the user has one.
    let targets = preference_targets(&Book::default(), &f.data, &f.homes());
    assert!(!targets.iter().any(|t| t.harness == Kind::OpenCode));
    fs::write(opencode.join("AGENTS.md"), "my opencode rules\n").unwrap();
    fs::create_dir_all(f.home.join(".kiro")).unwrap();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");

    let report = f.regenerate(&Book::default(), &[]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let agents = read(&opencode.join("AGENTS.md"));
    assert!(agents.starts_with("my opencode rules\n"), "{agents}");
    assert!(section_of(&agents).contains("Answer tersely text"));
    let kiro = read(
        &f.home
            .join(".kiro")
            .join("steering")
            .join("gespan-preferences.md"),
    );
    assert!(kiro.starts_with("---\ninclusion: always\n---\n"), "{kiro}");
    assert!(section_of(&kiro).contains("Answer tersely text"));
}

#[test]
fn project_pointer_files_are_excluded_from_git() {
    let f = fixture();
    let worktree = f.repo.parent().unwrap().join("wt");
    git_out(
        &f.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            worktree.to_str().unwrap(),
        ],
    );
    f.approve(EntryKind::Decision, scope_for(&f.repo), "Use sqlite");
    let workspaces = [f.repo.clone(), worktree.clone()];

    let report = f.regenerate(&Book::default(), &workspaces);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    for ws in &workspaces {
        let targets = project_targets(ws);
        assert_eq!(targets.len(), 3, "{targets:?}");
        for target in &targets {
            assert!(matches!(target.mode, Mode::OwnedFile));
            assert!(target.path.starts_with(ws), "{}", target.path.display());
            assert!(
                section_of(&read(&target.path)).contains("Use sqlite text"),
                "{}",
                target.path.display()
            );
        }
        assert_eq!(
            git_out(ws, &["status", "--porcelain"]),
            "",
            "{}",
            ws.display()
        );
    }
    let cursor = read(
        &worktree
            .join(".cursor")
            .join("rules")
            .join("gespan-decisions.mdc"),
    );
    assert!(cursor.starts_with("---\n"), "{cursor}");
    assert!(cursor.contains("alwaysApply: true"), "{cursor}");
    // One exclude file, in the common dir, with each pattern once.
    let common = git_common_dir(&worktree).unwrap();
    let exclude = read(&common.join("info").join("exclude"));
    for pattern in [
        "/.claude/rules/gespan-decisions.md",
        "/.cursor/rules/gespan-decisions.mdc",
        "/.kiro/steering/gespan-decisions.md",
    ] {
        assert_eq!(
            exclude.lines().filter(|l| *l == pattern).count(),
            1,
            "{exclude}"
        );
    }
    // Nothing was written into .gitignore.
    assert!(!f.repo.join(".gitignore").exists());
    assert!(!worktree.join(".gitignore").exists());
}

#[test]
fn shared_file_holds_active_only_never_pending() {
    let f = fixture();
    let scope = scope_for(&f.repo);
    f.approve(EntryKind::Decision, scope.clone(), "Use sqlite");
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    f.propose(EntryKind::Decision, scope.clone(), "Maybe postgres");
    let replaced = f.approve(EntryKind::Decision, scope.clone(), "Use mysql");
    let newer = f.propose(EntryKind::Decision, scope.clone(), "Use mariadb");
    f.store
        .approve(newer, None, false, &[replaced], 30)
        .unwrap();
    let rejected = f.propose(EntryKind::Decision, scope, "Use mongo");
    f.store.reject(rejected, 30).unwrap();
    // A folder outside git keeps its shared file in the data dir.
    let plain = f.home.parent().unwrap().join("plain");
    fs::create_dir_all(&plain).unwrap();
    f.approve(EntryKind::Decision, scope_for(&plain), "Plain decision");

    let report = f.regenerate(&Book::default(), &[f.repo.clone(), plain.clone()]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let shared = shared_file(&f.repo, &f.data);
    assert_eq!(
        shared,
        git_common_dir(&f.repo)
            .unwrap()
            .join("gespan")
            .join("decisions.md")
    );
    let cursor = f
        .repo
        .join(".cursor")
        .join("rules")
        .join("gespan-decisions.mdc");
    for file in [&shared, &cursor] {
        let body = section_of(&read(file));
        assert!(body.contains("Use sqlite text"), "{body}");
        assert!(body.contains("Use mariadb text"), "{body}");
        // Cursor has no global preference file, so the project file
        // carries preferences too.
        assert!(body.contains("Answer tersely text"), "{body}");
        for absent in [
            "Maybe postgres",
            "[not yet approved",
            "Use mysql",
            "Use mongo",
        ] {
            assert!(!body.contains(absent), "{absent} in {body}");
        }
        assert!(!body.contains("Plain decision"), "{body}");
    }
    let plain_shared = shared_file(&plain, &f.data);
    assert!(plain_shared.starts_with(f.data.join("decisions")));
    let body = section_of(&read(&plain_shared));
    assert!(body.contains("Plain decision text"), "{body}");
    assert!(!body.contains("Use sqlite"), "{body}");
}

#[test]
fn shared_file_respects_start_budget() {
    let f = fixture();
    let scope = scope_for(&f.repo);
    for n in 0..120 {
        f.approve(
            EntryKind::Decision,
            scope.clone(),
            &format!("Decision number {n} with a fairly long title to fill the file"),
        );
    }
    let report = f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let mut files = vec![shared_file(&f.repo, &f.data)];
    files.extend(project_targets(&f.repo).into_iter().map(|t| t.path));
    for file in files {
        let body = section_of(&read(&file));
        assert!(body.len() <= START_BUDGET, "{} bytes", body.len());
        assert!(
            body.contains("more decisions: use decisions_search"),
            "{body}"
        );
        // The newest decision is kept, the oldest dropped.
        assert!(body.contains("Decision number 119 "), "{body}");
        assert!(!body.contains("Decision number 0 "), "{body}");
    }
}

#[test]
fn regenerate_reports_overwritten_user_edits() {
    let f = fixture();
    f.install_claude_and_codex();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    f.approve(EntryKind::Decision, scope_for(&f.repo), "Use sqlite");
    let claude = f.home.join(".claude").join("CLAUDE.md");
    let first = f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    assert!(first.user_edits_overwritten.is_empty());

    // An edit outside the markers is the user's text, not an overwrite.
    let text = read(&claude);
    fs::write(&claude, format!("my own rule\n\n{text}")).unwrap();
    let report = f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    assert!(report.user_edits_overwritten.is_empty());
    assert!(read(&claude).starts_with("my own rule\n\n"));

    // An edit inside the section is replaced, and reported.
    let text = read(&claude).replace("Answer tersely text", "Answer at length");
    fs::write(&claude, &text).unwrap();
    let rules = f
        .repo
        .join(".claude")
        .join("rules")
        .join("gespan-decisions.md");
    let owned = read(&rules).replace("Use sqlite text", "hand edit");
    fs::write(&rules, owned).unwrap();
    let report = f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    assert!(
        report.user_edits_overwritten.contains(&claude),
        "{report:?}"
    );
    assert!(report.user_edits_overwritten.contains(&rules), "{report:?}");
    assert_eq!(report.user_edits_overwritten.len(), 2, "{report:?}");
    let now = read(&claude);
    assert!(now.contains("Answer tersely text") && !now.contains("Answer at length"));
    assert!(now.starts_with("my own rule\n\n"));

    // Nothing edited since: nothing reported.
    let report = f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    assert!(report.user_edits_overwritten.is_empty(), "{report:?}");
    assert!(
        report.written.is_empty(),
        "unchanged files are not rewritten: {report:?}"
    );
}

#[test]
fn regenerate_keeps_user_edit_made_outside_markers_between_read_and_write() {
    let f = fixture();
    f.install_claude_and_codex();
    let claude = f.home.join(".claude").join("CLAUDE.md");
    fs::write(&claude, "user line one\n").unwrap();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    let target = claude.clone();
    BEFORE_WRITE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |path: &Path| {
            if path == target {
                let now = fs::read_to_string(path).unwrap();
                fs::write(path, format!("{now}user line two\n")).unwrap();
            }
        }));
    });
    let report = f.regenerate(&Book::default(), &[]);
    BEFORE_WRITE.with(|hook| *hook.borrow_mut() = None);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let text = read(&claude);
    assert!(text.starts_with("user line one\nuser line two\n"), "{text}");
    assert!(section_of(&text).contains("Answer tersely text"), "{text}");
    assert_eq!(text.matches(BEGIN).count(), 1, "{text}");
}

#[test]
fn remove_all_restores_files_to_user_text_and_deletes_owned_files() {
    let f = fixture();
    f.install_claude_and_codex();
    let claude = f.home.join(".claude").join("CLAUDE.md");
    let codex = f.home.join(".codex").join("AGENTS.md");
    // Ends without a newline: removal may leave one, and nothing else.
    let user = "# Mine\n\tkeep  this\nlast line";
    fs::write(&claude, user).unwrap();
    let common = git_common_dir(&f.repo).unwrap();
    let exclude = common.join("info").join("exclude");
    fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    fs::write(&exclude, "# user excludes\n*.log\n").unwrap();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    f.approve(EntryKind::Decision, scope_for(&f.repo), "Use sqlite");
    let workspaces = [f.repo.clone()];
    f.regenerate(&Book::default(), &workspaces);
    let owned: Vec<PathBuf> = project_targets(&f.repo)
        .into_iter()
        .map(|t| t.path)
        .collect();
    assert!(owned.iter().all(|p| p.exists()));
    assert!(codex.exists());

    let report = f.remove_all(&Book::default(), &workspaces);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let restored = read(&claude);
    assert!(
        restored == user || restored == format!("{user}\n"),
        "{restored:?}"
    );
    // A file Gespan created holding only its section is gone.
    assert!(!codex.exists());
    for path in &owned {
        assert!(!path.exists(), "{}", path.display());
    }
    // So are the folders it made for them, and the shared file.
    assert!(!f.repo.join(".cursor").exists());
    assert!(!f.repo.join(".kiro").exists());
    assert!(!f.repo.join(".claude").exists());
    assert!(!shared_file(&f.repo, &f.data).exists());
    assert_eq!(read(&exclude), "# user excludes\n*.log\n");
    assert_eq!(git_out(&f.repo, &["status", "--porcelain"]), "");
    // Running it again changes nothing.
    let again = f.remove_all(&Book::default(), &workspaces);
    assert!(
        again.errors.is_empty() && again.written.is_empty(),
        "{again:?}"
    );
    assert_eq!(read(&claude), restored);
}

#[test]
fn remove_all_cleans_profiles_and_workspaces_no_longer_known() {
    let f = fixture();
    f.install_claude_and_codex();
    let book = extra_profiles();
    let work = profile_dir(&f.data, Kind::Claude, "work");
    fs::create_dir_all(&work).unwrap();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    f.approve(EntryKind::Decision, scope_for(&f.repo), "Use sqlite");
    f.regenerate(&book, std::slice::from_ref(&f.repo));
    assert!(work.join("CLAUDE.md").exists());

    // The account was removed and the project closed since.
    f.remove_all(&Book::default(), &[]);
    assert!(!work.join("CLAUDE.md").exists());
    assert!(!f.repo.join(".claude").exists());
    assert!(!shared_file(&f.repo, &f.data).exists());
    assert_eq!(git_out(&f.repo, &["status", "--porcelain"]), "");
}

#[test]
fn an_empty_log_writes_nothing_and_withdraws_what_it_wrote() {
    let f = fixture();
    f.install_claude_and_codex();
    let claude = f.home.join(".claude").join("CLAUDE.md");
    fs::write(&claude, "mine\n").unwrap();
    let report = f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    assert!(report.written.is_empty(), "{report:?}");
    assert_eq!(read(&claude), "mine\n");
    assert!(!f.repo.join(".claude").exists());
    assert!(!f.home.join(".codex").join("AGENTS.md").exists());

    // The only preference archived: its section goes, the user's text stays.
    let id = f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    assert!(read(&claude).contains("Answer tersely text"));
    f.store.archive(id, 40).unwrap();
    f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    assert_eq!(read(&claude), "mine\n");
    assert!(!f.repo.join(".claude").exists());
}

#[test]
fn a_run_scheduled_before_the_log_is_turned_off_writes_nothing() {
    let f = fixture();
    f.install_claude_and_codex();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    let claude = f.home.join(".claude").join("CLAUDE.md");
    // A regeneration is scheduled, then the user turns the log off before
    // it gets the lock.
    let report = std::thread::scope(|scope| {
        let held = hold_writing();
        let run = scope.spawn(|| f.regenerate(&Book::default(), std::slice::from_ref(&f.repo)));
        service::set_enabled(false);
        drop(held);
        run.join().unwrap()
    });
    service::set_enabled(true);
    assert!(report.written.is_empty(), "{report:?}");
    assert!(!claude.exists());
    assert!(!f.repo.join(".cursor").exists());
}

#[test]
fn a_removal_scheduled_before_the_log_is_turned_back_on_removes_nothing() {
    let f = fixture();
    f.install_claude_and_codex();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    f.regenerate(&Book::default(), &[]);
    let claude = f.home.join(".claude").join("CLAUDE.md");
    assert!(claude.exists());
    // Off schedules the removal; on again before it runs.
    service::set_enabled(false);
    let report = std::thread::scope(|scope| {
        let held = hold_writing();
        let run = scope.spawn(|| remove_all(&Book::default(), &f.data, &f.homes(), &[]));
        service::set_enabled(true);
        drop(held);
        run.join().unwrap()
    });
    assert!(report.written.is_empty(), "{report:?}");
    assert!(section_of(&read(&claude)).contains("Answer tersely text"));
}

#[test]
fn moved_default_homes_are_written_and_cleaned_after_they_move_back() {
    let f = fixture();
    let root = f.home.parent().unwrap().to_path_buf();
    // CLAUDE_CONFIG_DIR and XDG_CONFIG_HOME point outside the home folder.
    let claude_dir = root.join("claude-config");
    let xdg_opencode = root.join("xdg").join("opencode");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::create_dir_all(&xdg_opencode).unwrap();
    fs::write(xdg_opencode.join("AGENTS.md"), "mine\n").unwrap();
    fs::create_dir_all(f.home.join(".claude")).unwrap();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    let moved = |kind: Kind| match kind {
        Kind::Claude => Some(claude_dir.clone()),
        Kind::OpenCode => Some(xdg_opencode.clone()),
        _ => None,
    };
    let report = regenerate(&f.store, &Book::default(), &f.data, &moved, &[]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(section_of(&read(&claude_dir.join("CLAUDE.md"))).contains("Answer tersely text"));
    assert!(section_of(&read(&xdg_opencode.join("AGENTS.md"))).contains("Answer tersely text"));
    assert!(!f.home.join(".claude").join("CLAUDE.md").exists());

    // The variables are gone by the time the log is turned off: the
    // recorded files are still cleaned.
    let report = f.remove_all(&Book::default(), &[]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(!claude_dir.join("CLAUDE.md").exists());
    assert_eq!(read(&xdg_opencode.join("AGENTS.md")), "mine\n");
}

#[test]
fn a_worktree_created_after_the_last_regenerate_gets_the_block_until_written() {
    let f = fixture();
    // start_context reads the process-wide store, so use it here.
    let scope = scope_for(&f.repo);
    service::with(|store| {
        let id = store
            .propose(
                NewEntry {
                    kind: EntryKind::Decision,
                    scope: scope.clone(),
                    title: "Use sqlite".into(),
                    text: "Use sqlite text".into(),
                    why: "why".into(),
                    user_answer: None,
                    source: Source {
                        session: Some(1),
                        question_ref: Some("q".into()),
                        ..Source::default()
                    },
                },
                10,
            )
            .unwrap()
            .id;
        store.approve(id, None, false, &[], 20).unwrap();
    })
    .unwrap();
    let run = |workspaces: &[PathBuf]| {
        service::with(|store| regenerate(store, &Book::default(), &f.data, &f.homes(), workspaces))
            .unwrap()
    };
    run(std::slice::from_ref(&f.repo));
    let start = |kind: Kind, ws: &Path| crate::decisions::start_context(kind, ws, None);
    for kind in [Kind::Claude, Kind::Cursor, Kind::Kiro] {
        assert_eq!(start(kind, &f.repo), None, "{kind:?} reads the file");
    }

    let worktree = f.repo.parent().unwrap().join("late");
    git_out(
        &f.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "late",
            worktree.to_str().unwrap(),
        ],
    );
    // Its first session gets the decisions injected instead.
    for kind in [Kind::Claude, Kind::Cursor, Kind::Kiro] {
        let (block, _) = start(kind, &worktree).expect("injected");
        assert!(block.contains("Use sqlite text"), "{kind:?}: {block}");
    }
    // The regeneration the session schedules writes its files, excluded.
    let report = run(&[f.repo.clone(), worktree.clone()]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    for target in project_targets(&worktree) {
        assert!(section_of(&read(&target.path)).contains("Use sqlite text"));
    }
    assert_eq!(git_out(&worktree, &["status", "--porcelain"]), "");
    for kind in [Kind::Claude, Kind::Cursor, Kind::Kiro] {
        assert_eq!(start(kind, &worktree), None, "{kind:?}");
    }
    // A harness that reads no file always gets the block.
    assert!(start(Kind::Codex, &worktree).is_some());
}

#[test]
fn oversized_or_irregular_files_are_never_read_or_written() {
    let f = fixture();
    f.install_claude_and_codex();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    let claude = f.home.join(".claude").join("CLAUDE.md");
    let big = "x".repeat(MAX_FILE_BYTES as usize + 1);
    fs::write(&claude, &big).unwrap();
    // A folder where Codex's file would be.
    let codex = f.home.join(".codex").join("AGENTS.md");
    fs::create_dir_all(&codex).unwrap();

    let report = f.regenerate(&Book::default(), &[]);
    let failed: Vec<&PathBuf> = report.errors.iter().map(|(p, _)| p).collect();
    assert!(failed.contains(&&claude), "{report:?}");
    assert!(failed.contains(&&codex), "{report:?}");
    assert!(read(&claude) == big, "the large file is untouched");
    assert!(codex.is_dir());
    assert!(report.written.is_empty(), "{report:?}");
}

#[cfg(unix)]
#[test]
fn project_files_that_are_symlinks_or_lead_into_git_are_refused() {
    use std::os::unix::fs::symlink;
    let f = fixture();
    f.approve(EntryKind::Decision, scope_for(&f.repo), "Use sqlite");
    // Claude's rule file links to a file of the user's.
    let rules = f.repo.join(".claude").join("rules");
    fs::create_dir_all(&rules).unwrap();
    // It even carries Gespan's markers, so only the symlink check stops it.
    let notes = format!("user notes\n{BEGIN}\nold\n{END}\n");
    fs::write(f.repo.join("notes.md"), &notes).unwrap();
    symlink(f.repo.join("notes.md"), rules.join("gespan-decisions.md")).unwrap();
    // Cursor's folder links into .git, where hooks live.
    let hooks = f.repo.join(".git").join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let before: Vec<_> = fs::read_dir(&hooks)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    symlink(&hooks, f.repo.join(".cursor")).unwrap();

    let report = f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    let failed: Vec<&PathBuf> = report.errors.iter().map(|(p, _)| p).collect();
    assert!(
        failed.contains(&&rules.join("gespan-decisions.md")),
        "{report:?}"
    );
    assert!(
        failed.contains(
            &&f.repo
                .join(".cursor")
                .join("rules")
                .join("gespan-decisions.mdc")
        ),
        "{report:?}"
    );
    assert_eq!(read(&f.repo.join("notes.md")), notes);
    let after: Vec<_> = fs::read_dir(&hooks)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(before, after, "nothing was written into .git/hooks");
    assert!(!hooks.join("rules").exists());
    // Kiro's file, which is neither, is still written.
    assert!(f
        .repo
        .join(".kiro")
        .join("steering")
        .join("gespan-decisions.md")
        .exists());
}

#[test]
fn a_project_file_tracked_by_git_is_never_written() {
    let f = fixture();
    let steering = f.repo.join(".kiro").join("steering");
    fs::create_dir_all(&steering).unwrap();
    let tracked = steering.join("gespan-decisions.md");
    // Even one that looks like Gespan's: it is the repository's file.
    let committed = format!("{BEGIN}\nteam text\n{END}\n");
    fs::write(&tracked, &committed).unwrap();
    git_out(&f.repo, &["add", "."]);
    git_out(&f.repo, &["commit", "-q", "-m", "steering"]);
    f.approve(EntryKind::Decision, scope_for(&f.repo), "Use sqlite");

    let report = f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    assert!(
        report.errors.iter().any(|(p, _)| p == &tracked),
        "{report:?}"
    );
    assert_eq!(read(&tracked), committed);
    let again = f.remove_all(&Book::default(), std::slice::from_ref(&f.repo));
    assert!(again.errors.iter().any(|(p, _)| p == &tracked), "{again:?}");
    assert_eq!(read(&tracked), committed);
    assert_eq!(git_out(&f.repo, &["status", "--porcelain"]), "");
}

#[test]
fn claudes_project_file_has_decisions_but_not_preferences() {
    let f = fixture();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    f.approve(EntryKind::Decision, scope_for(&f.repo), "Use sqlite");
    f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    for target in project_targets(&f.repo) {
        let body = section_of(&read(&target.path));
        assert!(body.contains("Use sqlite text"), "{body}");
        // Claude has its preferences in its global CLAUDE.md.
        assert_eq!(
            body.contains("Answer tersely text"),
            target.harness != Kind::Claude,
            "{:?}: {body}",
            target.harness
        );
    }
}

#[test]
fn regenerate_withdraws_files_of_workspaces_and_profiles_no_longer_known() {
    let f = fixture();
    f.install_claude_and_codex();
    let book = extra_profiles();
    let work = profile_dir(&f.data, Kind::Codex, "work");
    fs::create_dir_all(&work).unwrap();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    f.approve(EntryKind::Decision, scope_for(&f.repo), "Use sqlite");
    f.regenerate(&book, std::slice::from_ref(&f.repo));
    assert!(work.join("AGENTS.md").exists());
    assert!(f.repo.join(".claude").exists());

    // The project was closed and the account removed.
    let report = f.regenerate(&Book::default(), &[]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(!work.join("AGENTS.md").exists());
    assert!(!f.repo.join(".claude").exists());
    assert!(!shared_file(&f.repo, &f.data).exists());
    assert_eq!(git_out(&f.repo, &["status", "--porcelain"]), "");
    // Known profiles keep theirs.
    assert!(section_of(&read(&f.home.join(".claude").join("CLAUDE.md"))).contains("Answer tersely"));
}

#[test]
fn removal_reports_no_overwritten_edits() {
    let f = fixture();
    f.install_claude_and_codex();
    let claude = f.home.join(".claude").join("CLAUDE.md");
    fs::write(&claude, "mine\n").unwrap();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    f.regenerate(&Book::default(), &[]);
    fs::write(
        &claude,
        read(&claude).replace("Answer tersely text", "my edit"),
    )
    .unwrap();
    let report = f.remove_all(&Book::default(), &[]);
    assert!(report.user_edits_overwritten.is_empty(), "{report:?}");
    assert_eq!(read(&claude), "mine\n");
}

#[test]
fn remove_all_keeps_folders_gespan_did_not_create() {
    let f = fixture();
    // The user already has an (empty) .kiro folder; .cursor is new.
    fs::create_dir_all(f.repo.join(".kiro")).unwrap();
    f.approve(EntryKind::Decision, scope_for(&f.repo), "Use sqlite");
    f.regenerate(&Book::default(), std::slice::from_ref(&f.repo));
    assert!(f.repo.join(".kiro").join("steering").exists());

    f.remove_all(&Book::default(), std::slice::from_ref(&f.repo));
    assert!(f.repo.join(".kiro").is_dir(), "the user's folder stays");
    assert!(!f.repo.join(".kiro").join("steering").exists());
    assert!(!f.repo.join(".cursor").exists());
}

#[cfg(unix)]
#[test]
fn a_fifo_in_place_of_a_harness_file_is_skipped_without_blocking() {
    use std::os::unix::fs::FileTypeExt;
    let f = fixture();
    f.install_claude_and_codex();
    f.approve(EntryKind::Preference, Scope::Global, "Answer tersely");
    let claude = f.home.join(".claude").join("CLAUDE.md");
    let made = Command::new("mkfifo").arg(&claude).status().unwrap();
    assert!(made.success());
    // Reading a FIFO would block until a writer comes; it must be refused.
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _ = done.send(f.regenerate(&Book::default(), &[]));
        });
        let report = match wait.recv_timeout(std::time::Duration::from_secs(20)) {
            Ok(report) => report,
            Err(_) => {
                // The reader cannot be unblocked, and the scope would wait
                // for it forever: fail the whole run instead of hanging.
                eprintln!("regenerate blocked on a FIFO");
                std::process::abort();
            }
        };
        assert!(
            report.errors.iter().any(|(p, _)| p == &claude),
            "{report:?}"
        );
        assert!(fs::symlink_metadata(&claude).unwrap().file_type().is_fifo());
    });
}
