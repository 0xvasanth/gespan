use super::super::scope_for;
use super::super::service::{self, isolated};
use super::super::toolkit::Toolkit;
use crate::tool_servers::{self, Mount, DECISIONS, DECISIONS_READ};
use gespan_memory::{Entry, Kind, NewEntry, Scope, Source, Status, STANDING_INSTRUCTION};
use mcp_hub::{Grant, Toolkit as _};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;

const FULL: Toolkit = Toolkit { may_propose: true };
const READ: Toolkit = Toolkit { may_propose: false };

const OFF: &str = "The decision log is off. The user can turn it on in Settings.";
const UNVERIFIED: &str = "Saved for review; not shown to other sessions because the answer wasn't found in your conversation with the user.";

/// A grant no session refreshed, so the project is the workspace itself.
fn grant(session: i64, workspace: &Path, toolkit: &str) -> Grant {
    Grant {
        session,
        workspace: workspace.to_path_buf(),
        toolkits: BTreeSet::from([toolkit.to_owned()]),
        servers: BTreeSet::new(),
    }
}

fn names(toolkit: &Toolkit, grant: &Grant) -> Vec<String> {
    toolkit
        .tools(grant)
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

fn say(result: Result<Value, String>) -> String {
    let value = result.unwrap();
    assert_eq!(value["isError"], false);
    value["content"][0]["text"].as_str().unwrap().to_owned()
}

fn call(toolkit: &Toolkit, grant: &Grant, tool: &str, input: Value) -> String {
    say(toolkit.call(grant, tool, input))
}

/// What each session's user wrote, for the verification the toolkit does.
fn users_said(texts: &'static [(i64, &'static [&'static str])]) {
    service::set_user_text_source(Box::new(move |session| {
        texts
            .iter()
            .filter(|(id, _)| *id == session)
            .flat_map(|(_, lines)| lines.iter().map(|line| (*line).to_owned()))
            .collect()
    }));
}

fn scope(workspace: &Path) -> Scope {
    scope_for(workspace)
}

fn seed(scope: &Scope, kind: Kind, title: &str, text: &str, active: bool) -> Entry {
    service::with(|store| {
        let entry = store
            .propose(
                NewEntry {
                    kind,
                    scope: scope.clone(),
                    title: title.into(),
                    text: text.into(),
                    why: "because".into(),
                    user_answer: None,
                    source: Source {
                        session: Some(1),
                        question_ref: Some("q".into()),
                        ..Source::default()
                    },
                },
                10,
            )
            .unwrap();
        if active {
            store.approve(entry.id, None, false, &[], 20).unwrap()
        } else {
            entry
        }
    })
    .unwrap()
}

fn pending() -> Vec<Entry> {
    service::with(|store| store.pending().unwrap()).unwrap()
}

fn propose(session: i64, dir: &Path, answer: &str) -> String {
    call(
        &FULL,
        &grant(session, dir, DECISIONS),
        "decisions_propose",
        json!({
            "title": "Store decisions in SQLite",
            "text": "Keep the decision log in one SQLite file.",
            "why": "One file is easy to back up.",
            "question": "Where should the log live?",
            "user_answer": answer,
        }),
    )
}

#[test]
fn propose_records_session_and_user_answer_as_pending() {
    let _guard = isolated();
    users_said(&[(5, &["Let's keep it in one SQLite file, please"])]);
    let dir = tempfile::tempdir().unwrap();
    let said = propose(5, dir.path(), "keep it in one SQLite file");
    let rows = pending();
    assert_eq!(rows.len(), 1);
    let entry = &rows[0];
    assert!(said.contains(&entry.label()), "{said}");
    assert!(!said.contains("not shown to other sessions"), "{said}");
    assert_eq!(entry.status, Status::Pending);
    assert_eq!(entry.kind, Kind::Decision);
    assert_eq!(entry.scope, scope(dir.path()));
    assert_eq!(entry.source.session, Some(5));
    assert_eq!(
        entry.user_answer.as_deref(),
        Some("keep it in one SQLite file")
    );
    assert_eq!(
        entry.source.question_ref.as_deref(),
        Some("Where should the log live?")
    );
    assert!(entry.source.user_turn.is_some());
    // The user source makes it visible to another session of the project.
    let other = call(
        &FULL,
        &grant(6, dir.path(), DECISIONS),
        "decisions_list",
        json!({}),
    );
    assert!(other.contains("not yet approved"), "{other}");
    assert!(other.contains("session 5"), "{other}");
}

#[test]
fn an_answer_the_user_never_wrote_gets_no_user_source() {
    let _guard = isolated();
    users_said(&[(5, &["I prefer plain files"])]);
    let dir = tempfile::tempdir().unwrap();
    let said = propose(5, dir.path(), "Yes, use SQLite for everything");
    assert!(said.contains(UNVERIFIED), "{said}");
    let entry = &pending()[0];
    assert!(entry.source.question_ref.is_none());
    assert!(entry.source.user_turn.is_none());
    assert_eq!(entry.source.session, Some(5));
    assert!(
        entry.user_answer.is_none(),
        "an unverified quote is not kept as the user's answer"
    );
    let other = call(
        &FULL,
        &grant(6, dir.path(), DECISIONS),
        "decisions_list",
        json!({}),
    );
    assert!(
        !other.contains("SQLite"),
        "hidden from other sessions: {other}"
    );
}

#[test]
fn without_a_registered_source_every_proposal_fails_closed() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let said = propose(5, dir.path(), "keep it in one SQLite file");
    assert!(said.contains(UNVERIFIED), "{said}");
    assert!(pending()[0].source.user_turn.is_none());
}

#[test]
fn only_the_callers_own_session_can_vouch_for_an_answer() {
    let _guard = isolated();
    users_said(&[
        (5, &["nothing relevant"]),
        (6, &["keep it in one SQLite file"]),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let said = propose(5, dir.path(), "keep it in one SQLite file");
    assert!(
        said.contains(UNVERIFIED),
        "session 6's words are not session 5's"
    );
    assert!(pending()[0].source.user_turn.is_none());
}

#[test]
fn verification_collapses_whitespace_but_is_case_sensitive() {
    let _guard = isolated();
    users_said(&[(5, &["Please   keep it\nin one SQLite   file"])]);
    let dir = tempfile::tempdir().unwrap();
    let ok = propose(5, dir.path(), "  keep it in one\tSQLite file ");
    assert!(!ok.contains("not shown to other sessions"), "{ok}");
    assert!(pending()[0].source.user_turn.is_some());

    let other_dir = tempfile::tempdir().unwrap();
    let wrong_case = propose(5, other_dir.path(), "keep it in one sqlite file");
    assert!(wrong_case.contains(UNVERIFIED), "{wrong_case}");
}

#[test]
fn an_empty_answer_never_verifies_even_against_any_text() {
    let _guard = isolated();
    users_said(&[(5, &["anything at all"])]);
    let dir = tempfile::tempdir().unwrap();
    let said = call(
        &FULL,
        &grant(5, dir.path(), DECISIONS),
        "decisions_propose",
        json!({"title": "T", "text": "x", "why": "y", "user_answer": "   "}),
    );
    assert!(said.contains(UNVERIFIED), "{said}");
    assert!(pending()[0].source.user_turn.is_none());
    let none = call(
        &FULL,
        &grant(5, dir.path(), DECISIONS),
        "decisions_propose",
        json!({"title": "Another thing", "text": "x", "why": "y"}),
    );
    assert!(none.contains(UNVERIFIED), "{none}");
}

#[test]
fn propose_without_why_is_a_tool_error() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let g = grant(5, dir.path(), DECISIONS);
    for why in [json!(""), json!("   ")] {
        let error = FULL
            .call(
                &g,
                "decisions_propose",
                json!({"title": "T", "text": "x", "why": why}),
            )
            .unwrap_err();
        assert!(error.contains("why"), "{error}");
    }
    let missing = FULL
        .call(&g, "decisions_propose", json!({"title": "T", "text": "x"}))
        .unwrap_err();
    assert_eq!(missing, "why is required");
    assert!(pending().is_empty());
}

#[test]
fn propose_redacts_and_caps_what_it_stores() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    call(
        &FULL,
        &grant(5, dir.path(), DECISIONS),
        "decisions_propose",
        json!({
            "title": "Key policy",
            "text": format!("Use sk-abcdefghijklmnopqrstuvwx {}", "a".repeat(3_000)),
            "why": "so",
        }),
    );
    let entry = &pending()[0];
    assert!(
        !entry.text.contains("sk-abcdefghijklmnopqrstuvwx"),
        "{}",
        entry.text
    );
    assert!(entry.text.chars().count() <= 1_000);
}

#[test]
fn list_returns_rendered_block_for_the_callers_scope() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    seed(
        &scope(dir.path()),
        Kind::Decision,
        "Use tabs",
        "Tabs everywhere",
        true,
    );
    seed(
        &Scope::Global,
        Kind::Preference,
        "Be terse",
        "Short replies",
        true,
    );
    seed(
        &scope(elsewhere.path()),
        Kind::Decision,
        "Use spaces",
        "Spaces only",
        true,
    );
    let listed = call(
        &FULL,
        &grant(5, dir.path(), DECISIONS),
        "decisions_list",
        json!({}),
    );
    assert!(listed.contains(STANDING_INSTRUCTION), "{listed}");
    assert!(listed.contains("Use tabs"), "{listed}");
    assert!(listed.contains("Be terse"), "{listed}");
    assert!(
        !listed.contains("Use spaces"),
        "another project's decision: {listed}"
    );
}

