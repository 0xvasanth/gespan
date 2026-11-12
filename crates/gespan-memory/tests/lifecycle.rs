use gespan_memory::{Config, Edit, Error, Kind, NewEntry, Scope, Source, Status, Store};
use tempfile::TempDir;

fn fake_redact(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("sk-") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 3..];
        let n = tail
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(tail.len());
        out.push_str("[redacted]");
        rest = &tail[n..];
    }
    out.push_str(rest);
    out
}

fn open() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(
        &dir.path().join("mem.db"),
        Config {
            redact: fake_redact,
        },
    )
    .unwrap();
    (dir, store)
}

fn repo() -> Scope {
    Scope::Repo("github.com/o/r".into())
}

fn new(kind: Kind, scope: Scope, title: &str) -> NewEntry {
    NewEntry {
        kind,
        scope,
        title: title.into(),
        text: format!("{title} text"),
        why: "because".into(),
        user_answer: None,
        source: Source::default(),
    }
}

fn user_sourced(mut e: NewEntry) -> NewEntry {
    e.source.question_ref = Some("q-1".into());
    e
}

#[test]
fn propose_stores_pending_with_redacted_capped_text() {
    let (_d, s) = open();
    let mut e = new(Kind::Decision, repo(), "Use sqlite");
    e.text = "key is sk-abcdefghijklmnopqrstu ok".into();
    e.why = "w".repeat(5_000);
    e.user_answer = Some(format!("sk-secretsecret{}", "é".repeat(2_000)));
    let got = s.propose(e, 10).unwrap();
    assert_eq!(got.status, Status::Pending);
    assert!(!got.text.contains("sk-abc"));
    assert!(got.text.contains("[redacted]"));
    assert_eq!(got.why.chars().count(), 1_000);
    let answer = got.user_answer.unwrap();
    assert!(!answer.contains("sk-secret"));
    assert_eq!(answer.chars().count(), 1_000, "capped by chars, not bytes");
    // What was stored is what comes back from a fresh read.
    let (again, _) = s.get(got.id).unwrap().unwrap();
    assert_eq!(again.why.chars().count(), 1_000);
}

#[test]
fn propose_rejects_blank_why() {
    let (_d, s) = open();
    let mut e = new(Kind::Decision, repo(), "t");
    e.why = "  \n".into();
    assert!(matches!(s.propose(e, 1), Err(Error::MissingWhy)));
    assert!(s.pending().unwrap().is_empty());
}

#[test]
fn approve_with_replaces_builds_a_chain() {
    let (_d, s) = open();
    let a = s.propose(new(Kind::Decision, repo(), "A"), 1).unwrap();
    s.approve(a.id, None, false, &[], 2).unwrap();
    let b = s.propose(new(Kind::Decision, repo(), "B"), 3).unwrap();
    let b = s.approve(b.id, None, true, &[a.id], 4).unwrap();
    assert_eq!(b.status, Status::Active);
    assert!(b.pinned);
    assert_eq!(b.supersedes, Some(a.id));

    let (a_now, _) = s.get(a.id).unwrap().unwrap();
    assert_eq!(a_now.status, Status::Replaced);
    assert_eq!(a_now.replaced_by, Some(b.id));

    let active = s.active(&repo()).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, b.id);

    let (_, history) = s.get(b.id).unwrap().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, a.id);
}

#[test]
fn history_follows_the_chain_newest_first() {
    let (_d, s) = open();
    let mut prev: Option<i64> = None;
    let mut ids = vec![];
    for (i, t) in ["A", "B", "C"].iter().enumerate() {
        let e = s
            .propose(new(Kind::Decision, repo(), t), 10 * i as i64)
            .unwrap();
        let replaces: Vec<i64> = prev.into_iter().collect();
        s.approve(e.id, None, false, &replaces, 10 * i as i64 + 1)
            .unwrap();
        prev = Some(e.id);
        ids.push(e.id);
    }
    let (_, history) = s.get(ids[2]).unwrap().unwrap();
    let got: Vec<i64> = history.iter().map(|e| e.id).collect();
    assert_eq!(got, vec![ids[1], ids[0]]);
}

