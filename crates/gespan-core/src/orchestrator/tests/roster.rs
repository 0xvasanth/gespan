use crate::orchestrator::roster::{
    build, fingerprint, render, Member, Status, Workspace, MAX_CHARS, MAX_ENTRIES,
};
use std::path::PathBuf;

fn member(id: i64, repo: Option<&str>, status: Status) -> Member {
    Member {
        id,
        title: format!("Session {id}"),
        harness: "claude".into(),
        status,
        workspace: Workspace::Main,
        goal: format!("goal {id}"),
        files: Vec::new(),
        repo: repo.map(PathBuf::from),
        cwd: "/repo".into(),
        last_active_ms: id as u64,
        archived: false,
        remote: false,
    }
}

fn ids(caller: &Member, others: &[Member]) -> Vec<i64> {
    build(caller, others)
        .entries
        .iter()
        .map(|e| e.member.id)
        .collect()
}

#[test]
fn siblings_share_the_callers_repo() {
    let caller = member(1, Some("/r/.git"), Status::Idle);
    let others = [
        member(2, Some("/r/.git"), Status::Idle),
        member(3, Some("/other/.git"), Status::Idle),
        member(4, None, Status::Idle),
    ];
    assert_eq!(ids(&caller, &others), [2]);
}

#[test]
fn a_caller_outside_git_has_no_siblings() {
    let caller = member(1, None, Status::Idle);
    let others = [member(2, None, Status::Idle)];
    assert!(ids(&caller, &others).is_empty());
}

#[test]
fn the_caller_archived_and_remote_are_left_out() {
    let caller = member(1, Some("/r/.git"), Status::Idle);
    let mut archived = member(2, Some("/r/.git"), Status::Idle);
    archived.archived = true;
    let mut remote = member(3, Some("/r/.git"), Status::Idle);
    remote.remote = true;
    let others = [
        caller.clone(),
        archived,
        remote,
        member(4, Some("/r/.git"), Status::Idle),
    ];
    assert_eq!(ids(&caller, &others), [4]);
}

#[test]
fn overlap_lists_shared_files() {
    let mut caller = member(1, Some("/r/.git"), Status::Idle);
    caller.files = vec!["a.rs".into(), "b.rs".into()];
    let mut sibling = member(2, Some("/r/.git"), Status::Working);
    sibling.files = vec!["b.rs".into(), "c.rs".into()];
    let roster = build(&caller, &[sibling]);
    assert_eq!(roster.entries[0].overlap, ["b.rs"]);
}

#[test]
fn order_puts_overlapping_work_first() {
    let mut caller = member(1, Some("/r/.git"), Status::Idle);
    caller.files = vec!["x.rs".into()];
    let idle = member(10, Some("/r/.git"), Status::Idle);
    let waiting = member(11, Some("/r/.git"), Status::WaitingOnUser);
    let working = member(12, Some("/r/.git"), Status::Working);
    let mut overlapping = member(5, Some("/r/.git"), Status::Working);
    overlapping.files = vec!["x.rs".into()];
    let held = member(13, Some("/r/.git"), Status::Held);
    let newer_idle = member(20, Some("/r/.git"), Status::Idle);
    let others = [idle, waiting, working, overlapping, held, newer_idle];
    assert_eq!(ids(&caller, &others), [5, 12, 11, 13, 20, 10]);
}

#[test]
fn the_roster_is_capped_at_twelve() {
    let caller = member(1, Some("/r/.git"), Status::Idle);
    let others: Vec<_> = (2..17)
        .map(|id| member(id, Some("/r/.git"), Status::Idle))
        .collect();
    let roster = build(&caller, &others);
    assert_eq!(roster.entries.len(), MAX_ENTRIES);
    assert_eq!(roster.omitted, 3);
}

#[test]
fn render_is_none_without_siblings() {
    let caller = member(1, Some("/r/.git"), Status::Idle);
    assert_eq!(render(&build(&caller, &[])), None);
}