#[test]
fn search_finds_by_words_and_stays_in_scope() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    seed(
        &scope(dir.path()),
        Kind::Decision,
        "Retry policy",
        "Retry three times with backoff",
        true,
    );
    seed(
        &scope(elsewhere.path()),
        Kind::Decision,
        "Retry elsewhere",
        "Retry forever",
        true,
    );
    let g = grant(5, dir.path(), DECISIONS);
    let found = call(
        &FULL,
        &g,
        "decisions_search",
        json!({"query": "retry", "limit": 5}),
    );
    assert!(found.contains("Retry policy"), "{found}");
    assert!(!found.contains("elsewhere"), "{found}");
    let none = call(&FULL, &g, "decisions_search", json!({"query": "zebra"}));
    assert!(none.contains("No matching"), "{none}");
    assert!(FULL
        .call(&g, "decisions_search", json!({"limit": 3}))
        .is_err());
}

#[test]
fn get_accepts_label_and_shows_history() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let s = scope(dir.path());
    let old = seed(&s, Kind::Decision, "Use tabs", "Tabs everywhere", true);
    let new = seed(&s, Kind::Decision, "Use spaces", "Spaces only", false);
    service::with(|store| store.approve(new.id, None, false, &[old.id], 30).unwrap()).unwrap();
    let g = grant(5, dir.path(), DECISIONS);
    let shown = call(&FULL, &g, "decisions_get", json!({"id": new.label()}));
    assert!(shown.contains("Spaces only"), "{shown}");
    assert!(shown.contains("because"), "the why: {shown}");
    assert!(
        shown.contains(&old.label()) && shown.contains("Tabs everywhere"),
        "history: {shown}"
    );
    let lower = call(
        &FULL,
        &g,
        "decisions_get",
        json!({"id": new.label().to_lowercase()}),
    );
    assert!(lower.contains("Spaces only"), "{lower}");
    let wrong_kind = format!("P-{}", new.id);
    let miss = call(&FULL, &g, "decisions_get", json!({"id": wrong_kind}));
    assert!(miss.contains("No visible entry"), "{miss}");
    let bad = FULL
        .call(&g, "decisions_get", json!({"id": "nope"}))
        .unwrap_err();
    assert!(bad.contains("D-14"), "{bad}");
}