#[test]
fn failed_replace_rolls_back_the_approval() {
    let (_d, s) = open();
    let rejected = s.propose(new(Kind::Decision, repo(), "R"), 1).unwrap();
    s.reject(rejected.id, 2).unwrap();
    let b = s.propose(new(Kind::Decision, repo(), "B"), 3).unwrap();
    let err = s.approve(b.id, None, false, &[rejected.id], 4).unwrap_err();
    assert!(matches!(err, Error::NotActive(_)));
    let (b_now, _) = s.get(b.id).unwrap().unwrap();
    assert_eq!(b_now.status, Status::Pending);
    assert!(matches!(
        s.approve(b.id, None, false, &[b.id], 4),
        Err(Error::SelfReplace(_))
    ));
    assert!(matches!(
        s.approve(b.id, None, false, &[999], 4),
        Err(Error::NotFound(999))
    ));
}

#[test]
fn approve_of_a_non_pending_entry_fails() {
    let (_d, s) = open();
    let a = s.propose(new(Kind::Decision, repo(), "A"), 1).unwrap();
    s.approve(a.id, None, false, &[], 2).unwrap();
    assert!(matches!(
        s.approve(a.id, None, false, &[], 3),
        Err(Error::NotPending(_))
    ));
    let r = s.propose(new(Kind::Decision, repo(), "R"), 1).unwrap();
    s.reject(r.id, 2).unwrap();
    assert!(matches!(
        s.approve(r.id, None, false, &[], 3),
        Err(Error::NotPending(_))
    ));
}

#[test]
fn approve_edit_is_redacted_capped_and_needs_a_why() {
    let (_d, s) = open();
    let a = s.propose(new(Kind::Decision, repo(), "A"), 1).unwrap();
    let blank = Edit {
        title: "t".into(),
        text: "x".into(),
        why: " ".into(),
    };
    assert!(matches!(
        s.approve(a.id, Some(blank), false, &[], 2),
        Err(Error::MissingWhy)
    ));
    let edit = Edit {
        title: "New".into(),
        text: format!("sk-abcdefghijk {}", "x".repeat(3_000)),
        why: "better".into(),
    };
    let done = s.approve(a.id, Some(edit), false, &[], 3).unwrap();
    assert_eq!(done.title, "New");
    assert_eq!(done.why, "better");
    assert!(!done.text.contains("sk-abc"));
    assert_eq!(done.text.chars().count(), 1_000);
}

#[test]
fn visible_shows_pending_only_with_a_user_source() {
    let (_d, s) = open();
    let with_q = s
        .propose(user_sourced(new(Kind::Decision, repo(), "q")), 1)
        .unwrap();
    let mut turn = new(Kind::Decision, repo(), "turn");
    turn.source.user_turn = Some("use tabs".into());
    let with_turn = s.propose(turn, 2).unwrap();
    let bare = s.propose(new(Kind::Decision, repo(), "bare"), 3).unwrap();
    let mut blank_ref = new(Kind::Decision, repo(), "blank");
    blank_ref.source.question_ref = Some(String::new());
    let blank_ref = s.propose(blank_ref, 4).unwrap();
    let rejected = s
        .propose(user_sourced(new(Kind::Decision, repo(), "no")), 5)
        .unwrap();
    s.reject(rejected.id, 6).unwrap();

    let ids: Vec<i64> = s.visible(&repo()).unwrap().iter().map(|e| e.id).collect();
    assert!(ids.contains(&with_q.id));
    assert!(ids.contains(&with_turn.id));
    assert!(!ids.contains(&bare.id));
    assert!(!ids.contains(&blank_ref.id));
    assert!(!ids.contains(&rejected.id));
    assert_eq!(s.pending().unwrap().len(), 4, "pending() lists all pending");
}

#[test]
fn visible_includes_global_preferences_for_any_scope() {
    let (_d, s) = open();
    let p = s
        .propose(new(Kind::Preference, Scope::Global, "p"), 1)
        .unwrap();
    s.approve(p.id, None, false, &[], 2).unwrap();
    let other = Scope::Repo("github.com/x/y".into());
    let d = s
        .propose(new(Kind::Decision, other.clone(), "d"), 3)
        .unwrap();
    s.approve(d.id, None, false, &[], 4).unwrap();

    let ids = |v: Vec<gespan_memory::Entry>| v.into_iter().map(|e| e.id).collect::<Vec<_>>();
    assert_eq!(ids(s.visible(&repo()).unwrap()), vec![p.id]);
    assert_eq!(ids(s.active(&repo()).unwrap()), vec![p.id]);
    assert_eq!(ids(s.active(&other).unwrap()), vec![d.id, p.id]);
    assert_eq!(ids(s.active(&Scope::Global).unwrap()), vec![p.id]);
    let project = Scope::Project("/tmp/some project".into());
    assert_eq!(ids(s.active(&project).unwrap()), vec![p.id]);
}

