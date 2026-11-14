//! The Pilot's Docs tab, as the board draws it: what the Pilot wrote, its
//! pinned documents (like the SOP) on top, then this run's (drafts while
//! the run is going), then everything earlier, newest first.
use super::panels::{Route, empty_state, matches_query};
use super::*;
use gespan_core::flow::{FlowRun, FlowState, FlowVisit};
use gpui::{FontWeight, Hsla};

/// One row: a document and the line under its title.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DocRow {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub badge: Option<(String, Badge)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Badge {
    Plain,
    Draft,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DocGroup {
    pub label: &'static str,
    pub rows: Vec<DocRow>,
}

/// "Today", or the day as the board writes it: "Nov 12".
fn when(ms: u64, now: u64) -> String {
    use chrono::TimeZone;
    let day = |ms: u64| {
        chrono::Local
            .timestamp_millis_opt(ms as i64)
            .single()
            .map(|t| t.date_naive())
    };
    match (day(ms), day(now)) {
        (Some(a), Some(b)) if a == b => "Today".into(),
        (Some(_), _) => chrono::Local
            .timestamp_millis_opt(ms as i64)
            .single()
            .map_or_else(String::new, |t| t.format("%b %-d").to_string()),
        _ => String::new(),
    }
}

/// The step of `run` that was running when `at` came: the newest such
/// visit's step, a branch by its own title.
fn step_at(run: &FlowRun, visits: &[FlowVisit], at: u64) -> Option<String> {
    let at = at as i64;
    visits
        .iter()
        .filter(|v| !matches!(run.flow.state(&v.state_id), Some(FlowState::Parallel(_))))
        .filter(|v| v.started_at <= at && v.finished_at.is_none_or(|end| end >= at))
        .max_by_key(|v| v.seq)
        .and_then(|v| run.flow.state(&v.state_id).map(|s| s.title().to_owned()))
}

/// The Docs tab's groups: Pinned, This run (the newest run still going),
/// Earlier. Each is newest first and left out when empty.
pub(super) fn doc_groups(
    documents: &[pilot::Document],
    runs: &[(FlowRun, Vec<FlowVisit>)],
    now: u64,
) -> Vec<DocGroup> {
    let going = runs.iter().find(|(r, _)| r.status.is_active());
    let mut sorted: Vec<&pilot::Document> = documents.iter().collect();
    sorted.sort_by_key(|d| std::cmp::Reverse(d.updated_at.max(d.created_at)));
    let (mut pinned, mut this, mut earlier) = (Vec::new(), Vec::new(), Vec::new());
    for d in sorted {
        let this_run = going.filter(|(r, _)| d.run_id.as_deref() == Some(r.id.as_str()));
        if d.pinned {
            pinned.push(DocRow {
                id: d.id.clone(),
                title: d.title.clone(),
                detail: format!("Updated {}", when(d.updated_at.max(d.created_at), now)),
                badge: Some(("Pinned".into(), Badge::Plain)),
            });
        } else if let Some((run, visits)) = this_run {
            let mut detail = when(d.created_at, now);
            if let Some(step) = step_at(run, visits, d.created_at) {
                detail.push_str(&format!(" · written in {step}"));
            }
            this.push(DocRow {
                id: d.id.clone(),
                title: d.title.clone(),
                detail,
                badge: Some(("Draft".into(), Badge::Draft)),
            });
        } else {
            earlier.push(DocRow {
                id: d.id.clone(),
                title: d.title.clone(),
                detail: when(d.updated_at.max(d.created_at), now),
                badge: (d.kind == "html").then(|| ("Visual".into(), Badge::Plain)),
            });
        }
    }
    [("Pinned", pinned), ("This run", this), ("Earlier", earlier)]
        .into_iter()
        .filter(|(_, rows)| !rows.is_empty())
        .map(|(label, rows)| DocGroup { label, rows })
        .collect()
}

fn ink(light_alpha: f32, dark: u32) -> Hsla {
    if crate::theme::is_light() {
        content(light_alpha)
    } else {
        rgb(dark).into()
    }
}

/// The board's `.mini` pill; a draft's word is yellow.
fn badge(label: String, kind: Badge) -> AnyElement {
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
        .text_color(match kind {
            Badge::Draft => rgb(0xf5dc84).into(),
            Badge::Plain => ink(0.64, 0xa3a3a3),
        })
        .child(label)
        .into_any_element()
}