#[test]
fn get_and_flag_cannot_reach_another_projects_entry() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let secret = seed(
        &scope(elsewhere.path()),
        Kind::Decision,
        "Private plan",
        "Hidden text",
        true,
    );
    let g = grant(5, dir.path(), DECISIONS);
    let got = call(&FULL, &g, "decisions_get", json!({"id": secret.label()}));
    assert!(!got.contains("Hidden text"), "{got}");
    let flagged = call(
        &FULL,
        &g,
        "decisions_flag",
        json!({"id": secret.label(), "reason": "x"}),
    );
    assert!(flagged.contains("No visible entry"), "{flagged}");
    assert!(service::with(|s| s.open_flags().unwrap())
        .unwrap()
        .is_empty());
}

#[test]
fn flag_records_reason_and_returns_ok_text() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let entry = seed(
        &scope(dir.path()),
        Kind::Decision,
        "Use tabs",
        "Tabs everywhere",
        true,
    );
    let g = grant(9, dir.path(), DECISIONS_READ);
    let said = call(
        &READ,
        &g,
        "decisions_flag",
        json!({"id": entry.label(), "reason": "Breaks the Makefile"}),
    );
    assert!(said.contains(&entry.label()), "{said}");
    let flags = service::with(|s| s.open_flags().unwrap()).unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].0.session, 9);
    assert_eq!(flags[0].0.reason, "Breaks the Makefile");
    let empty = READ
        .call(
            &g,
            "decisions_flag",
            json!({"id": entry.label(), "reason": " "}),
        )
        .unwrap_err();
    assert!(empty.contains("reason"), "{empty}");
}

