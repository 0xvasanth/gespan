use crate::accounts::{Account, Book};
use crate::decisions::files::{BEGIN, END};
use crate::decisions::harness::{
    preference_targets, project_targets, regenerate, remove_all, Mode, BEFORE_WRITE,
};
use crate::decisions::scope::{git_common_dir, scope_for, shared_file};
use crate::provider::Kind;
use gespan_memory::{Config, Kind as EntryKind, NewEntry, Scope, Source, Store, START_BUDGET};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// A temp home, data dir and git repository, and a store of their own.
struct Fixture {
    _tmp: TempDir,
    home: PathBuf,
    data: PathBuf,
    repo: PathBuf,
    store: Store,
}

fn fixture() -> Fixture {
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
        _tmp: tmp,
        home,
        data,
        repo,
        store,
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

    fn regenerate(&self, book: &Book, workspaces: &[PathBuf]) -> crate::decisions::harness::Report {
        regenerate(&self.store, book, &self.data, &self.home, workspaces)
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
    let targets = preference_targets(&book, &f.data, &f.home);
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
    let targets = preference_targets(&Book::default(), &f.data, &f.home);
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
    let owned = read(&rules).replace("Answer tersely text", "hand edit");
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

    let report = remove_all(&Book::default(), &f.data, &f.home, &workspaces);
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
    let again = remove_all(&Book::default(), &f.data, &f.home, &workspaces);
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
    remove_all(&Book::default(), &f.data, &f.home, &[]);
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
