//! A repo's reviewer Pilot on the Pilots side: made when reviews are turned
//! on, read (never written) by review runs, and taught by hidden learning
//! runs that see only the user's and pushers' comments.
//!
//! The review itself runs as the automation's own session, so it keeps the
//! reviewer the user picked, the review policy and the run bookkeeping. It
//! is given the rules this Pilot learned in its prompt and holds no Pilot
//! capability: any app call from it is refused here.
use super::panels::Route;
use super::*;
use gespan_core::pilot::bridge::Call;
use gespan_core::pilot::code_review::{Lesson, learning_prompt};
use gespan_core::time::now_ms;

/// The only app actions a learning run may call. No `memory.remove`: a rule
/// a comment contradicts is replaced (kept, struck through), so a comment
/// can't wipe what the reviewer learned.
pub(super) const LEARNING_ACTIONS: [&str; 4] = [
    "memory.read",
    "memory.search",
    "memory.add",
    "memory.replace",
];
/// The actions above that change memory: only ever in the review topic.
const LEARNING_WRITES: [&str; 2] = ["memory.add", "memory.replace"];
pub(super) const LEARNING_ONLY: &str = "A learning run can only read this reviewer's memory and add or replace rules in its \"review\" topic";
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
    /// The id of `repo`'s reviewer Pilot, if one was made.
    pub(crate) fn reviewer_id(&self, repo: &str) -> Option<String> {
        self.roster.reviewer(repo).map(|pilot| pilot.id.clone())
    }
    /// A system review's session: it may never act as its reviewer. Kept for
    /// as long as the app runs, not just the run: a finished review session
    /// can be picked up and continued, and it must stay refused then too.
    /// One id per review is all it costs.
    pub(crate) fn register_code_review(&mut self, session: i64) {
        self.code_reviews.insert(session);
    }
    /// Why `call` is refused for a review or learning run, if it is.
    pub(super) fn code_review_refusal(&self, call: &Call) -> Option<&'static str> {
        if self.code_reviews.contains(&call.session) {
            return Some(REVIEW_CANT_WRITE);
        }
        if !self.learning.contains_key(&call.session) {
            return None;
        }
        let action = call.action.as_str();
        let topic = call.input.get("topic").and_then(|topic| topic.as_str());
        let allowed = LEARNING_ACTIONS.contains(&action)
            && (!LEARNING_WRITES.contains(&action)
                || topic == Some(gespan_core::pilot::code_review::REVIEW_TOPIC));
        (!allowed).then_some(LEARNING_ONLY)
    }
    /// Starts a hidden learning run for `lesson`'s comments the reviewer has
    /// not learned from yet. Each comment is learned from once.
    pub(crate) fn learn(&mut self, lesson: Lesson, cx: &mut Context<Self>) {
        let Some(pilot) = self.roster.reviewer(&lesson.repo) else {
            return;
        };
        let pilot_id = pilot.id.clone();
        let label = pilot.label();
        let Some(pilot) = self.roster.get(&pilot_id) else {
            return;
        };
        let fresh: Vec<_> = lesson
            .comments
            .iter()
            .filter(|comment| {
                let key = lesson.key(comment);
                !pilot.has_learned(&key) && !self.learning_pending.contains(&key)
            })
            .cloned()
            .collect();
        if fresh.is_empty() {
            return;
        }
        let keys: Vec<String> = fresh.iter().map(|comment| lesson.key(comment)).collect();
        self.learning_pending.extend(keys.iter().cloned());
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
        let learner = pilot_id.clone();
        cx.spawn(async move |this, cx| {
            let result = spawned.await;
            let _ = this.update(cx, |this, cx| {
                for key in &keys {
                    this.learning_pending.remove(key);
                }
                match result {
                    Err(error) => {
                        // Not marked learned: the next read of the comment tries again.
                        tracing::warn!(%error, "a reviewer could not start learning");
                    }
                    Ok((session, _, true)) => {
                        if let Some(pilot) = this.roster.get_mut(&learner) {
                            for key in &keys {
                                pilot.learn_comment(key);
                            }
                        }
                        this.save(cx);
                        if let Some(run) = this.learning.get_mut(&session) {
                            run.dispatched = true;
                        }
                        this.learning_changed(session, cx);
                    }
                    Ok((session, _, false)) => {
                        this.learning.remove(&session);
                    }
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

impl Pilots {
    /// A reviewer's Learned tab: each review rule it learned from the user's
    /// and pushers' corrections, newest last, with where it came from.
    pub(super) fn learned_rules(&self, pilot: &Pilot) -> AnyElement {
        let text = gespan_core::pilot::memory::read(
            &self.root,
            &pilot.id,
            Some(gespan_core::pilot::code_review::REVIEW_TOPIC),
        )
        .ok()
        .and_then(|file| file.text)
        .unwrap_or_default();
        let rules = learned_rule_lines(&text);
        let intro = div()
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(content(0.62))
            .child(
                "Learned from your comments, and those of people who can push, on pull requests it reviewed. \
                 Every review reads these; only a learning run changes them, never a review.",
            );
        let body = if rules.is_empty() {
            div()
                .debug_selector(|| "pilot-learned-empty".into())
                .text_size(px(12.5))
                .text_color(content(0.72))
                .child(
                    "No rules yet. Comment on a pull request it reviewed, saying what it got wrong or missed, and it learns.",
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(rules.into_iter().enumerate().map(|(ix, (rule, day))| {
                    div()
                        .debug_selector(move || format!("pilot-learned-rule-{ix}"))
                        .p(px(10.))
                        .rounded(px(10.))
                        .bg(content(0.04))
                        .border_1()
                        .border_color(crate::glass::edge())
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .line_height(px(18.))
                                .text_color(content(0.92))
                                .child(rule),
                        )
                        .when_some(day, |d, day| {
                            d.child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(content(0.5))
                                    .child(format!("Learned {day}")),
                            )
                        })
                }))
                .into_any_element()
        };
        div()
            .debug_selector(|| "pilot-learned".into())
            .id("pilot-learned")
            .size_full()
            .overflow_y_scroll()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(intro)
            .child(body)
            .into_any_element()
    }
}

/// The rules in a review topic's text: each live `- <day> · <rule>` entry,
/// leaving out struck-through (replaced) ones.
pub(super) fn learned_rule_lines(text: &str) -> Vec<(String, Option<String>)> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .filter(|entry| !entry.starts_with("~~"))
        .map(|entry| match entry.split_once(" · ") {
            Some((day, rule)) if day.len() == 10 && day.as_bytes()[4] == b'-' => {
                (rule.trim().to_string(), Some(day.to_string()))
            }
            _ => (entry.trim().to_string(), None),
        })
        .filter(|(rule, _)| !rule.is_empty())
        .collect()
}

/// One run of a repo's code review, for its reviewer's Reviews tab.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReviewRunRow {
    /// The pull request it reviewed, as the run named it.
    pub title: String,
    /// `queued`, `running`, `succeeded`, `failed` and so on.
    pub status: String,
    /// When it started (or was queued), in ms.
    pub at: i64,
    /// The session it ran in, once it has one.
    pub session: Option<String>,
    /// That session's handle here, so the Reviews tab opens it in place.
    pub native: Option<i64>,
}

impl ReviewRunRow {
    pub(crate) fn of(run: &gespan_core::automations::AutomationRun) -> Self {
        Self {
            title: run
                .event_title
                .clone()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| "Review".into()),
            status: run.status.replace('_', " "),
            at: run.started_at.unwrap_or(run.created_at),
            session: run.session_id.clone(),
            native: None,
        }
    }
}

impl Pilots {
    /// What the Shell read of each reviewed repo's runs.
    pub(crate) fn set_review_runs(
        &mut self,
        runs: HashMap<String, Vec<ReviewRunRow>>,
        cx: &mut Context<Self>,
    ) {
        if self.review_runs != runs {
            self.review_runs = runs;
            cx.notify();
        }
    }

    /// A reviewer's Reviews tab: each pull request its repo's review read,
    /// newest first; one opens its review session.
    pub(super) fn reviews_list(&self, pilot: &Pilot, cx: &mut Context<Self>) -> AnyElement {
        let runs = pilot
            .reviewed_repo()
            .and_then(|repo| self.review_runs.get(repo))
            .cloned()
            .unwrap_or_default();
        let body = if runs.is_empty() {
            div()
                .debug_selector(|| "pilot-reviews-empty".into())
                .text_size(px(12.5))
                .text_color(content(0.72))
                .child("No reviews yet. Press Review on a pull request, or turn the repo's review on in Settings › Git.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .children(runs.into_iter().enumerate().map(|(ix, run)| {
                    let tone = match run.status.as_str() {
                        "succeeded" => crate::glass::Tone::Ok,
                        "failed" => crate::glass::Tone::Bad,
                        "running" | "pending" | "queued" => crate::glass::Tone::Running,
                        _ => crate::glass::Tone::Muted,
                    };
                    let session = run.session.clone();
                    let native = run.native;
                    div()
                        .id(SharedString::from(format!("pilot-review-run-{ix}")))
                        .debug_selector(move || format!("pilot-review-run-{ix}"))
                        .role(Role::Button)
                        .aria_label(format!("Open the review of {}", run.title))
                        .cursor_pointer()
                        .p(px(8.))
                        .rounded(px(8.))
                        .hover(|s| s.bg(content(0.05)))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(crate::glass::status(run.status.clone(), tone).flex_none())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.5))
                                .text_color(content(0.9))
                                .child(run.title.clone()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            // The review opens inside its reviewer's page,
                            // like a History session; one not resolved yet
                            // opens in the workspace.
                            if let Some(id) = native {
                                this.route = Some(Route::Session(id));
                                cx.notify();
                            } else if let Some(id) = &session {
                                cx.emit(OpenRunSession(id.clone()));
                            }
                        }))
                }))
                .into_any_element()
        };
        div()
            .debug_selector(|| "pilot-reviews".into())
            .id("pilot-reviews")
            .size_full()
            .overflow_y_scroll()
            .p(px(12.))
            .child(body)
            .into_any_element()
    }
}