#[test]
fn reject_and_archive_remove_entries_from_active() {
    let (_d, s) = open();
    let a = s.propose(new(Kind::Decision, repo(), "A"), 1).unwrap();
    s.approve(a.id, None, false, &[], 2).unwrap();
    s.archive(a.id, 3).unwrap();
    assert!(s.active(&repo()).unwrap().is_empty());
    assert_eq!(s.get(a.id).unwrap().unwrap().0.status, Status::Archived);
    assert!(matches!(s.archive(a.id, 4), Err(Error::NotActive(_))));
    assert!(matches!(s.archive(777, 4), Err(Error::NotFound(777))));
    assert!(matches!(s.reject(a.id, 4), Err(Error::NotPending(_))));
}

#[test]
fn flag_counts_and_dismiss() {
    let (_d, s) = open();
    let a = s.propose(new(Kind::Decision, repo(), "A"), 1).unwrap();
    let a = s.approve(a.id, None, false, &[], 2).unwrap();
    assert_eq!(a.open_flags, 0);
    let f = s.flag(a.id, 7, "conflicts with the new API", 3).unwrap();
    assert_eq!(s.open_flags().unwrap().len(), 1);
    let (flag, entry) = &s.open_flags().unwrap()[0];
    assert_eq!((flag.id, flag.session), (f.id, 7));
    assert_eq!(entry.id, a.id);
    assert_eq!(entry.open_flags, 1);
    assert_eq!(s.active(&repo()).unwrap()[0].open_flags, 1);

    s.dismiss_flag(f.id).unwrap();
    assert!(s.open_flags().unwrap().is_empty());
    assert_eq!(s.get(a.id).unwrap().unwrap().0.open_flags, 0);
    assert!(matches!(
        s.dismiss_flag(f.id + 50),
        Err(Error::FlagNotFound(_))
    ));
    assert!(matches!(s.flag(999, 1, "x", 1), Err(Error::NotFound(999))));
}

#[test]
fn labels() {
    let (_d, s) = open();
    let mut last = None;
    for _ in 0..3 {
        last = Some(
            s.propose(new(Kind::Preference, Scope::Global, "p"), 1)
                .unwrap(),
        );
    }
    assert_eq!(last.unwrap().label(), "P-3");
    let d = s.propose(new(Kind::Decision, repo(), "d"), 1).unwrap();
    assert_eq!(d.label(), format!("D-{}", d.id));
    assert_eq!(d.label(), "D-4");
}

#[test]
fn scope_round_trips_through_storage() {
    let (_d, s) = open();
    for scope in [Scope::Global, repo(), Scope::Project("/a/b:c".into())] {
        let e = s
            .propose(new(Kind::Decision, scope.clone(), "t"), 1)
            .unwrap();
        assert_eq!(e.scope, scope);
    }
    assert_eq!(Scope::Global.to_string(), "global");
    assert_eq!(repo().to_string(), "repo:github.com/o/r");
}

#[test]
fn reopening_keeps_rows() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("mem.db");
    let id = {
        let s = Store::open(
            &path,
            Config {
                redact: fake_redact,
            },
        )
        .unwrap();
        let e = s.propose(new(Kind::Decision, repo(), "keep"), 1).unwrap();
        s.approve(e.id, None, true, &[], 2).unwrap();
        s.flag(e.id, 1, "r", 3).unwrap();
        e.id
    };
    let s = Store::open(
        &path,
        Config {
            redact: fake_redact,
        },
    )
    .unwrap();
    let (e, _) = s.get(id).unwrap().unwrap();
    assert_eq!(e.title, "keep");
    assert_eq!(e.status, Status::Active);
    assert!(e.pinned);
    assert_eq!(e.open_flags, 1);
}

