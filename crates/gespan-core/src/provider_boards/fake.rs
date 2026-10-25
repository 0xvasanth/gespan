//! In-memory `BoardProvider` for sync-engine tests. Writes advance a logical
//! clock by one second, so `updated_at` order is deterministic.
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard};

use serde::Deserialize;

use super::*;

const SYNC_BATCH: usize = 50;
const CLOCK_START: i64 = 1_767_225_600; // 2026-01-01T00:00:00Z

pub(crate) struct FakeProvider {
    inner: Mutex<State>,
}

struct State {
    containers: Vec<(Container, Vec<WorkflowState>)>,
    labels: HashMap<String, Vec<BoardLabel>>,
    members: HashMap<String, Vec<Member>>,
    issues: BTreeMap<String, RemoteIssue>,
    comments: Vec<(String, CommentRef, String)>,
    required: HashMap<String, Vec<String>>,
    failures: HashMap<&'static str, ProviderError>,
    clock: i64,
    next_id: u64,
}

impl State {
    fn container(&self, id: &str) -> ProviderResult<&(Container, Vec<WorkflowState>)> {
        self.containers
            .iter()
            .find(|(container, _)| container.id == id)
            .ok_or_else(|| ProviderError::InvalidInput(format!("Unknown container {id}")))
    }

    fn issue_mut(&mut self, id: &str) -> ProviderResult<&mut RemoteIssue> {
        self.issues
            .get_mut(id)
            .ok_or_else(|| ProviderError::InvalidInput(format!("Unknown issue {id}")))
    }

    fn tick(&mut self) -> String {
        self.clock += 1;
        timestamp(self.clock)
    }

    fn fresh_id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }
}

fn timestamp(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .expect("valid clock")
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Labels and assignees aren't modelled; refusing beats silently dropping them.
fn reject_people_and_labels(labels: bool, assignees: bool) -> ProviderResult<()> {
    if labels || assignees {
        return Err(ProviderError::InvalidInput(
            "The fake provider does not model labels or assignees".into(),
        ));
    }
    Ok(())
}

/// Orders by the instant, not the text: `…00.5Z`, `…00Z` and `+00:00` forms
/// don't sort correctly as strings.
fn sync_key(issue: &RemoteIssue) -> (i64, String) {
    (instant_ms(&issue.updated_at), issue.id.clone())
}

fn instant_ms(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|at| at.timestamp_millis())
        .unwrap_or(0)
}

fn encode_cursor((ms, id): &(i64, String)) -> SyncCursor {
    SyncCursor(format!("{ms}|{id}"))
}

/// `None` for the empty cursor a first sync of an empty container returns.
fn decode_cursor(cursor: &SyncCursor) -> ProviderResult<Option<(i64, String)>> {
    if cursor.0.is_empty() {
        return Ok(None);
    }
    cursor
        .0
        .split_once('|')
        .and_then(|(ms, id)| Some(Some((ms.parse().ok()?, id.to_string()))))
        .ok_or_else(|| ProviderError::InvalidInput(format!("Bad sync cursor {}", cursor.0)))
}