#[test]
fn read_toolkit_has_no_propose() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(READ.name(), DECISIONS_READ);
    assert_eq!(FULL.name(), DECISIONS);
    let read = grant(7, dir.path(), DECISIONS_READ);
    assert_eq!(
        names(&READ, &read),
        [
            "decisions_list",
            "decisions_search",
            "decisions_get",
            "decisions_flag"
        ]
    );
    assert_eq!(
        names(&FULL, &grant(7, dir.path(), DECISIONS)),
        [
            "decisions_list",
            "decisions_search",
            "decisions_get",
            "decisions_propose",
            "decisions_flag"
        ]
    );
    let refused = READ
        .call(
            &read,
            "decisions_propose",
            json!({"title": "T", "text": "x", "why": "y"}),
        )
        .unwrap_err();
    assert!(
        refused.contains("read-only") || refused.contains("can't propose"),
        "{refused}"
    );
    assert!(READ.routes(&read, "decisions_propose"));
    assert!(pending().is_empty());
}

#[test]
fn read_tools_are_marked_read_only_and_propose_and_flag_are_not() {
    let dir = tempfile::tempdir().unwrap();
    let hints: Vec<(String, bool)> = FULL
        .tools(&grant(7, dir.path(), DECISIONS))
        .iter()
        .map(|tool| {
            (
                tool["name"].as_str().unwrap().to_owned(),
                tool["annotations"]["readOnlyHint"].as_bool().unwrap(),
            )
        })
        .collect();
    for (name, read_only) in hints {
        let expected = matches!(
            name.as_str(),
            "decisions_list" | "decisions_search" | "decisions_get"
        );
        assert_eq!(read_only, expected, "{name}");
    }
}

#[test]
fn the_hub_offers_decisions_only_when_granted() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed(
        &scope(root),
        Kind::Decision,
        "Use tabs",
        "Tabs everywhere",
        true,
    );
    let granted = tool_servers::test_connection(&Mount {
        session: 7201,
        worktree: root.to_path_buf(),
        toolkits: BTreeSet::from([DECISIONS_READ.to_owned()]),
        project: root.display().to_string(),
        ..Mount::default()
    });
    let listed = mcp_hub::call_tool(granted.clone(), "decisions_list", json!({})).unwrap();
    assert!(listed.as_str().unwrap().contains("Use tabs"), "{listed}");
    let refused = mcp_hub::call_tool(
        granted,
        "decisions_propose",
        json!({"title": "T", "text": "x", "why": "y"}),
    );
    assert!(refused.is_err(), "a worker cannot propose");
    assert!(pending().is_empty());

    let ungranted = tool_servers::test_connection(&Mount {
        session: 7202,
        worktree: root.to_path_buf(),
        toolkits: BTreeSet::new(),
        project: root.display().to_string(),
        ..Mount::default()
    });
    let none = mcp_hub::call_tool(ungranted, "decisions_list", json!({}));
    assert!(none.unwrap_err().contains("isn't available"));
}

#[test]
fn disabled_switch_makes_every_tool_return_an_off_message() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let g = grant(5, dir.path(), DECISIONS);
    service::set_enabled(false);
    for (tool, input) in [
        ("decisions_list", json!({})),
        ("decisions_search", json!({"query": "x"})),
        ("decisions_get", json!({"id": "D-1"})),
        (
            "decisions_propose",
            json!({"title": "T", "text": "x", "why": "y"}),
        ),
        ("decisions_flag", json!({"id": "D-1", "reason": "r"})),
    ] {
        assert_eq!(call(&FULL, &g, tool, input), OFF, "{tool}");
    }
    assert!(pending().is_empty(), "nothing was written while off");
    service::set_enabled(true);
}