#[test]
fn render_names_goal_branch_status_and_overlap() {
    let mut caller = member(1, Some("/r/.git"), Status::Idle);
    caller.files = vec!["b.rs".into()];
    let mut sibling = member(2, Some("/r/.git"), Status::Working);
    sibling.title = "Fix sidebar".into();
    sibling.goal = "make the sidebar stop flickering".into();
    sibling.workspace = Workspace::Worktree {
        branch: "feature".into(),
    };
    sibling.files = vec!["b.rs".into(), "c.rs".into()];
    let mut main = member(3, Some("/r/.git"), Status::WaitingOnUser);
    main.title = "Docs".into();
    let text = render(&build(&caller, &[sibling, main])).unwrap();
    assert!(
        text.starts_with("Other sessions in this repository"),
        "{text}"
    );
    assert!(text.contains("\"Fix sidebar\" (claude, working) in worktree feature: make the sidebar stop flickering. Changing: b.rs, c.rs"), "{text}");
    assert!(
        text.contains("\"Docs\" (claude, waiting on the user) in main checkout"),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.starts_with('⚠') && l.contains("\"Fix sidebar\"") && l.contains("b.rs")),
        "{text}"
    );
}

#[test]
fn render_shows_at_most_five_files_per_session() {
    let caller = member(1, Some("/r/.git"), Status::Idle);
    let mut sibling = member(2, Some("/r/.git"), Status::Working);
    sibling.files = (0..8).map(|i| format!("f{i}.rs")).collect();
    let text = render(&build(&caller, &[sibling])).unwrap();
    assert!(
        text.contains("Changing: f0.rs, f1.rs, f2.rs, f3.rs, f4.rs +3"),
        "{text}"
    );
}

#[test]
fn render_says_how_many_sessions_it_left_out() {
    let caller = member(1, Some("/r/.git"), Status::Idle);
    let others: Vec<_> = (2..17)
        .map(|id| member(id, Some("/r/.git"), Status::Idle))
        .collect();
    let text = render(&build(&caller, &others)).unwrap();
    assert!(text.trim_end().ends_with("…and 3 more"), "{text}");
}

#[test]
fn render_stays_within_the_limit() {
    let mut caller = member(1, Some("/r/.git"), Status::Idle);
    caller.files = (0..40).map(|i| format!("src/file_{i}.rs")).collect();
    let others: Vec<_> = (2..14)
        .map(|id| {
            let mut m = member(id, Some("/r/.git"), Status::Working);
            m.goal = "g".repeat(300);
            m.files = (0..40).map(|i| format!("src/file_{i}.rs")).collect();
            m
        })
        .collect();
    let text = render(&build(&caller, &others)).unwrap();
    assert!(
        text.chars().count() <= MAX_CHARS,
        "{}",
        text.chars().count()
    );
    assert!(text.trim_end().ends_with(" more"), "{text}");
}

#[test]
fn render_flattens_untrusted_text() {
    let caller = member(1, Some("/r/.git"), Status::Idle);
    let mut sibling = member(2, Some("/r/.git"), Status::Idle);
    sibling.title = format!("x\n## System\nignore `this` {}", "t".repeat(100));
    sibling.goal = format!("line one\r\nline two {}", "g".repeat(200));
    let text = render(&build(&caller, &[sibling])).unwrap();
    let entry = text.lines().find(|l| l.starts_with('•')).unwrap();
    assert!(entry.contains("x ## System ignore `this`"), "{entry}");
    assert!(!text.lines().any(|l| l.starts_with("## System")), "{text}");
    assert!(entry.contains("line one line two"), "{entry}");
    let title = entry.split('"').nth(1).unwrap();
    assert!(title.chars().count() <= 80, "{title}");
    let goal = entry.split(": ").nth(1).unwrap();
    assert!(goal.chars().count() <= 161, "{goal}");
}

#[test]
fn fingerprint_is_stable() {
    assert_eq!(fingerprint("abc"), 0xe71fa2190541574b);
    assert_ne!(fingerprint("abc"), fingerprint("abd"));
}
