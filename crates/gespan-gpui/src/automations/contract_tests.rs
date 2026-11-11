//! Every trigger the editor or a template offers can actually start a run:
//! an option nothing can fire is an automation that silently never runs.
use super::*;
use gespan_core::automations::{Service, claim_inbox_events};
use gespan_core::inbox::Item;

const EVENT_SOURCES: [&str; 5] = ["github", "gitlab", "linear", "jira", "azuredevops"];

fn now() -> i64 {
    gespan_core::time::now_ms() as i64
}

/// One item of every kind the Inbox lists, created at `at`.
fn every_kind_of_item(at: i64) -> Vec<Item> {
    let created = chrono::DateTime::from_timestamp_millis(at)
        .unwrap()
        .to_rfc3339();
    [
        ("github", "pr", false),
        ("github", "pr", true),
        ("github", "issue", false),
        ("gitlab", "pr", false),
        ("gitlab", "issue", false),
        ("azuredevops", "pr", false),
        ("azuredevops", "issue", false),
        ("linear", "issue", false),
        ("jira", "issue", false),
    ]
    .into_iter()
    .enumerate()
    .map(|(number, (provider, kind, draft))| Item {
        provider: provider.into(),
        kind: kind.into(),
        draft,
        number: number as i64 + 1,
        id: format!("{provider}-{number}"),
        repo: "acme/app".into(),
        project_path: "/repo".into(),
        title: format!("Item {number}"),
        created_at: created.clone(),
        ..Default::default()
    })
    // What the background refresh finds on listed items: comments on
    // both forges, and GitHub's checks, reviews, merges and assignments.
    .chain({
        use gespan_core::inbox::{Check, Checks, Comment, events};
        let on = |provider: &str, kind: &str, number: i64| Item {
            provider: provider.into(),
            kind: kind.into(),
            number,
            repo: "acme/app".into(),
            project_path: "/repo".into(),
            title: format!("Item {number}"),
            updated_at: created.clone(),
            ..Default::default()
        };
        let said = Comment {
            id: "501".into(),
            kind: "comment".into(),
            author: "alice".into(),
            permission: "write".into(),
            body: "Please rename this".into(),
            created_at: created.clone(),
            ..Default::default()
        };
        let failing = Checks {
            head_oid: "abc1234".into(),
            checks: vec![Check {
                name: "test".into(),
                state: "fail".into(),
                completed_at: Some(created.clone()),
                ..Default::default()
            }],
        };
        let mut found =
            events::comment_items(&on("gitlab", "pr", 90), std::slice::from_ref(&said), "");
        found.extend(events::comment_items(&on("github", "pr", 91), &[said], ""));
        found.extend(events::check_failed_items(
            &on("github", "pr", 92),
            &failing,
        ));
        found.push(events::review_requested_item(
            &on("github", "pr", 93),
            "abc1234",
        ));
        found.push(events::merged_item(
            &on("github", "pr", 94),
            &created,
            "alice",
        ));
        found.push(events::assigned_item(&on("github", "issue", 95), "me"));
        found
    })
    .collect()
}

fn saved(
    service: &mut Service,
    trigger: AutomationTrigger,
) -> gespan_core::automations::Automation {
    let mut draft = AutomationUpsert::new("/repo".into(), "codex".into(), String::new());
    draft.name = "Contract".into();
    draft.prompt = "Do the work".into();
    draft.triggers = Some(vec![trigger]);
    draft.sync_triggers(now());
    service.upsert(draft).unwrap()
}

#[test]
fn every_event_the_editor_offers_can_start_a_run() {
    for kind in EVENT_SOURCES {
        let offered = editor::events(kind);
        assert!(!offered.is_empty(), "{kind} offers no events");
        for (event, label) in offered {
            let directory = tempfile::tempdir().unwrap();
            let mut service = Service::open(&directory.path().join("db")).unwrap();
            let automation = saved(&mut service, AutomationTrigger::new(kind, &event));
            let items = every_kind_of_item(automation.events_since.unwrap());
            let claims = claim_inbox_events(&mut service, &items, now()).unwrap();
            assert_eq!(claims.len(), 1, "{kind} {event} ({label}) can never run");
        }
    }
}

#[test]
fn every_schedule_the_editor_offers_comes_due_within_a_week_and_runs() {
    for (schedule, label) in editor::events("time") {
        let directory = tempfile::tempdir().unwrap();
        let mut service = Service::open(&directory.path().join("db")).unwrap();
        let automation = saved(&mut service, AutomationTrigger::new("time", &schedule));
        let next = automation.next_run_at;
        assert!(next > now(), "{label}");
        assert!(next <= now() + 7 * 86_400_000, "{label} is never due");
        assert!(service.claim_due(next - 1).unwrap().is_empty(), "{label}");
        let due = service.claim_due(next).unwrap();
        assert_eq!(due.len(), 1, "{label}");
        assert_eq!(due[0].run.status, "pending", "{label}");
    }
}

#[test]
fn every_template_saves_and_its_trigger_can_start_a_run() {
    let templates: Vec<Template> =
        serde_json::from_str(include_str!("templates.json")).expect("templates parse");
    assert!(!templates.is_empty());
    for template in templates {
        let kind = template.trigger["kind"].as_str().unwrap_or("time");
        let event = template.trigger["event"].as_str().unwrap_or("weekdays");
        let mut trigger = AutomationTrigger::new(kind, event);
        if let Some(value) = template.trigger["time"].as_str() {
            trigger.time = value.into();
        }
        if let Some(value) = template.trigger["dayOfWeek"].as_i64() {
            trigger.day_of_week = value;
        }
        if let Some(value) = template.trigger["minute"].as_i64() {
            trigger.minute = value;
        }
        let directory = tempfile::tempdir().unwrap();
        let mut service = Service::open(&directory.path().join("db")).unwrap();
        let mut draft = AutomationUpsert::new("/repo".into(), "codex".into(), String::new());
        draft.name = template.name.clone();
        draft.prompt = template.prompt.clone();
        draft.triggers = Some(vec![trigger]);
        draft.sync_triggers(now());
        let automation = service
            .upsert(draft)
            .unwrap_or_else(|error| panic!("{}: {error}", template.id));
        if kind == "time" {
            let due = service.claim_due(automation.next_run_at).unwrap();
            assert_eq!(due.len(), 1, "{} never comes due", template.id);
        } else {
            let items = every_kind_of_item(automation.events_since.unwrap());
            let claims = claim_inbox_events(&mut service, &items, now()).unwrap();
            assert_eq!(claims.len(), 1, "{} can never run", template.id);
        }
    }
}
