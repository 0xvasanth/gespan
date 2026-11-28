//! A repo's reviewer Pilot on the Pilots side: made when reviews are turned
//! on, read (never written) by review runs, and taught by hidden learning
//! runs that see only the user's and pushers' comments.
//!
//! The review itself runs as the automation's own session, so it keeps the
//! reviewer the user picked, the review policy and the run bookkeeping. It
//! is given the rules this Pilot learned in its prompt and holds no Pilot
//! capability: any app call from it is refused here.
use super::*;
use gespan_core::pilot::bridge::Call;
use gespan_core::pilot::code_review::{Lesson, learning_prompt};
use gespan_core::time::now_ms;

/// The only app actions a learning run may call.
pub(super) const LEARNING_ACTIONS: [&str; 5] = [
    "memory.read",
    "memory.search",
    "memory.add",
    "memory.replace",
    "memory.remove",
];
pub(super) const LEARNING_ONLY: &str =
    "A learning run can only read and change this reviewer's memory";
pub(crate) const REVIEW_CANT_WRITE: &str = "A code review can't use the reviewer's app actions or change its memory; say what should become a rule in your review instead";

/// One hidden run teaching a reviewer from a pull request's comments.
pub(crate) struct LearningRun {
    pub(super) pilot: String,
    pub(super) view: Entity<Conversation>,
    pub(super) started: u64,
    dispatched: bool,
    _changes: Subscription,
}

impl Pilots {
    /// The id of `repo`'s reviewer Pilot, made the first time, working in
    /// `project`.
    pub(crate) fn ensure_reviewer(
        &mut self,
        repo: &str,
        project: &str,
        cx: &mut Context<Self>,
    ) -> String {
        let before = self.roster.reviewer(repo).map(|p| p.projects.clone());
        let id = self.roster.ensure_reviewer(repo, project);
        if before.as_ref() != self.roster.get(&id).map(|p| &p.projects) {
            self.save(cx);
            self.changed(cx);
        }
        id
    }
    /// Where `repo`'s reviewer keeps its files, and its id.
    pub(crate) fn reviewer_files(&self, repo: &str) -> Option<(PathBuf, String)> {
        self.roster
            .reviewer(repo)
            .map(|pilot| (self.root.clone(), pilot.id.clone()))
    }
    /// A system review's session: it may never act as its reviewer.
    pub(crate) fn register_code_review(&mut self, session: i64) {
        self.code_reviews.insert(session);
    }
    /// Why `call` is refused for a review or learning run, if it is.
    pub(super) fn code_review_refusal(&self, call: &Call) -> Option<&'static str> {
        if self.code_reviews.contains(&call.session) {
            return Some(REVIEW_CANT_WRITE);
        }
        (self.learning.contains_key(&call.session)
            && !LEARNING_ACTIONS.contains(&call.action.as_str()))
        .then_some(LEARNING_ONLY)
    }
    /// Starts a hidden learning run for `lesson`'s comments the reviewer has
    /// not learned from yet. Each comment is learned from once.
    pub(crate) fn learn(&mut self, lesson: Lesson, cx: &mut Context<Self>) {
        let Some(pilot) = self.roster.reviewer(&lesson.repo) else {
            return;
        };
        let pilot_id = pilot.id.clone();
        let label = pilot.label();
        let Some(pilot) = self.roster.get_mut(&pilot_id) else {
            return;
        };
        let fresh: Vec<_> = lesson
            .comments
            .iter()
            .filter(|comment| pilot.learn_comment(&lesson.key(comment)))
            .cloned()
            .collect();
        if fresh.is_empty() {
            return;
        }
        self.save(cx);
        let prompt = learning_prompt(&lesson.repo, lesson.number, &fresh);
        let title = format!("{label} · learn · #{}", lesson.number);
        let pilot = pilot_id.clone();
        let spawned = self.spawn_hidden_run(
            &pilot_id,
            title,
            prompt,
            AccessMode::ReadOnly,
            move |this, session, view, cx| {
                let changes = cx.subscribe(view, move |this, _, _: &SessionChanged, cx| {
                    this.learning_changed(session, cx)
                });
                this.learning.insert(
                    session,
                    LearningRun {
                        pilot,
                        view: view.clone(),
                        started: now_ms(),
                        dispatched: false,
                        _changes: changes,
                    },
                );
            },
            cx,
        );
        cx.spawn(async move |this, cx| {
            let result = spawned.await;
            let _ = this.update(cx, |this, cx| match result {
                Err(error) => {
                    tracing::warn!(%error, "a reviewer could not start learning");
                }
                Ok((session, _, true)) => {
                    if let Some(run) = this.learning.get_mut(&session) {
                        run.dispatched = true;
                    }
                    this.learning_changed(session, cx);
                }
                Ok((session, _, false)) => {
                    this.learning.remove(&session);
                }
            });
        })
        .detach();
    }
    /// A learning run's turn ended: its capability goes with it.
    fn learning_changed(&mut self, session: i64, cx: &mut Context<Self>) {
        let Some(run) = self.learning.get(&session).filter(|r| r.dispatched) else {
            return;
        };
        if run.view.read(cx).busy {
            return;
        }
        if let Some(run) = self.learning.remove(&session) {
            self.reload_files(&run.pilot, cx);
        }
    }
    /// Learning runs still going after a habit's time box are stopped.
    pub(super) fn check_learning_timeouts(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        let late: Vec<i64> = self
            .learning
            .iter()
            .filter(|(_, r)| now.saturating_sub(r.started) > habits::TIMEOUT_MS)
            .map(|(&id, _)| id)
            .collect();
        for id in late {
            if let Some(run) = self.learning.remove(&id) {
                run.view.update(cx, |v, cx| v.pilot_stop(cx));
            }
        }
    }
}