impl Pilots {
    pub(super) fn document_list(&mut self, pilot: &Pilot, cx: &mut Context<Self>) -> AnyElement {
        // This run's documents need the Pilot's runs, which History loads.
        self.shown_history(&pilot.id, cx);
        let filter = self.field(
            "pilot-document-search",
            "Search docs",
            String::new(),
            false,
            cx,
        );
        let query = filter.read(cx).content.to_string();
        let now = gespan_core::time::now_ms();
        let empty = Vec::new();
        let runs = self.histories.get(&pilot.id).map_or(&empty, |h| &h.runs);
        let mut groups = doc_groups(&pilot.documents, runs, now);
        for group in &mut groups {
            group
                .rows
                .retain(|r| matches_query(&query, &format!("{} {}", r.title, r.detail)));
        }
        groups.retain(|g| !g.rows.is_empty());
        let selected = match &self.route {
            Some(Route::Document(id)) => Some(id.clone()),
            _ => None,
        };
        let pilot_name = pilot.label();
        let search = div()
            .debug_selector(|| "pilot-docs-search".into())
            .flex_1()
            .min_w_0()
            .h(px(32.))
            .px(px(10.))
            .rounded(px(9.))
            .bg(gpui::black().opacity(0.25))
            .border_1()
            .border_color(crate::glass::edge())
            .flex()
            .items_center()
            .gap(px(8.))
            .child(icon("search", 14.).text_color(ink(0.55, 0x8a8a8a)))
            .child(div().flex_1().min_w_0().child(filter));
        let new = crate::glass::button("pilot-docs-new", "+ New", crate::glass::Button::Glass)
            .debug_selector(|| "pilot-docs-new".into())
            .role(Role::Button)
            .aria_label(SharedString::from(format!(
                "Ask {pilot_name} to write a doc"
            )))
            .on_click(cx.listener(|this, _, window, cx| this.ask_for_a_doc(window, cx)));
        div()
            .size_full()
            .px(px(12.))
            .flex()
            .flex_col()
            .child(
                div()
                    .mt(px(12.))
                    .mb(px(2.))
                    .flex_shrink_0()
                    .flex()
                    .gap(px(8.))
                    .child(search)
                    .child(new),
            )
            .child(
                div()
                    .id("pilot-document-list")
                    .debug_selector(|| "pilot-document-list".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .when(pilot.documents.is_empty(), |d| {
                        d.child(empty_state(
                            "file",
                            &format!(
                                "Ask {pilot_name} to write something up, like a plan or a summary, and it shows up here."
                            ),
                        ))
                    })
                    .when(groups.is_empty() && !pilot.documents.is_empty(), |d| {
                        d.child(empty_state("search-01", "No documents match this search."))
                    })
                    .children(groups.into_iter().map(|group| {
                        let label = group.label;
                        div()
                            .debug_selector(move || format!("pilot-docs-group-{label}"))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .px(px(4.))
                                    .pt(px(14.))
                                    .pb(px(6.))
                                    .text_size(px(11.))
                                    .line_height(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ink(0.55, 0x8a8a8a))
                                    .child(label.to_uppercase()),
                            )
                            .children(group.rows.into_iter().map(|row| {
                                let id = row.id.clone();
                                let on = selected.as_deref() == Some(row.id.as_str());
                                let selector = format!("pilot-document-{}", row.id);
                                div()
                                    .id(SharedString::from(selector.clone()))
                                    .debug_selector(move || selector.clone())
                                    .role(Role::Button)
                                    .aria_label(SharedString::from(format!(
                                        "Open document {}",
                                        row.title
                                    )))
                                    .cursor_pointer()
                                    .flex()
                                    .items_start()
                                    .gap(px(10.))
                                    .p(px(10.))
                                    .rounded(px(10.))
                                    .when(on, |d| d.bg(content(0.10)))
                                    .when(!on, |d| d.hover(|s| s.bg(content(0.06))))
                                    .child(
                                        div()
                                            .pt(px(1.))
                                            .child(icon("doc-page", 15.).text_color(rgb(0xa5b4fc))),
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
                                    .children(row.badge.map(|(label, kind)| badge(label, kind)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_document(&id, cx);
                                        this.route = Some(Route::Document(id.clone()));
                                        cx.notify();
                                    }))
                            }))
                    })),
            )
            .into_any_element()
    }

