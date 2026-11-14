//! The Pilot's History tab, as the board draws it: every session the Pilot
//! started, grouped by the pipeline run that started it (newest run first),
//! then the sessions it started outside any run. Each row shows the step it
//! ran for, how long it took and its diff size.
//!
//! Runs and diff sizes load off the main thread and are kept per Pilot
//! until a run moves on or the Pilot's Flows change.
use super::panels::{Route, empty_state, matches_query};
use super::*;
use gespan_core::flow::graph::{current_row, layout};
use gespan_core::flow::{Flow, FlowRun, FlowVisit, RunStatus, VisitStatus};
use gpui::{FontWeight, Hsla};

/// Runs shown, newest first.
const RUN_LIMIT: usize = 12;
const NOT_READY: &str = "Flows are not ready";

/// What the tab shows for one Pilot, as last loaded.
#[derive(Default)]
pub(super) struct History {
    /// The Pilot's runs, newest first, each with its visits.
    pub(super) runs: Vec<(FlowRun, Vec<FlowVisit>)>,
    /// Lines added and removed per session, from its newest checkpoint.
    pub(super) diffs: HashMap<i64, (i64, i64)>,
    pub(super) loaded: bool,
    pub(super) error: Option<String>,
    generation: u64,
}

/// One row: a session and what it ran for.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    pub session: i64,
    pub title: String,
    pub detail: String,
}

/// One group: a run's header and its sessions.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Group {
    pub key: String,
    pub label: String,
    pub chip: Option<(String, Chip)>,
    pub rows: Vec<Row>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Chip {
    Ok,
    Bad,
    Run,
    Wait,
    Muted,
}

/// The session id a visit ran in.
fn session_of(visit: &FlowVisit) -> Option<i64> {
    visit.session_id.as_deref()?.parse().ok()
}

/// "Nov 12", in local time.
fn day(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map_or_else(String::new, |t| t.format("%b %-d").to_string())
}

/// The step a visit's row names: a branch by its parallel step ("Build &
/// test"), any other step by its own title.
fn step_label(flow: &Flow, state: &str) -> String {
    flow.parent_of(state)
        .map(|p| p.title.clone())
        .or_else(|| flow.state(state).map(|s| s.title().to_owned()))
        .unwrap_or_else(|| state.to_owned())
}