#[test]
fn near_duplicate_proposal_updates_the_existing_pending_one() {
    let _guard = isolated();
    users_said(&[(5, &["one SQLite file"])]);
    let dir = tempfile::tempdir().unwrap();
    let g = grant(5, dir.path(), DECISIONS);
    call(
        &FULL,
        &g,
        "decisions_propose",
        json!({"title": "Store decisions in SQLite", "text": "Keep the decision log in one SQLite file.", "why": "first why", "user_answer": "one SQLite file"}),
    );
    let first = pending();
    assert_eq!(first.len(), 1);
    let said = call(
        &FULL,
        &g,
        "decisions_propose",
        json!({"title": "Store decisions in sqlite", "text": "Keep the decision log in one SQLite file, backed up.", "why": "second why", "user_answer": "one SQLite file"}),
    );
    let rows = pending();
    assert_eq!(rows.len(), 1, "no new row");
    assert_eq!(rows[0].id, first[0].id);
    assert_eq!(
        rows[0].text,
        "Keep the decision log in one SQLite file, backed up."
    );
    assert_eq!(rows[0].why, "second why");
    assert!(said.contains(&rows[0].label()), "{said}");
}

#[test]
fn an_unverified_repeat_cannot_rewrite_a_user_sourced_pending_entry() {
    let _guard = isolated();
    users_said(&[(5, &["one SQLite file"])]);
    let dir = tempfile::tempdir().unwrap();
    let g = grant(5, dir.path(), DECISIONS);
    call(
        &FULL,
        &g,
        "decisions_propose",
        json!({"title": "Store decisions in SQLite", "text": "Keep the log in one SQLite file", "why": "w", "user_answer": "one SQLite file"}),
    );
    let said = call(
        &FULL,
        &g,
        "decisions_propose",
        json!({"title": "Store decisions in SQLite", "text": "Keep the log in one SQLite file today", "why": "w", "user_answer": "never said this"}),
    );
    let rows = pending();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].text, "Keep the log in one SQLite file");
    assert!(said.contains(&rows[0].label()), "{said}");
}

#[test]
fn another_sessions_pending_proposal_is_reported_not_rewritten() {
    let _guard = isolated();
    users_said(&[(5, &["one SQLite file"]), (6, &["one SQLite file"])]);
    let dir = tempfile::tempdir().unwrap();
    call(
        &FULL,
        &grant(5, dir.path(), DECISIONS),
        "decisions_propose",
        json!({"title": "Store decisions in SQLite", "text": "Keep the log in one SQLite file", "why": "w", "user_answer": "one SQLite file"}),
    );
    let said = call(
        &FULL,
        &grant(6, dir.path(), DECISIONS),
        "decisions_propose",
        json!({"title": "Store decisions in SQLite", "text": "Keep the log in one SQLite file now", "why": "w", "user_answer": "one SQLite file"}),
    );
    let rows = pending();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].text, "Keep the log in one SQLite file");
    assert!(said.contains(&rows[0].label()), "{said}");
}

#[test]
fn different_titles_are_separate_proposals() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let g = grant(5, dir.path(), DECISIONS);
    for title in [
        "Store decisions in SQLite",
        "Retry network calls three times",
    ] {
        call(
            &FULL,
            &g,
            "decisions_propose",
            json!({"title": title, "text": "x", "why": "y"}),
        );
    }
    assert_eq!(pending().len(), 2);
}

#[test]
fn proposal_matching_an_active_decision_returns_it_instead() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let active = seed(
        &scope(dir.path()),
        Kind::Decision,
        "Store decisions in SQLite",
        "Keep the decision log in one SQLite file.",
        true,
    );
    let said = propose(5, dir.path(), "whatever");
    assert!(
        said.starts_with(&format!("already decided: {}", active.label())),
        "{said}"
    );
    assert!(pending().is_empty(), "no row was added");
}

#[test]
fn same_title_opposite_text_files_a_new_proposal() {
    let _guard = isolated();
    users_said(&[(5, &["spaces, not tabs"])]);
    let dir = tempfile::tempdir().unwrap();
    let active = seed(
        &scope(dir.path()),
        Kind::Decision,
        "Indentation style for all source files: tabs",
        "Indent every file with tabs.",
        true,
    );
    let g = grant(5, dir.path(), DECISIONS);
    let said = call(
        &FULL,
        &g,
        "decisions_propose",
        json!({"title": "Indentation style for all source files: spaces", "text": "Indent every file with spaces.", "why": "the user changed their mind", "user_answer": "spaces, not tabs"}),
    );
    assert!(!said.contains("already decided"), "{said}");
    let rows = pending();
    assert_eq!(rows.len(), 1, "filed for review");
    assert_eq!(rows[0].text, "Indent every file with spaces.");
    assert_ne!(rows[0].id, active.id);
    // The same again, now against its own pending proposal, rewrites nothing.
    call(
        &FULL,
        &g,
        "decisions_propose",
        json!({"title": "Indentation style for all source files: tabs", "text": "Indent every file with tabs.", "why": "w", "user_answer": "spaces, not tabs"}),
    );
    let rows = pending();
    assert!(rows
        .iter()
        .any(|r| r.text == "Indent every file with spaces."));
}