impl FakeProvider {
    pub(crate) fn new() -> Self {
        let state = |id: &str, name: &str, group| WorkflowState {
            id: id.into(),
            name: name.into(),
            group,
        };
        Self {
            inner: Mutex::new(State {
                containers: vec![(
                    Container {
                        id: "C1".into(),
                        key: "ONE".into(),
                        name: "One".into(),
                    },
                    vec![
                        state("todo", "Todo", StateGroup::Unstarted),
                        state("doing", "Doing", StateGroup::Started),
                        state("done", "Done", StateGroup::Completed),
                    ],
                )],
                labels: HashMap::new(),
                members: HashMap::new(),
                issues: BTreeMap::new(),
                comments: Vec::new(),
                required: HashMap::new(),
                failures: HashMap::new(),
                clock: CLOCK_START,
                next_id: 0,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Takes the lock, first returning any failure queued for `operation`.
    fn enter(&self, operation: &'static str) -> ProviderResult<MutexGuard<'_, State>> {
        let mut state = self.lock();
        match state.failures.remove(operation) {
            Some(error) => Err(error),
            None => Ok(state),
        }
    }

    pub(crate) fn with_container(self, container: Container, states: Vec<WorkflowState>) -> Self {
        self.lock().containers.push((container, states));
        self
    }

    /// Seeds an issue. The clock moves past its `updated_at`, so a later write
    /// is always newer than the seed.
    pub(crate) fn with_issue(self, issue: RemoteIssue) -> Self {
        let mut state = self.lock();
        state.clock = state
            .clock
            .max(instant_ms(&issue.updated_at).div_euclid(1000));
        state.issues.insert(issue.id.clone(), issue);
        drop(state);
        self
    }

    pub(crate) fn with_labels(self, container_id: &str, labels: Vec<BoardLabel>) -> Self {
        self.lock().labels.insert(container_id.into(), labels);
        self
    }

    pub(crate) fn with_members(self, container_id: &str, members: Vec<Member>) -> Self {
        self.lock().members.insert(container_id.into(), members);
        self
    }

    /// Moving into `state_id` fails unless these fields are supplied.
    pub(crate) fn require_fields(self, state_id: &str, fields: &[&str]) -> Self {
        self.lock().required.insert(
            state_id.into(),
            fields.iter().map(|field| field.to_string()).collect(),
        );
        self
    }

    /// The next call to `operation` returns `error`.
    pub(crate) fn fail_next(&self, operation: &'static str, error: ProviderError) {
        self.lock().failures.insert(operation, error);
    }

    pub(crate) fn comments(&self, issue_id: &str) -> Vec<(CommentRef, String)> {
        self.lock()
            .comments
            .iter()
            .filter(|(issue, _, _)| issue == issue_id)
            .map(|(_, comment, body)| (comment.clone(), body.clone()))
            .collect()
    }

    /// Any state of any container, by id.
    pub(crate) fn state(&self, state_id: &str) -> Option<WorkflowState> {
        self.lock()
            .containers
            .iter()
            .flat_map(|(_, states)| states)
            .find(|state| state.id == state_id)
            .cloned()
    }
}

impl BoardProvider for FakeProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Fake
    }

    fn whoami(&self) -> ProviderResult<Account> {
        drop(self.enter("whoami")?);
        Ok(Account {
            id: "me".into(),
            name: "Fake User".into(),
            email: "me@example.com".into(),
        })
    }

    fn list_containers(&self) -> ProviderResult<Vec<Container>> {
        let state = self.enter("list_containers")?;
        Ok(state.containers.iter().map(|(c, _)| c.clone()).collect())
    }

    fn list_states(&self, container_id: &str) -> ProviderResult<Vec<WorkflowState>> {
        let state = self.enter("list_states")?;
        Ok(state.container(container_id)?.1.clone())
    }

    fn list_labels(&self, container_id: &str) -> ProviderResult<Vec<BoardLabel>> {
        let state = self.enter("list_labels")?;
        state.container(container_id)?;
        Ok(state.labels.get(container_id).cloned().unwrap_or_default())
    }

    fn list_members(&self, container_id: &str) -> ProviderResult<Vec<Member>> {
        let state = self.enter("list_members")?;
        state.container(container_id)?;
        Ok(state.members.get(container_id).cloned().unwrap_or_default())
    }

    fn list_issues(
        &self,
        filter: &IssueFilter,
        cursor: Option<&str>,
    ) -> ProviderResult<Page<RemoteIssue>> {
        let state = self.enter("list_issues")?;
        let offset = match cursor {
            Some(raw) => raw
                .parse::<usize>()
                .map_err(|_| ProviderError::InvalidInput(format!("Bad cursor {raw}")))?,
            None => 0,
        };
        let mut matching: Vec<&RemoteIssue> = state
            .issues
            .values()
            .filter(|issue| {
                filter.container_ids.is_empty()
                    || filter.container_ids.contains(&issue.container_id)
            })
            .filter(|issue| filter.include_closed || !issue.state.group.is_closed())
            .collect();
        matching.sort_by(|a, b| {
            let (a_ms, b_ms) = (instant_ms(&a.updated_at), instant_ms(&b.updated_at));
            b_ms.cmp(&a_ms).then(a.id.cmp(&b.id))
        });
        let limit = filter.limit.max(1) as usize;
        let items: Vec<RemoteIssue> = matching
            .iter()
            .skip(offset)
            .take(limit)
            .map(|i| (*i).clone())
            .collect();
        let end = offset + items.len();
        Ok(Page {
            items,
            next_cursor: (end < matching.len()).then(|| end.to_string()),
        })
    }

    fn get_issue(&self, id: &str) -> ProviderResult<RemoteIssue> {
        let mut state = self.enter("get_issue")?;
        state.issue_mut(id).map(|issue| issue.clone())
    }

    fn create_issue(&self, container_id: &str, draft: &IssueDraft) -> ProviderResult<RemoteIssue> {
        let mut state = self.enter("create_issue")?;
        reject_people_and_labels(!draft.label_ids.is_empty(), !draft.assignee_ids.is_empty())?;
        let (container, states) = state.container(container_id)?.clone();
        let workflow_state = match &draft.state_id {
            Some(id) => states
                .iter()
                .find(|s| &s.id == id)
                .cloned()
                .ok_or_else(|| ProviderError::InvalidInput(format!("Unknown state {id}")))?,
            None => states.first().cloned().unwrap_or_default(),
        };
        let id = state.fresh_id("new-");
        let issue = RemoteIssue {
            key: format!("{}-{}", container.key, state.next_id),
            id: id.clone(),
            title: draft.title.clone(),
            container_id: container.id,
            updated_at: state.tick(),
            state: workflow_state,
            description: Some(draft.description.clone()),
            ..Default::default()
        };
        state.issues.insert(id, issue.clone());
        Ok(issue)
    }

    fn update_issue(&self, id: &str, patch: &IssuePatch) -> ProviderResult<RemoteIssue> {
        let mut state = self.enter("update_issue")?;
        reject_people_and_labels(patch.label_ids.is_some(), patch.assignee_ids.is_some())?;
        state.issue_mut(id)?;
        let now = state.tick();
        let issue = state.issue_mut(id)?;
        if let Some(title) = &patch.title {
            issue.title = title.clone();
        }
        if let Some(description) = &patch.description {
            issue.description = Some(description.clone());
        }
        issue.updated_at = now;
        Ok(issue.clone())
    }

    fn move_issue(&self, id: &str, target: &MoveTarget) -> ProviderResult<RemoteIssue> {
        let mut state = self.enter("move_issue")?;
        let container_id = state.issue_mut(id)?.container_id.clone();
        let next = state
            .container(&container_id)?
            .1
            .iter()
            .find(|s| s.id == target.state_id)
            .cloned()
            .ok_or_else(|| {
                ProviderError::InvalidInput(format!(
                    "State {} is not in container {container_id}",
                    target.state_id
                ))
            })?;
        if let Some(fields) = state.required.get(&target.state_id) {
            let missing: Vec<&str> = fields
                .iter()
                .filter(|field| !target.required_fields.contains_key(*field))
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                return Err(ProviderError::InvalidInput(format!(
                    "Missing required fields: {}",
                    missing.join(", ")
                )));
            }
        }
        let now = state.tick();
        let issue = state.issue_mut(id)?;
        issue.state = next;
        issue.updated_at = now;
        Ok(issue.clone())
    }

    fn comment(
        &self,
        issue_id: &str,
        body: &str,
        reply_to: Option<&str>,
    ) -> ProviderResult<CommentRef> {
        let mut state = self.enter("comment")?;
        state.issue_mut(issue_id)?;
        if body.trim().is_empty() {
            return Err(ProviderError::InvalidInput(
                "Comment cannot be empty".into(),
            ));
        }
        if let Some(parent) = reply_to {
            if !state
                .comments
                .iter()
                .any(|(issue, c, _)| issue == issue_id && c.id == parent)
            {
                return Err(ProviderError::InvalidInput(format!(
                    "Unknown comment {parent}"
                )));
            }
        }
        let id = state.fresh_id("comment-");
        let comment = CommentRef {
            url: format!("fake://{issue_id}#{id}"),
            id,
        };
        state
            .comments
            .push((issue_id.into(), comment.clone(), body.into()));
        Ok(comment)
    }

    fn sync_since(
        &self,
        container_id: &str,
        cursor: Option<&SyncCursor>,
    ) -> ProviderResult<SyncBatch> {
        let state = self.enter("sync_since")?;
        state.container(container_id)?;
        let after = cursor.map(decode_cursor).transpose()?.flatten();
        let mut changed: Vec<&RemoteIssue> = state
            .issues
            .values()
            .filter(|issue| issue.container_id == container_id)
            .filter(|issue| after.as_ref().is_none_or(|after| &sync_key(issue) > after))
            .collect();
        changed.sort_by_key(|issue| sync_key(issue));
        let has_more = changed.len() > SYNC_BATCH;
        let changes: Vec<RemoteIssue> = changed.into_iter().take(SYNC_BATCH).cloned().collect();
        let next_cursor = changes
            .last()
            .map(|issue| encode_cursor(&sync_key(issue)))
            .unwrap_or_else(|| cursor.cloned().unwrap_or_default());
        Ok(SyncBatch {
            changes,
            next_cursor,
            has_more,
        })
    }

    fn parse_webhook(
        &self,
        _headers: &[(String, String)],
        body: &[u8],
    ) -> ProviderResult<Vec<ChangeHint>> {
        drop(self.enter("parse_webhook")?);
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Delivery {
            One(ChangeHint),
            Many(Vec<ChangeHint>),
        }
        match serde_json::from_slice::<Delivery>(body) {
            Ok(Delivery::One(hint)) => Ok(vec![hint]),
            Ok(Delivery::Many(hints)) => Ok(hints),
            Err(error) => Err(ProviderError::InvalidInput(format!(
                "Unreadable webhook: {error}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::timestamp_ms;

    fn issue(id: &str, state: &str, updated_at: &str) -> RemoteIssue {
        let fake = FakeProvider::new();
        RemoteIssue {
            id: id.into(),
            key: id.to_uppercase(),
            title: format!("Issue {id}"),
            container_id: "C1".into(),
            updated_at: updated_at.into(),
            state: fake.state(state).expect("seed state"),
            ..Default::default()
        }
    }

    fn ids(issues: &[RemoteIssue]) -> Vec<&str> {
        issues.iter().map(|issue| issue.id.as_str()).collect()
    }

    #[test]
    fn connect_returns_the_fake_account() {
        let fake = FakeProvider::new();
        assert_eq!(fake.kind(), ProviderKind::Fake);
        assert_eq!(fake.connect().unwrap().id, "me");
        let groups: Vec<_> = fake
            .list_states("C1")
            .unwrap()
            .into_iter()
            .map(|state| (state.id, state.group))
            .collect();
        assert_eq!(
            groups,
            [
                ("todo".to_string(), StateGroup::Unstarted),
                ("doing".to_string(), StateGroup::Started),
                ("done".to_string(), StateGroup::Completed),
            ]
        );
        assert!(matches!(
            fake.list_states("nope"),
            Err(ProviderError::InvalidInput(_))
        ));
    }

    #[test]
    fn list_issues_filters_closed_and_pages_by_offset() {
        let fake = FakeProvider::new()
            .with_issue(issue("a", "todo", "2026-01-01T00:00:05Z"))
            .with_issue(issue("b", "done", "2026-01-01T00:00:04Z"))
            .with_issue(issue("c", "doing", "2026-01-01T00:00:03Z"))
            .with_issue(issue("d", "done", "2026-01-01T00:00:02Z"))
            .with_issue(issue("e", "todo", "2026-01-01T00:00:01Z"));
        let filter = IssueFilter {
            limit: 2,
            ..Default::default()
        };
        let first = fake.list_issues(&filter, None).unwrap();
        assert_eq!(ids(&first.items), ["a", "c"]);
        let second = fake
            .list_issues(&filter, first.next_cursor.as_deref())
            .unwrap();
        assert_eq!(ids(&second.items), ["e"]);
        assert_eq!(second.next_cursor, None);

        let all = IssueFilter {
            include_closed: true,
            limit: 10,
            ..Default::default()
        };
        assert_eq!(fake.list_issues(&all, None).unwrap().items.len(), 5);
        let elsewhere = IssueFilter {
            container_ids: vec!["C2".into()],
            include_closed: true,
            limit: 10,
            ..Default::default()
        };
        assert!(fake.list_issues(&elsewhere, None).unwrap().items.is_empty());
        assert!(matches!(
            fake.list_issues(&filter, Some("x")),
            Err(ProviderError::InvalidInput(_))
        ));
    }

    #[test]
    fn sync_since_returns_changes_in_update_order_and_resumes() {
        let fake = FakeProvider::new()
            .with_issue(issue("late", "todo", "2025-12-31T00:00:09Z"))
            .with_issue(issue("early", "todo", "2025-12-31T00:00:01Z"));
        let first = fake.sync_since("C1", None).unwrap();
        assert_eq!(ids(&first.changes), ["early", "late"]);
        assert!(!first.has_more);

        let quiet = fake.sync_since("C1", Some(&first.next_cursor)).unwrap();
        assert!(quiet.changes.is_empty());
        assert_eq!(quiet.next_cursor, first.next_cursor);

        let moved = fake
            .move_issue(
                "early",
                &MoveTarget {
                    state_id: "doing".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(moved.state.group, StateGroup::Started);
        assert!(moved.updated_at.as_str() > "2025-12-31T00:00:09Z");
        let next = fake.sync_since("C1", Some(&first.next_cursor)).unwrap();
        assert_eq!(ids(&next.changes), ["early"]);
        assert!(fake.sync_since("C2", None).is_err());
    }

    #[test]
    fn sync_since_breaks_timestamp_ties_by_id() {
        let mut fake = FakeProvider::new();
        for index in 0..51 {
            fake = fake.with_issue(issue(
                &format!("i{index:02}"),
                "todo",
                "2026-01-01T00:00:00Z",
            ));
        }
        let first = fake.sync_since("C1", None).unwrap();
        assert_eq!(first.changes.len(), 50);
        assert!(first.has_more);
        let second = fake.sync_since("C1", Some(&first.next_cursor)).unwrap();
        assert_eq!(ids(&second.changes), ["i50"]);
        assert!(!second.has_more);
    }

    #[test]
    fn writes_never_move_a_seeded_issue_back_in_time() {
        let fake = FakeProvider::new().with_issue(issue("a", "todo", "2026-03-01T10:00:00.500Z"));
        let first = fake.sync_since("C1", None).unwrap();
        let moved = fake
            .move_issue(
                "a",
                &MoveTarget {
                    state_id: "doing".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(
            timestamp_ms(&moved.updated_at) > timestamp_ms("2026-03-01T10:00:00.500Z"),
            "{}",
            moved.updated_at
        );
        let next = fake.sync_since("C1", Some(&first.next_cursor)).unwrap();
        assert_eq!(ids(&next.changes), ["a"]);
    }

    #[test]
    fn sync_orders_by_instant_not_by_timestamp_text() {
        // As text, ".500Z" sorts before "Z" and "+00:00" sorts before both.
        let fake = FakeProvider::new()
            .with_issue(issue("half", "todo", "2026-01-01T00:00:00.500Z"))
            .with_issue(issue("whole", "todo", "2026-01-01T00:00:00Z"))
            .with_issue(issue("offset", "todo", "2026-01-01T00:00:01+00:00"));
        let batch = fake.sync_since("C1", None).unwrap();
        assert_eq!(ids(&batch.changes), ["whole", "half", "offset"]);
    }

    #[test]
    fn move_rejects_foreign_state_and_missing_required_fields() {
        let other_state = WorkflowState {
            id: "elsewhere".into(),
            name: "Elsewhere".into(),
            group: StateGroup::Started,
        };
        let fake = FakeProvider::new()
            .with_container(
                Container {
                    id: "C2".into(),
                    key: "TWO".into(),
                    name: "Two".into(),
                },
                vec![other_state],
            )
            .with_issue(issue("a", "todo", "2026-01-01T00:00:00Z"))
            .require_fields("done", &["resolution"]);
        let to = |state: &str| MoveTarget {
            state_id: state.into(),
            ..Default::default()
        };
        assert!(matches!(
            fake.move_issue("a", &to("elsewhere")),
            Err(ProviderError::InvalidInput(_))
        ));
        assert!(matches!(
            fake.move_issue("a", &to("done")),
            Err(ProviderError::InvalidInput(message)) if message.contains("resolution")
        ));
        assert_eq!(fake.get_issue("a").unwrap().state.id, "todo");

        let mut target = to("done");
        target
            .required_fields
            .insert("resolution".into(), serde_json::json!("Fixed"));
        assert_eq!(fake.move_issue("a", &target).unwrap().state.id, "done");
        assert!(matches!(
            fake.move_issue("missing", &to("todo")),
            Err(ProviderError::InvalidInput(_))
        ));
    }

    #[test]
    fn create_and_update_bump_the_clock() {
        let fake = FakeProvider::new();
        let created = fake
            .create_issue(
                "C1",
                &IssueDraft {
                    title: "New".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(created.state.id, "todo");
        assert_eq!(created.container_id, "C1");
        let updated = fake
            .update_issue(
                &created.id,
                &IssuePatch {
                    title: Some("Renamed".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.title, "Renamed");
        assert!(updated.updated_at > created.updated_at);
        assert_eq!(fake.get_issue(&created.id).unwrap(), updated);
        assert!(fake.create_issue("C9", &IssueDraft::default()).is_err());
    }

    #[test]
    fn label_and_assignee_edits_are_rejected_not_dropped() {
        let fake = FakeProvider::new().with_issue(issue("a", "todo", "2026-01-01T00:00:00Z"));
        let patch = IssuePatch {
            label_ids: Some(vec!["l1".into()]),
            ..Default::default()
        };
        assert!(matches!(
            fake.update_issue("a", &patch),
            Err(ProviderError::InvalidInput(_))
        ));
        let draft = IssueDraft {
            title: "New".into(),
            assignee_ids: vec!["u1".into()],
            ..Default::default()
        };
        assert!(matches!(
            fake.create_issue("C1", &draft),
            Err(ProviderError::InvalidInput(_))
        ));
        assert_eq!(
            fake.get_issue("a").unwrap().updated_at,
            "2026-01-01T00:00:00Z"
        );
    }

    #[test]
    fn comment_threads_and_rejects_unknown_parent() {
        let fake = FakeProvider::new().with_issue(issue("a", "todo", "2026-01-01T00:00:00Z"));
        let root = fake.comment("a", "First", None).unwrap();
        let reply = fake.comment("a", "Reply", Some(&root.id)).unwrap();
        assert_ne!(root.id, reply.id);
        assert_eq!(
            fake.comments("a"),
            [
                (root.clone(), "First".to_string()),
                (reply, "Reply".to_string())
            ]
        );
        assert!(matches!(
            fake.comment("a", "Lost", Some("nope")),
            Err(ProviderError::InvalidInput(_))
        ));
        assert!(matches!(
            fake.comment("a", "  ", None),
            Err(ProviderError::InvalidInput(_))
        ));
        assert!(fake.comment("missing", "Hi", None).is_err());
    }

    #[test]
    fn parse_webhook_reads_single_and_batched_hints() {
        let fake = FakeProvider::new();
        let single = br#"{"entity":"issue","id":"a","event":"updated"}"#;
        assert_eq!(
            fake.parse_webhook(&[], single).unwrap(),
            [ChangeHint {
                entity: "issue".into(),
                id: "a".into(),
                event: ChangeEvent::Updated,
            }]
        );
        let batch = br#"[{"entity":"issue","id":"a","event":"created"},{"entity":"comment","id":"c","event":"deleted"}]"#;
        let hints = fake.parse_webhook(&[], batch).unwrap();
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[1].event, ChangeEvent::Deleted);
        assert!(matches!(
            fake.parse_webhook(&[], br#"{"entity":"issue","id":"a","event":"archived"}"#),
            Err(ProviderError::InvalidInput(_))
        ));
        assert!(matches!(
            fake.parse_webhook(&[], b"not json"),
            Err(ProviderError::InvalidInput(_))
        ));
    }

    #[test]
    fn fail_next_fails_once_then_recovers() {
        let fake = FakeProvider::new();
        fake.fail_next("sync_since", ProviderError::Remote("Rate limited".into()));
        assert_eq!(
            fake.sync_since("C1", None),
            Err(ProviderError::Remote("Rate limited".into()))
        );
        assert!(fake.sync_since("C1", None).is_ok());
        assert!(fake.whoami().is_ok());
    }

    #[test]
    fn labels_and_members_are_per_container() {
        let fake = FakeProvider::new()
            .with_labels(
                "C1",
                vec![BoardLabel {
                    id: "l1".into(),
                    name: "bug".into(),
                    color: "ff0000".into(),
                }],
            )
            .with_members(
                "C1",
                vec![Member {
                    id: "u1".into(),
                    name: "Ada".into(),
                    avatar_url: String::new(),
                }],
            );
        assert_eq!(fake.list_labels("C1").unwrap()[0].name, "bug");
        assert_eq!(fake.list_members("C1").unwrap()[0].name, "Ada");
        assert_eq!(fake.list_containers().unwrap()[0].id, "C1");
        assert!(fake.list_labels("C9").is_err());
    }

    #[test]
    fn empty_container_cursor_can_be_resumed() {
        let fake = FakeProvider::new();
        let first = fake.sync_since("C1", None).unwrap();
        assert!(first.changes.is_empty());
        let again = fake.sync_since("C1", Some(&first.next_cursor)).unwrap();
        assert!(again.changes.is_empty());
        let fake = fake.with_issue(issue("a", "todo", "2026-01-01T00:00:00Z"));
        let later = fake.sync_since("C1", Some(&again.next_cursor)).unwrap();
        assert_eq!(ids(&later.changes), ["a"]);
        assert!(matches!(
            fake.sync_since("C1", Some(&SyncCursor("garbage".into()))),
            Err(ProviderError::InvalidInput(_))
        ));
    }

    #[test]
    fn list_issues_orders_newest_first_by_instant() {
        let fake = FakeProvider::new()
            .with_issue(issue("whole", "todo", "2026-01-01T00:00:00Z"))
            .with_issue(issue("half", "todo", "2026-01-01T00:00:00.500Z"))
            .with_issue(issue("offset", "todo", "2026-01-01T00:00:01+00:00"));
        let page = fake
            .list_issues(
                &IssueFilter {
                    limit: 10,
                    ..Default::default()
                },
                None,
            )
            .unwrap();
        assert_eq!(ids(&page.items), ["offset", "half", "whole"]);
    }
}