    /// "+ New": the Pilot writes documents, so the chat box asks it for one.
    fn ask_for_a_doc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat) = self.chat() else {
            return;
        };
        let input = chat.read(cx).input.clone();
        input.update(cx, |input, cx| input.set_text("Write a doc: ".into(), cx));
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// Pins or unpins document `id` of `pilot`.
    pub(super) fn set_pinned(
        &mut self,
        pilot: &str,
        id: &str,
        pinned: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(m) = self.roster.get_mut(pilot) else {
            return;
        };
        if let Some(document) = m.documents.iter_mut().find(|d| d.id == id) {
            document.pinned = pinned;
        }
        if let Some(document) = self.work.documents.get_mut(id) {
            document.pinned = pinned;
        }
        self.save(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::flow::{AgentState, Flow, RunStatus, RunTag, Transition, VisitStatus};

    const MIN: u64 = 60_000;

    fn doc(id: &str, title: &str, at: u64) -> pilot::Document {
        pilot::Document {
            id: id.into(),
            kind: "document".into(),
            title: title.into(),
            created_at: at,
            updated_at: at,
            ..Default::default()
        }
    }

    fn run(id: &str, status: RunStatus) -> FlowRun {
        let agent = |id: &str, title: &str| {
            FlowState::Agent(AgentState {
                id: id.into(),
                title: title.into(),
                harness: "claude".into(),
                model: "opus".into(),
                brief: "do it".into(),
                outputs: vec![],
                transitions: vec![Transition {
                    to: "$succeeded".into(),
                    when: None,
                }],
                max_visits: 1,
                when: None,
            })
        };
        FlowRun {
            id: id.into(),
            flow_id: "release".into(),
            flow: Flow {
                id: "release".into(),
                name: "Release".into(),
                description: String::new(),
                cwd: "/w".into(),
                inputs: vec![],
                initial: "plan".into(),
                states: vec![agent("plan", "Plan"), agent("publish", "Publish")],
                access: "auto".into(),
                owner: Some("p1".into()),
                created_at: 0,
                updated_at: 0,
            },
            status,
            inputs: HashMap::new(),
            worktree_cwd: None,
            current_state: None,
            error: None,
            created_at: 0,
            finished_at: None,
            tag: Some(RunTag {
                goal: "g".into(),
                task: "Release 0.2.1".into(),
            }),
            edits: vec![],
        }
    }

    fn visit(seq: u32, state: &str, from: u64, to: Option<u64>) -> FlowVisit {
        FlowVisit {
            id: format!("v{seq}"),
            run_id: "r".into(),
            seq,
            state_id: state.into(),
            status: if to.is_some() {
                VisitStatus::Completed
            } else {
                VisitStatus::Running
            },
            prompt: None,
            session_id: None,
            outputs: None,
            error: None,
            retried: false,
            note: None,
            started_at: from as i64,
            finished_at: to.map(|t| t as i64),
        }
    }

    #[test]
    fn docs_group_as_pinned_this_run_and_earlier() {
        let now = 1_800_000_000_000;
        let mut sop = doc("sop", "Gespan Release Process SOP", now - 3 * 24 * 60 * MIN);
        sop.pinned = true;
        sop.run_id = Some("going".into());
        let mut plan = doc("plan", "Release plan 0.2.1", now - 30 * MIN);
        plan.run_id = Some("going".into());
        let mut old_plan = doc("old", "Release plan 0.2.0", now - 2 * 24 * 60 * MIN);
        old_plan.run_id = Some("done".into());
        let loose = doc("loose", "Notes", now - 5 * 24 * 60 * MIN);
        let runs = vec![
            (
                run("going", RunStatus::Running),
                vec![
                    visit(1, "plan", now - 40 * MIN, Some(now - 20 * MIN)),
                    visit(2, "publish", now - 20 * MIN, None),
                ],
            ),
            (run("done", RunStatus::Succeeded), vec![]),
        ];
        let groups = doc_groups(&[loose, old_plan, plan, sop], &runs, now);
        let labels: Vec<&str> = groups.iter().map(|g| g.label).collect();
        assert_eq!(labels, ["Pinned", "This run", "Earlier"]);
        // A pinned document stays pinned, whatever run it was written for.
        assert_eq!(groups[0].rows[0].id, "sop");
        assert!(groups[0].rows[0].detail.starts_with("Updated "));
        let this = &groups[1].rows[0];
        assert_eq!(this.id, "plan");
        assert_eq!(
            this.detail, "Today · written in Plan",
            "the step running when it was written"
        );
        assert_eq!(this.badge, Some(("Draft".into(), Badge::Draft)));
        // A finished run's document is earlier, newest first.
        let earlier: Vec<&str> = groups[2].rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(earlier, ["old", "loose"]);
        assert_eq!(groups[2].rows[0].badge, None);
    }

    #[test]
    fn without_a_run_going_nothing_is_a_draft() {
        let now = 1_800_000_000_000;
        let mut plan = doc("plan", "Release plan", now);
        plan.run_id = Some("done".into());
        let runs = vec![(run("done", RunStatus::Succeeded), vec![])];
        let groups = doc_groups(&[plan], &runs, now);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, "Earlier");
        assert_eq!(groups[0].rows[0].detail, "Today");
    }
}