#[test]
fn a_verified_revision_replaces_the_old_attestation() {
    let _guard = isolated();
    users_said(&[(5, &["first answer", "second answer"])]);
    let dir = tempfile::tempdir().unwrap();
    let g = grant(5, dir.path(), DECISIONS);
    let send = |answer: &str, text: &str| {
        call(
            &FULL,
            &g,
            "decisions_propose",
            json!({"title": "Store decisions in SQLite", "text": text, "question": answer, "why": "w", "user_answer": answer}),
        )
    };
    send("first answer", "Keep the decision log in one SQLite file.");
    send(
        "second answer",
        "Keep the decision log in one SQLite file, backed up.",
    );
    let rows = pending();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].user_answer.as_deref(), Some("second answer"));
    assert_eq!(rows[0].source.user_turn.as_deref(), Some("second answer"));
    assert_eq!(
        rows[0].source.question_ref.as_deref(),
        Some("second answer")
    );
}

#[test]
fn an_answer_must_match_whole_words() {
    let _guard = isolated();
    users_said(&[(
        5,
        &["Yes, use Option for nullable fields.", "the user said so"],
    )]);
    let verifies = |answer: &str| {
        let dir = tempfile::tempdir().unwrap();
        !propose(5, dir.path(), answer).contains(UNVERIFIED)
    };
    assert!(verifies("Yes, use Option"), "a phrase ending at a word");
    assert!(
        verifies("Yes, use Option for nullable fields"),
        "before the full stop"
    );
    assert!(verifies("user said"));
    assert!(verifies("the user said so"), "the whole text");
    assert!(!verifies("ser said"), "starts mid-word");
    assert!(!verifies("nullable field"), "ends mid-word");
    assert!(!verifies("es, use Option"), "starts mid-word");
    users_said(&[(5, &["the user said so"])]);
    assert!(!verifies("use"), "\"use\" is not inside \"user\"");
}

#[test]
fn rejected_or_archived_entries_without_a_user_source_stay_with_their_proposer() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let s = scope(dir.path());
    let g5 = grant(5, dir.path(), DECISIONS);
    call(
        &FULL,
        &g5,
        "decisions_propose",
        json!({"title": "Odd idea", "text": "Do the odd thing", "why": "w"}),
    );
    let id = pending()[0].id;
    service::with(|store| store.reject(id, 30).unwrap()).unwrap();
    let mine = call(
        &FULL,
        &g5,
        "decisions_get",
        json!({"id": format!("D-{id}")}),
    );
    assert!(mine.contains("Do the odd thing"), "{mine}");
    let theirs = call(
        &FULL,
        &grant(6, dir.path(), DECISIONS),
        "decisions_get",
        json!({"id": format!("D-{id}")}),
    );
    assert!(theirs.contains("No visible entry"), "{theirs}");
    let _ = s;
}

#[test]
fn only_active_entries_can_be_flagged() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let s = scope(dir.path());
    let waiting = seed(&s, Kind::Decision, "Not yet", "Waiting for review", false);
    let g = grant(5, dir.path(), DECISIONS);
    let error = FULL
        .call(
            &g,
            "decisions_flag",
            json!({"id": waiting.label(), "reason": "r"}),
        )
        .unwrap_err();
    assert!(error.contains("active"), "{error}");
    assert!(service::with(|s| s.open_flags().unwrap())
        .unwrap()
        .is_empty());
}

#[test]
fn dedup_sees_entries_beyond_the_first_search_hits() {
    let _guard = isolated();
    let dir = tempfile::tempdir().unwrap();
    let s = scope(dir.path());
    for n in 0..30 {
        seed(
            &s,
            Kind::Decision,
            &format!("Storage note {n}"),
            "Storage note",
            true,
        );
    }
    let long = "storage note ".to_string() + &"filler ".repeat(40);
    let target = seed(&s, Kind::Decision, "Storage note", &long, true);
    let said = call(
        &FULL,
        &grant(5, dir.path(), DECISIONS),
        "decisions_propose",
        json!({"title": "Storage note", "text": long, "why": "w"}),
    );
    assert!(
        said.starts_with(&format!("already decided: {}", target.label())),
        "{said}"
    );
}