/// The groups the tab draws for `runs`: one per run, its sessions ordered
/// as the pipeline reads (newest step first, a parallel step's branches in
/// their order). `title` names a session; `waiting` says whether one waits
/// for the user. Runs without a session are left out.
pub(super) fn groups(
    runs: &[(FlowRun, Vec<FlowVisit>)],
    title: impl Fn(i64) -> Option<String>,
    waiting: impl Fn(i64) -> bool,
    now: i64,
) -> Vec<Group> {
    let newest_active = runs
        .iter()
        .find(|(r, _)| r.status.is_active())
        .map(|(r, _)| r.id.clone());
    runs.iter()
        .filter_map(|(run, visits)| {
            let flow = &run.flow;
            let name = run
                .tag
                .as_ref()
                .map_or(flow.name.clone(), |t| t.task.clone());
            // A step's session is titled "<task> · <step>"; under the run's
            // heading the task goes without saying.
            let prefix = format!("{name} · ");
            let title = |id: i64| {
                title(id).map(|t| t.strip_prefix(&prefix).map(str::to_owned).unwrap_or(t))
            };
            // The seq a visit sorts under: its parallel step's for a branch.
            let group_seq = |v: &FlowVisit| {
                flow.parent_of(&v.state_id)
                    .and_then(|p| {
                        visits
                            .iter()
                            .filter(|s| s.state_id == p.id && s.seq < v.seq)
                            .map(|s| s.seq)
                            .max()
                    })
                    .unwrap_or(v.seq)
            };
            let branch_index = |v: &FlowVisit| {
                flow.parent_of(&v.state_id)
                    .and_then(|p| p.branches.iter().position(|b| *b == v.state_id))
                    .unwrap_or(0)
            };
            let mut ran: Vec<&FlowVisit> =
                visits.iter().filter(|v| session_of(v).is_some()).collect();
            // Each try of a retried branch ran in its own session: newest first.
            ran.sort_by_key(|v| {
                (
                    std::cmp::Reverse(group_seq(v)),
                    branch_index(v),
                    std::cmp::Reverse(v.seq),
                )
            });
            let rows: Vec<Row> = ran
                .into_iter()
                .filter_map(|v| {
                    let session = session_of(v)?;
                    let step = step_label(flow, &v.state_id);
                    let end = v.finished_at.unwrap_or(now);
                    let took = crate::shell::running_elapsed((end - v.started_at).max(0) as u64);
                    let outcome = match v.status {
                        VisitStatus::Running | VisitStatus::WaitingApproval if waiting(session) => {
                            Some("needs you")
                        }
                        VisitStatus::Running | VisitStatus::WaitingApproval => Some("running"),
                        VisitStatus::Failed => Some("failed"),
                        VisitStatus::Cancelled => Some("stopped"),
                        // A branch is a check run beside others: it passed.
                        VisitStatus::Completed if flow.parent_of(&v.state_id).is_some() => {
                            Some("passed")
                        }
                        VisitStatus::Completed => None,
                    };
                    let mut detail = format!("{step} · {took}");
                    if let Some(outcome) = outcome {
                        detail.push_str(" · ");
                        detail.push_str(outcome);
                    }
                    let fallback = flow
                        .state(&v.state_id)
                        .map_or(v.state_id.clone(), |s| s.title().to_owned());
                    Some(Row {
                        session,
                        title: title(session).unwrap_or(fallback),
                        detail,
                    })
                })
                .collect();
            if rows.is_empty() {
                return None;
            }
            let this = newest_active.as_deref() == Some(run.id.as_str());
            let label = if this {
                format!("This run · {name}")
            } else {
                format!("{name} · {}", day(run.created_at))
            };
            let any_waiting = visits
                .iter()
                .any(|v| v.status == VisitStatus::Running && session_of(v).is_some_and(&waiting));
            let step = current_row(&layout(flow), visits).map_or(1, |i| i + 1);
            let chip = match run.status {
                RunStatus::Succeeded => Some(("Done".to_string(), Chip::Ok)),
                RunStatus::Failed => Some(("Failed".to_string(), Chip::Bad)),
                RunStatus::Cancelled => Some(("Stopped".to_string(), Chip::Muted)),
                _ if any_waiting => Some((format!("Step {step}"), Chip::Wait)),
                _ => Some((format!("Step {step}"), Chip::Run)),
            };
            Some(Group {
                key: run.id.clone(),
                label,
                chip,
                rows,
            })
        })
        .collect()
}

/// The board's ink: muted text, faint text and the row icon.
fn ink(light_alpha: f32, dark: u32) -> Hsla {
    if crate::theme::is_light() {
        content(light_alpha)
    } else {
        rgb(dark).into()
    }
}