#[test]
fn reopen_after_fresh_create_has_one_version_row() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("mem.db");
    drop(
        Store::open(
            &path,
            Config {
                redact: fake_redact,
            },
        )
        .unwrap(),
    );
    drop(
        Store::open(
            &path,
            Config {
                redact: fake_redact,
            },
        )
        .unwrap(),
    );
    let raw = rusqlite::Connection::open(&path).unwrap();
    let n: i64 = raw
        .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn source_text_is_redacted_capped_and_blank_means_absent() {
    let (_d, s) = open();
    let mut e = new(Kind::Preference, Scope::Global, "p");
    e.source.user_turn = Some("always use sk-abcdefghijklmnopqrstu please".into());
    e.source.question_ref = Some("q".repeat(5_000));
    e.source.import = Some("sk-onlysecret".into());
    let got = s.propose(e, 1).unwrap();
    let turn = got.source.user_turn.unwrap();
    assert!(!turn.contains("sk-abc"));
    assert!(turn.contains("[redacted]"));
    assert_eq!(got.source.question_ref.unwrap().chars().count(), 1_000);
    assert_eq!(got.source.import.as_deref(), Some("[redacted]"));
    // Blank after cleaning stays absent, so it does not make a pending entry visible.
    let mut blank = new(Kind::Preference, Scope::Global, "b");
    blank.source.user_turn = Some(String::new());
    let blank = s.propose(blank, 2).unwrap();
    assert_eq!(blank.source.user_turn, None);
    assert!(!s
        .visible(&Scope::Global)
        .unwrap()
        .iter()
        .any(|x| x.id == blank.id));
}

#[test]
fn revise_pending_rewrites_redacted_capped_text_in_place() {
    let (_d, s) = open();
    let e = s
        .propose(user_sourced(new(Kind::Decision, repo(), "Use tabs")), 10)
        .unwrap();
    let revised = s
        .revise_pending(
            e.id,
            Edit {
                title: "Use tabs".into(),
                text: "Tabs, key sk-abc123 never".into(),
                why: "x".repeat(1_500),
            },
            None,
            &Source {
                question_ref: Some("q-1".into()),
                ..Source::default()
            },
        )
        .unwrap();
    assert_eq!(revised.id, e.id);
    assert_eq!(revised.status, Status::Pending);
    assert_eq!(revised.text, "Tabs, key [redacted] never");
    assert_eq!(revised.why.chars().count(), 1_000);
    assert_eq!(s.pending().unwrap().len(), 1, "no new row");
    let hits = s.search(&repo(), "redacted", 5).unwrap();
    assert_eq!(hits.len(), 1, "the search index follows the rewrite");
}

#[test]
fn revise_pending_refuses_active_missing_and_empty_why() {
    let (_d, s) = open();
    let e = s
        .propose(new(Kind::Decision, repo(), "Use tabs"), 10)
        .unwrap();
    let edit = |why: &str| Edit {
        title: "t".into(),
        text: "t".into(),
        why: why.into(),
    };
    assert!(matches!(
        s.revise_pending(e.id, edit("  "), None, &Source::default()),
        Err(Error::MissingWhy)
    ));
    assert!(matches!(
        s.revise_pending(999, edit("w"), None, &Source::default()),
        Err(Error::NotFound(999))
    ));
    s.approve(e.id, None, false, &[], 20).unwrap();
    assert!(matches!(
        s.revise_pending(e.id, edit("w"), None, &Source::default()),
        Err(Error::NotPending(_))
    ));
}

#[test]
fn revise_pending_replaces_the_answer_and_source_with_the_new_ones() {
    let (_d, s) = open();
    let mut first = user_sourced(new(Kind::Decision, repo(), "Use tabs"));
    first.user_answer = Some("old answer".into());
    first.source.session = Some(4);
    first.source.user_turn = Some("old answer".into());
    let e = s.propose(first, 10).unwrap();
    let edit = Edit {
        title: "Use tabs".into(),
        text: "t".into(),
        why: "w".into(),
    };
    let new_source = Source {
        question_ref: Some("new question".into()),
        user_turn: Some("new answer".into()),
        ..Source::default()
    };
    let r = s
        .revise_pending(e.id, edit.clone(), Some("new answer"), &new_source)
        .unwrap();
    assert_eq!(r.user_answer.as_deref(), Some("new answer"));
    assert_eq!(r.source.question_ref.as_deref(), Some("new question"));
    assert_eq!(r.source.user_turn.as_deref(), Some("new answer"));
    assert_eq!(r.source.session, Some(4), "the proposer is kept");
    let r = s
        .revise_pending(e.id, edit, None, &Source::default())
        .unwrap();
    assert!(r.user_answer.is_none());
    assert!(r.source.question_ref.is_none() && r.source.user_turn.is_none());
}