fn chip(label: String, tone: Chip) -> AnyElement {
    let (ink, base): (Hsla, Hsla) = match tone {
        Chip::Ok => (rgb(0x6ee7b7).into(), rgb(0x6ee7b7).into()),
        Chip::Bad => (rgb(0xfda4af).into(), rgb(0xfda4af).into()),
        Chip::Run => (rgb(0x8cc4ff).into(), rgb(0x459bf7).into()),
        Chip::Wait => (rgb(0xf5dc84).into(), rgb(0xe8c547).into()),
        Chip::Muted => (content(0.62), content(0.4)),
    };
    let border = if tone == Chip::Ok { 0.18 } else { 0.22 };
    div()
        .ml_auto()
        .flex_none()
        .h(px(22.))
        .pl(px(8.))
        .pr(px(9.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded_full()
        .bg(base.opacity(0.10))
        .border_1()
        .border_color(base.opacity(border))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(ink)
        .child(div().size(px(6.)).rounded_full().bg(ink))
        .child(label.to_uppercase())
        .into_any_element()
}

/// The board's `.side-sec`: an uppercase group heading.
fn heading(label: String, chip: Option<AnyElement>) -> AnyElement {
    div()
        .px(px(4.))
        .pt(px(14.))
        .pb(px(6.))
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(11.))
        .line_height(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ink(0.55, 0x8a8a8a))
        .child(div().min_w_0().truncate().child(label.to_uppercase()))
        .children(chip)
        .into_any_element()
}

/// The board's `.mini` diff pill: "+12 −3".
fn diff_pill(added: i64, removed: i64) -> AnyElement {
    div()
        .flex_none()
        .h(px(20.))
        .px(px(7.))
        .flex()
        .items_center()
        .rounded_full()
        .bg(content(0.05))
        .border_1()
        .border_color(content(0.09))
        .text_size(px(11.))
        .font_family(crate::fonts::monospace())
        .text_color(ink(0.64, 0xa3a3a3))
        .child(format!("+{added} \u{2212}{removed}"))
        .into_any_element()
}

impl Pilots {
    /// Reloads `pilot`'s History: its runs, then each session's diff size.
    pub(super) fn reload_history(&mut self, pilot: &str, cx: &mut Context<Self>) {
        let history = self.histories.entry(pilot.to_owned()).or_default();
        history.generation += 1;
        let generation = history.generation;
        let owner = self.owner.clone();
        let pilot = pilot.to_owned();
        let launched: Vec<(i64, String)> = self
            .roster
            .get(&pilot)
            .map(|p| {
                p.launches
                    .iter()
                    .map(|l| (l.session_id, l.cwd.clone()))
                    .collect()
            })
            .unwrap_or_default();
        cx.spawn(async move |this, cx| {
            // Read here: this may run inside a Shell update.
            let store = owner
                .update(cx, |shell, _| shell.pilot_flows_store())
                .unwrap_or_else(|_| Err(NOT_READY.into()));
            let id = pilot.clone();
            let result = match store {
                Ok(store) => {
                    cx.background_executor()
                        .spawn(async move {
                            let store = store
                                .lock()
                                .map_err(|_| "Flows are unavailable".to_string())?;
                            let mut runs = Vec::new();
                            for flow in store.owned(&id)? {
                                for run in store.runs(&flow.id, RUN_LIMIT)? {
                                    if run.flow.owner.as_deref() == Some(id.as_str()) {
                                        let visits = store.visits(&run.id)?;
                                        runs.push((run, visits));
                                    }
                                }
                            }
                            runs.sort_by_key(|(r, _): &(FlowRun, _)| {
                                std::cmp::Reverse(r.created_at)
                            });
                            runs.truncate(RUN_LIMIT);
                            drop(store);
                            // Each session's newest recorded changes.
                            let mut sessions: Vec<(i64, String)> = launched;
                            for (run, visits) in &runs {
                                let cwd = run.worktree_cwd.clone().unwrap_or(run.flow.cwd.clone());
                                sessions.extend(
                                    visits
                                        .iter()
                                        .filter_map(|v| Some((session_of(v)?, cwd.clone()))),
                                );
                            }
                            let diffs = diff_sizes(&sessions);
                            Ok::<_, String>((runs, diffs))
                        })
                        .await
                }
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |this, cx| {
                let history = this.histories.entry(pilot).or_default();
                if history.generation != generation {
                    return;
                }
                history.loaded = true;
                match result {
                    Ok((runs, diffs)) => {
                        history.runs = runs;
                        history.diffs = diffs;
                        history.error = None;
                    }
                    Err(error) => history.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `pilot`'s History as drawn: loaded on first use and reloaded once
    /// stale.
    fn shown_history(&mut self, pilot: &str, cx: &mut Context<Self>) {
        let fresh = self.histories.contains_key(pilot);
        if !fresh || self.stale_histories.remove(pilot) {
            self.reload_history(pilot, cx);
        }
    }

    pub(super) fn history_list(&mut self, pilot: &Pilot, cx: &mut Context<Self>) -> AnyElement {
        self.shown_history(&pilot.id, cx);
        let filter = self.field(
            "pilot-session-search",
            "Search runs and sessions",
            String::new(),
            false,
            cx,
        );
        let query = filter.read(cx).content.to_string();
        let views = self
            .owner
            .upgrade()
            .map(|owner| owner.read(cx).pilot_session_views(cx))
            .unwrap_or_default();
        let view = |id: i64| views.iter().find(|v| v.id == id);
        let now = gespan_core::time::now_ms() as i64;
        let history = self.histories.get(&pilot.id);
        let empty = Vec::new();
        let runs = history.map_or(&empty, |h| &h.runs);
        let mut groups = groups(
            runs,
            |id| {
                view(id)
                    .map(|v| v.title.clone())
                    .filter(|t| !t.trim().is_empty())
            },
            |id| view(id).is_some_and(|v| v.needs_input),
            now,
        );
        // Sessions the Pilot started outside any run, newest first.
        let in_runs: HashSet<i64> = groups
            .iter()
            .flat_map(|g| g.rows.iter().map(|r| r.session))
            .collect();
        let others: Vec<Row> = pilot
            .launches
            .iter()
            .rev()
            .filter(|l| !in_runs.contains(&l.session_id))
            .map(|l| {
                let state = view(l.session_id);
                let cwd = state.map_or(l.cwd.as_str(), |s| s.cwd.as_str());
                let project = std::path::Path::new(cwd)
                    .file_name()
                    .map_or(cwd.to_owned(), |n| n.to_string_lossy().to_string());
                let when = match state {
                    Some(s) if s.needs_input => "needs you".to_owned(),
                    Some(s) if s.busy => "running".to_owned(),
                    Some(s) => gespan_core::time::relative(s.updated_at, now as u64),
                    None => "unavailable".to_owned(),
                };
                Row {
                    session: l.session_id,
                    title: state.map_or(l.title.clone(), |s| s.title.clone()),
                    detail: format!("{project} · {when}"),
                }
            })
            .collect();
        if !others.is_empty() {
            groups.push(Group {
                key: "sessions".into(),
                label: "Sessions".into(),
                chip: None,
                rows: others,
            });
        }
        for group in &mut groups {
            if !matches_query(&query, &group.label) {
                group
                    .rows
                    .retain(|r| matches_query(&query, &format!("{} {}", r.title, r.detail)));
            }
        }
        let any = groups.iter().any(|g| !g.rows.is_empty());
        let started = !pilot.launches.is_empty() || !runs.is_empty();
        let diffs = history.map(|h| h.diffs.clone()).unwrap_or_default();
        let error = history.and_then(|h| h.error.clone());
        let selected = match self.route {
            Some(Route::Session(id)) => Some(id),
            _ => None,
        };
        let search = div()
            .debug_selector(|| "pilot-history-search".into())
            .mt(px(12.))
            .mb(px(2.))
            .h(px(32.))
            .px(px(10.))
            .flex_shrink_0()
            .rounded(px(9.))
            .bg(gpui::black().opacity(0.25))
            .border_1()
            .border_color(crate::glass::edge())
            .flex()
            .items_center()
            .gap(px(8.))
            .child(icon("search", 14.).text_color(ink(0.55, 0x8a8a8a)))
            .child(div().flex_1().min_w_0().child(filter));
        div()
            .size_full()
            .px(px(12.))
            .flex()
            .flex_col()
            .child(search)
            .child(
                div()
                    .id("pilot-session-list")
                    .debug_selector(|| "pilot-session-list".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .when_some(error, |d, error| {
                        d.child(
                            div()
                                .px(px(4.))
                                .pt(px(10.))
                                .text_size(px(11.))
                                .text_color(rgb(0xf87171))
                                .child(error),
                        )
                    })
                    .when(!started, |d| {
                        d.child(empty_state(
                            "message-multiple",
                            &format!(
                                "Ask {} to start a session on one of your projects, like a bug fix or a review, and it shows up here.",
                                pilot.label()
                            ),
                        ))
                    })
                    .when(started && !any, |d| {
                        d.child(empty_state("search-01", "No runs or sessions match this search."))
                    })
                    .children(groups.into_iter().filter(|g| !g.rows.is_empty()).map(|group| {
                        let key = group.key.clone();
                        div()
                            .debug_selector(move || format!("pilot-history-group-{key}"))
                            .flex()
                            .flex_col()
                            .child(heading(
                                group.label,
                                group.chip.map(|(label, tone)| chip(label, tone)),
                            ))
                            .children(group.rows.into_iter().map(|row| {
                                let id = row.session;
                                let (added, removed) = diffs.get(&id).copied().unwrap_or((0, 0));
                                let on = selected == Some(id);
                                div()
                                    .id(("pilot-launch", id as usize))
                                    .debug_selector(move || format!("pilot-launch-{id}"))
                                    .role(Role::Button)
                                    .aria_label(SharedString::from(format!(
                                        "{}, {}",
                                        row.title, row.detail
                                    )))
                                    .cursor_pointer()
                                    .flex()
                                    .items_start()
                                    .gap(px(10.))
                                    .p(px(10.))
                                    .rounded(px(10.))
                                    .when(on, |d| {
                                        d.bg(content(0.10)).shadow(vec![gpui::BoxShadow::new(
                                            px(0.),
                                            px(1.),
                                            content(0.08),
                                        )
                                        .inset()])
                                    })
                                    .when(!on, |d| d.hover(|s| s.bg(content(0.06))))
                                    .child(
                                        div().pt(px(2.)).child(
                                            icon("task-terminal", 14.).text_color(ink(0.7, 0xcfcfcf)),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap(px(3.))
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(13.))
                                                    .line_height(px(18.))
                                                    .text_color(content(0.92))
                                                    .child(row.title),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(11.5))
                                                    .line_height(px(16.))
                                                    .text_color(ink(0.55, 0x8a8a8a))
                                                    .child(row.detail),
                                            ),
                                    )
                                    .child(diff_pill(added, removed))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.route = Some(Route::Session(id));
                                        cx.notify();
                                    }))
                            }))
                    })),
            )
            .into_any_element()
    }
}

/// Lines added and removed per session, from its newest checkpoint.
fn diff_sizes(sessions: &[(i64, String)]) -> HashMap<i64, (i64, i64)> {
    let Some(store) = gespan_core::checkpoint::store() else {
        return HashMap::new();
    };
    sessions
        .iter()
        .filter_map(|(id, cwd)| {
            let status = store
                .exclusive(|s| s.status_readonly(&id.to_string(), cwd))
                .ok()?;
            let added = status.files.iter().map(|f| f.additions).sum();
            let removed = status.files.iter().map(|f| f.deletions).sum();
            Some((*id, (added, removed)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::flow::{AgentState, FlowState, Join, ParallelState, RunTag, Transition};

    fn agent(id: &str, title: &str, to: &[&str]) -> FlowState {
        FlowState::Agent(AgentState {
            id: id.into(),
            title: title.into(),
            harness: "claude".into(),
            model: "opus".into(),
            brief: "do it".into(),
            outputs: vec![],
            transitions: to
                .iter()
                .map(|t| Transition {
                    to: t.to_string(),
                    when: None,
                })
                .collect(),
            max_visits: 3,
            when: None,
        })
    }

    fn flow() -> Flow {
        Flow {
            id: "release".into(),
            name: "Release".into(),
            description: String::new(),
            cwd: "/w".into(),
            inputs: vec![],
            initial: "plan".into(),
            states: vec![
                agent("plan", "Plan", &["build"]),
                FlowState::Parallel(ParallelState {
                    id: "build".into(),
                    title: "Build & test".into(),
                    branches: vec!["mac".into(), "win".into()],
                    join: Join::All,
                    transitions: vec![Transition {
                        to: "publish".into(),
                        when: None,
                    }],
                    max_visits: 1,
                }),
                agent("mac", "macOS build", &[]),
                agent("win", "Windows build", &[]),
                agent("publish", "Publish", &["$succeeded"]),
            ],
            access: "auto".into(),
            owner: Some("p1".into()),
            created_at: 0,
            updated_at: 0,
        }
    }

    const MIN: i64 = 60_000;

    fn visit(
        seq: u32,
        state: &str,
        status: VisitStatus,
        session: Option<i64>,
        minutes: (i64, i64),
    ) -> FlowVisit {
        FlowVisit {
            id: format!("v{seq}"),
            run_id: "r".into(),
            seq,
            state_id: state.into(),
            status,
            prompt: None,
            session_id: session.map(|s| s.to_string()),
            outputs: None,
            error: None,
            retried: false,
            note: None,
            started_at: minutes.0 * MIN,
            finished_at: (status != VisitStatus::Running).then_some(minutes.1 * MIN),
        }
    }

    fn run(id: &str, task: &str, status: RunStatus, created_at: i64) -> FlowRun {
        FlowRun {
            id: id.into(),
            flow_id: "release".into(),
            flow: flow(),
            status,
            inputs: HashMap::new(),
            worktree_cwd: None,
            current_state: None,
            error: None,
            created_at,
            finished_at: None,
            tag: Some(RunTag {
                goal: "Ship it".into(),
                task: task.into(),
            }),
            edits: vec![],
        }
    }

    #[test]
    fn sessions_group_under_their_run_in_pipeline_order() {
        let going = run("r2", "Release 0.2.1", RunStatus::Running, 10 * MIN);
        let visits = vec![
            visit(1, "plan", VisitStatus::Completed, Some(1), (0, 4)),
            visit(2, "build", VisitStatus::Running, None, (4, 4)),
            visit(3, "mac", VisitStatus::Completed, Some(2), (4, 16)),
            visit(4, "win", VisitStatus::Running, Some(3), (4, 4)),
        ];
        let titles = HashMap::from([
            (1, "Write the release plan".to_string()),
            // As the runner titles a step's session.
            (2, "Release 0.2.1 · macOS build".to_string()),
        ]);
        let groups = groups(
            &[(going, visits)],
            |id| titles.get(&id).cloned(),
            |id| id == 3,
            20 * MIN,
        );
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group.label, "This run · Release 0.2.1");
        assert_eq!(
            group.chip,
            Some(("Step 2".into(), Chip::Wait)),
            "a branch waits on you"
        );
        let rows: Vec<(&str, &str)> = group
            .rows
            .iter()
            .map(|r| (r.title.as_str(), r.detail.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("macOS build", "Build & test · 12m · passed"),
                ("Windows build", "Build & test · 16m · needs you"),
                ("Write the release plan", "Plan · 4m"),
            ],
            "the branches, in their order, under their step, then the plan"
        );
    }

    #[test]
    fn a_finished_run_is_dated_and_each_try_of_a_branch_is_a_row() {
        let done = run("r1", "Release 0.2.0", RunStatus::Succeeded, 0);
        let visits = vec![
            visit(1, "build", VisitStatus::Completed, None, (0, 30)),
            visit(2, "mac", VisitStatus::Failed, Some(5), (0, 10)),
            visit(3, "win", VisitStatus::Completed, Some(6), (0, 20)),
            visit(4, "mac", VisitStatus::Completed, Some(7), (10, 31)),
        ];
        let groups = groups(&[(done, visits)], |_| None, |_| false, 0);
        let group = &groups[0];
        assert!(
            group.label.starts_with("Release 0.2.0 · "),
            "{}",
            group.label
        );
        assert_eq!(group.chip, Some(("Done".into(), Chip::Ok)));
        let sessions: Vec<i64> = group.rows.iter().map(|r| r.session).collect();
        // Both tries had their own session: each is a row, newest try first.
        assert_eq!(sessions, [7, 5, 6]);
        assert_eq!(group.rows[1].detail, "Build & test · 10m · failed");
        assert_eq!(
            group.rows[0].title, "macOS build",
            "named by its step without a session title"
        );
    }

    #[test]
    fn a_run_without_sessions_is_left_out() {
        let queued = run("r3", "Release 0.2.2", RunStatus::Running, 0);
        let visits = vec![visit(1, "plan", VisitStatus::Running, None, (0, 0))];
        assert!(groups(&[(queued, visits)], |_| None, |_| false, 0).is_empty());
    }
}
