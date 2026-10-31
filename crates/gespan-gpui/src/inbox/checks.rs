//! The Checks tab: a pull request's checks with their job evidence, picking
//! checks for an agent to repair, rerunning failed jobs, and following
//! repairs. Decisions live in `gespan_core::inbox::ci_repair`; this file holds
//! the view state, the background calls and the rows.
use super::{
    Event, Inbox, Tab,
    ui::{action, error, message},
};
use crate::{right_panel::spinner, runtime::locked, theme::*};
use gespan_core::inbox::{
    Check, Checks, Item, Service,
    ci_repair::{
        CheckAnnotation, CheckDetails, CheckFilter, CheckRef, CheckSelection, CheckStep,
        ChecksView, Counts, RepairEvidence, RepairGroup, RepairItem, RepairPhase, RepairState,
        Repairs, TrackedRepair, build_repair_request, check_duration, check_groups,
        find_check_repair, fix_label, github_actions_job_id, headline, is_failed, repair_groups,
        rerun_notice, rerun_run_ids, sort_checks, state_label,
    },
};
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, Role, SharedString, Stateful, div, prelude::*, px,
    rgb,
};
use std::collections::{HashMap, HashSet};

/// The GitHub calls behind the tab, which tests answer from fixtures.
#[derive(Clone, Copy)]
pub(super) struct Backend {
    checks: fn(&Service, &Item) -> Result<Checks, String>,
    details: fn(&Service, &Item, &str) -> Result<CheckDetails, String>,
    rerun: fn(&Service, &Item, &[String]) -> Result<usize, String>,
}
impl Default for Backend {
    fn default() -> Self {
        Self {
            checks: Service::checks,
            details: Service::check_details,
            rerun: Service::rerun_failed,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum DetailsLoad {
    Loading,
    Loaded(CheckDetails),
    Failed(String),
}

/// A job's details are saved per state of that job, so a poll that changes
/// the check reloads them and one that does not keeps them on screen.
fn details_key(check: &Check) -> String {
    format!(
        "{}\0{}\0{}\0{}\0{}",
        check.name,
        check.url.as_deref().unwrap_or_default(),
        check.state,
        check.started_at.as_deref().unwrap_or_default(),
        check.completed_at.as_deref().unwrap_or_default(),
    )
}

/// The tab's state. Everything but the tracked repairs belongs to the
/// selected item and is dropped with it.
#[derive(Default)]
pub(super) struct CiState {
    backend: Backend,
    repairs: Repairs,
    filter: CheckFilter,
    show_others: bool,
    selection: CheckSelection,
    expanded: HashSet<CheckRef>,
    details: HashMap<String, DetailsLoad>,
    all_annotations: HashSet<CheckRef>,
    open_steps: HashSet<CheckRef>,
    open_cards: HashSet<Option<i64>>,
    refreshing: bool,
    refresh_queued: bool,
    /// The last refresh failed, so the rows on screen may be out of date.
    stale_error: Option<String>,
    form: bool,
    preparing: bool,
    form_error: Option<String>,
    rerunning: bool,
    rerun_notice: Option<String>,
    rerun_error: Option<String>,
    /// Drops answers meant for a previously selected item.
    request: u64,
}
impl CiState {
    pub(super) fn load(store: &gespan_core::store::Store) -> Self {
        Self {
            repairs: Repairs::load(store),
            ..Default::default()
        }
    }
    pub(super) fn reset(&mut self) {
        *self = Self {
            backend: self.backend,
            repairs: std::mem::take(&mut self.repairs),
            request: self.request + 1,
            ..Default::default()
        };
    }
}

impl Inbox {
    fn checks_snapshot(&self) -> ChecksView<'_> {
        ChecksView {
            checks: self.checks.as_ref(),
            loading: self.extra_loading,
            refreshing: self.ci.refreshing,
            stale: self.ci.stale_error.is_some(),
            error: self.ci.stale_error.is_some() || self.extra_error.is_some(),
        }
    }
    fn repair_groups(&self, item: &Item) -> Vec<RepairGroup> {
        repair_groups(
            &self.ci.repairs,
            &self.repair_cwd(item),
            &item.repo,
            item.number,
            &self.checks_snapshot(),
        )
    }
    fn repair_cwd(&self, item: &Item) -> String {
        if item.project_path.is_empty() {
            self.start_project.to_string_lossy().into_owned()
        } else {
            item.project_path.clone()
        }
    }
    fn repair_blocked(&self) -> bool {
        self.ci.refreshing || self.ci.stale_error.is_some() || self.extra_error.is_some()
    }
    fn save_repairs(&mut self) {
        if let Err(error) = locked(&self.store).and_then(|db| self.ci.repairs.save(&db)) {
            self.ci.form_error = Some(error);
        }
    }

    /// Reloads the checks behind the rows on screen. A failed refresh keeps
    /// them, marked out of date; a refresh during another runs right after it.
    pub(super) fn refresh_checks(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.item() else {
            return;
        };
        if self.checks.is_none() {
            self.set_tab(Tab::Checks, cx);
            return;
        }
        if self.ci.refreshing {
            self.ci.refresh_queued = true;
            return;
        }
        self.ci.refreshing = true;
        let request = self.ci.request;
        let (service, backend) = (self.service.clone(), self.ci.backend);
        let task = cx
            .background_executor()
            .spawn(async move { (backend.checks)(&service, &item) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.ci.request != request {
                    return;
                }
                this.ci.refreshing = false;
                match result {
                    Ok(checks) => {
                        this.ci.stale_error = None;
                        this.ci.selection.retain_failed(&checks);
                        if !checks.checks.iter().any(is_failed) {
                            this.ci.form = false;
                        }
                        this.checks = Some(checks);
                        this.reload_open_details(cx);
                    }
                    Err(error) => this.ci.stale_error = Some(error),
                }
                if std::mem::take(&mut this.ci.refresh_queued) {
                    this.refresh_checks(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// After a refresh, open rows catch up with their job. A running job's
    /// steps advance without changing its row, so it always reloads.
    fn reload_open_details(&mut self, cx: &mut Context<Self>) {
        let Some(checks) = &self.checks else {
            return;
        };
        let stale = checks
            .checks
            .iter()
            .filter(|check| {
                self.ci.expanded.contains(&CheckRef::of(check))
                    && (check.state == "pending"
                        || !self.ci.details.contains_key(&details_key(check)))
            })
            .cloned()
            .collect::<Vec<_>>();
        for check in stale {
            self.load_details(&check, cx);
        }
    }

    fn load_details(&mut self, check: &Check, cx: &mut Context<Self>) {
        let Some(item) = self.item() else {
            return;
        };
        let Some(job) = github_actions_job_id(check.url.as_deref(), &item.repo) else {
            return;
        };
        let key = details_key(check);
        if !matches!(self.ci.details.get(&key), Some(DetailsLoad::Loaded(_))) {
            self.ci.details.insert(key.clone(), DetailsLoad::Loading);
        }
        let request = self.ci.request;
        let (service, backend) = (self.service.clone(), self.ci.backend);
        let task = cx
            .background_executor()
            .spawn(async move { (backend.details)(&service, &item, &job) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.ci.request == request {
                    this.ci.details.insert(
                        key,
                        match result {
                            Ok(details) => DetailsLoad::Loaded(details),
                            Err(error) => DetailsLoad::Failed(error),
                        },
                    );
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_details(&mut self, check: &Check, cx: &mut Context<Self>) {
        let key = CheckRef::of(check);
        if self.ci.expanded.remove(&key) {
            cx.notify();
            return;
        }
        self.ci.expanded.insert(key);
        if !self.ci.details.contains_key(&details_key(check)) {
            self.load_details(check, cx);
        }
        cx.notify();
    }

    fn open_form(&mut self, check: Option<&Check>, cx: &mut Context<Self>) {
        if let (Some(check), Some(checks)) = (check, &self.checks) {
            self.ci.selection.only(&checks.head_oid, check);
        }
        self.ci.form = true;
        self.ci.form_error = None;
        cx.notify();
    }

    /// Gathers the picked checks' job evidence, then hands the repair to an
    /// agent in a new project chat. The shell reports back on that chat.
    fn start_repair(&mut self, cx: &mut Context<Self>) {
        if self.ci.preparing || self.repair_blocked() {
            return;
        }
        let (Some(mut item), Some(checks)) = (self.item(), self.checks.clone()) else {
            return;
        };
        let targets = self.ci.selection.targets(&checks);
        if targets.is_empty() {
            return;
        }
        item.project_path = self.repair_cwd(&item);
        self.ci.preparing = true;
        self.ci.form_error = None;
        let request = self.ci.request;
        let (service, backend) = (self.service.clone(), self.ci.backend);
        let task = cx.background_executor().spawn({
            let item = item.clone();
            async move {
                let evidence = targets
                    .into_iter()
                    .map(|check| {
                        let details = github_actions_job_id(check.url.as_deref(), &item.repo)
                            .map(|job| (backend.details)(&service, &item, &job));
                        RepairEvidence { check, details }
                    })
                    .collect::<Vec<_>>();
                build_repair_request(&item.repo, item.number, &checks.head_oid, &evidence)
            }
        });
        cx.spawn(async move |this, cx| {
            let repair = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.ci.request != request {
                    return;
                }
                let tracked = TrackedRepair::new(
                    &item.project_path,
                    repair.target,
                    gespan_core::time::now_ms(),
                );
                let id = tracked.id.clone();
                this.ci.repairs.add(tracked);
                this.save_repairs();
                this.ci.preparing = false;
                this.ci.form = false;
                this.ci.selection.clear();
                cx.emit(Event::StartRepair {
                    item: Box::new(item),
                    prompt: repair.prompt,
                    repair: id,
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn track_test_repair(&mut self) -> String {
        let target = gespan_core::inbox::ci_repair::RepairTarget {
            repo: "acme/web".into(),
            number: 9,
            head_oid: "abc".into(),
            checks: vec![CheckRef::default()],
        };
        let repair = TrackedRepair::new("/work", target, 1);
        let id = repair.id.clone();
        self.ci.repairs.add(repair);
        id
    }
    #[cfg(test)]
    pub(crate) fn test_repair_progress(&self) -> (Option<i64>, RepairPhase) {
        let repair = &self.ci.repairs.items[0];
        (repair.session_id, repair.phase)
    }

    pub(crate) fn repair_session_started(
        &mut self,
        repair: &str,
        session: i64,
        cx: &mut Context<Self>,
    ) {
        if self.ci.repairs.attach_session(repair, session) {
            self.save_repairs();
            cx.notify();
        }
    }
    /// The chat could not be created, so nothing is repairing.
    pub(crate) fn repair_session_failed(
        &mut self,
        repair: &str,
        error: String,
        cx: &mut Context<Self>,
    ) {
        let shown = self.item().is_some_and(|item| {
            self.ci.repairs.items.iter().any(|tracked| {
                tracked.id == repair
                    && tracked.target.number == item.number
                    && tracked.target.repo.eq_ignore_ascii_case(&item.repo)
            })
        });
        if self.ci.repairs.remove(repair) {
            self.save_repairs();
            // The shell reports the error too; the form only reopens on its PR.
            self.ci.form = shown;
            self.ci.form_error = shown.then_some(error);
            cx.notify();
        }
    }
    pub(crate) fn repair_session_settled(&mut self, session: i64, cx: &mut Context<Self>) {
        if self.ci.repairs.settle(session, RepairPhase::Completed) {
            self.save_repairs();
            cx.notify();
        }
    }

    /// Reruns the picked checks' failed jobs, or every failed job when
    /// nothing is picked.
    fn rerun_checks(&mut self, cx: &mut Context<Self>) {
        if self.ci.rerunning {
            return;
        }
        let (Some(item), Some(checks)) = (self.item(), self.checks.as_ref()) else {
            return;
        };
        let rows = if self.ci.selection.is_empty() {
            checks.checks.clone()
        } else {
            self.ci.selection.targets(checks)
        };
        let runs = rerun_run_ids(&rows, &item.repo);
        self.ci.rerunning = true;
        self.ci.rerun_notice = None;
        self.ci.rerun_error = None;
        let request = self.ci.request;
        let (service, backend) = (self.service.clone(), self.ci.backend);
        let task = cx
            .background_executor()
            .spawn(async move { (backend.rerun)(&service, &item, &runs) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.ci.request != request {
                    return;
                }
                this.ci.rerunning = false;
                match result {
                    Ok(runs) => {
                        this.ci.rerun_notice = Some(rerun_notice(runs));
                        this.refresh_checks(cx);
                    }
                    Err(error) => this.ci.rerun_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn checks_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let root = div()
            .id("inbox-checks-scroll")
            .min_h_0()
            .flex_1()
            .overflow_y_scroll()
            .px(px(12.))
            .py(px(20.));
        let Some(checks) = self.checks.clone() else {
            return root
                .when(self.extra_loading, |d| d.child(message("Loading checks…")))
                .when_some(self.extra_error.clone(), |d, text| {
                    d.child(error(text)).child(
                        selectable_action("inbox-checks-retry", "Retry", "Retry loading checks")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_checks(cx))),
                    )
                })
                .into_any_element();
        };
        let Some(item) = self.item() else {
            return root.into_any_element();
        };
        let groups = self.repair_groups(&item);
        let rows = sort_checks(&checks.checks);
        let counts = Counts::checks(&rows);
        let filter = self.ci.filter.active(&counts);
        let shown = check_groups(&rows, filter, self.ci.show_others);
        let mut ix = 0;
        root.child(self.checks_header(&item, &checks, &counts, cx))
            .when_some(self.ci.rerun_notice.clone(), |d, text| {
                d.child(message(text).py(px(4.)).px(px(8.)))
            })
            .when_some(self.ci.rerun_error.clone(), |d, text| {
                d.child(error(text).py(px(4.)).px(px(8.)))
            })
            .children(
                groups
                    .iter()
                    .enumerate()
                    .map(|(ix, group)| self.repair_card(ix, group, &checks, cx)),
            )
            .when(self.ci.stale_error.is_some(), |d| {
                d.child(
                    message("Saved results may be out of date.")
                        .py(px(4.))
                        .px(px(8.)),
                )
            })
            .when(self.ci.form, |d| {
                d.child(self.repair_form(&item, &checks, cx))
            })
            .when(!rows.is_empty(), |d| {
                d.child(self.filter_bar(&counts, rows.len(), filter, cx))
            })
            .when(rows.is_empty(), |d| d.child(message("No checks reported")))
            .children(shown.iter().filter(|group| !group.hidden).map(|group| {
                div()
                    .child(
                        div()
                            .mt(px(12.))
                            .mb(px(6.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(12.))
                            .text_color(content(0.55))
                            .child(state_label(group.state))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(content(0.35))
                                    .child(group.checks.len().to_string()),
                            ),
                    )
                    .children(group.checks.iter().map(|check| {
                        let row = self.check_row(ix, check, &checks, &groups, &item, cx);
                        ix += 1;
                        row
                    }))
            }))
            .when(
                filter == CheckFilter::Attention && counts.pass + counts.skipping > 0,
                |d| {
                    let others = Counts {
                        pass: counts.pass,
                        skipping: counts.skipping,
                        ..Default::default()
                    };
                    d.child(
                        tagged("inbox-checks-others")
                            .role(Role::Button)
                            .cursor_pointer()
                            .mt(px(12.))
                            .pt(px(16.))
                            .px(px(8.))
                            .border_t_1()
                            .border_color(content(0.07))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(11.))
                            .text_color(content(0.5))
                            .hover(|s| s.text_color(content(1.)))
                            .child(icon(
                                if self.ci.show_others {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                12.,
                            ))
                            .child(others.describe().unwrap_or_default())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ci.show_others = !this.ci.show_others;
                                cx.notify();
                            })),
                    )
                },
            )
            .into_any_element()
    }

    fn checks_header(
        &self,
        item: &Item,
        checks: &Checks,
        counts: &Counts,
        cx: &mut Context<Self>,
    ) -> Div {
        let summary = Counts { fail: 0, ..*counts }.describe();
        let reruns = rerun_run_ids(&checks.checks, &item.repo);
        let blocked = self.repair_blocked();
        div()
            .mb(px(12.))
            .px(px(8.))
            .flex()
            .flex_wrap()
            .items_start()
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(!checks.checks.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(18.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(content(1.))
                                .child(headline(counts)),
                        )
                        .when_some(summary, |d, text| {
                            let mut chars = text.chars();
                            let text = chars
                                .next()
                                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                                .unwrap_or_default();
                            d.child(
                                div()
                                    .mt(px(4.))
                                    .text_size(px(12.))
                                    .text_color(content(0.55))
                                    .child(format!("{text}.")),
                            )
                        })
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(counts.fail > 0, |d| {
                        d.child(
                            selectable_action(
                                "inbox-fix-checks",
                                fix_label(self.ci.selection.len()),
                                "Fix failed checks with AI",
                            )
                            .bg(content(0.1))
                            .when(blocked, |d| d.opacity(0.5))
                            .child(icon("sparkles", 14.))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if !this.repair_blocked() {
                                        this.open_form(None, cx);
                                    }
                                },
                            )),
                        )
                    })
                    .when(!reruns.is_empty(), |d| {
                        d.child(
                            selectable_action(
                                "inbox-checks-rerun",
                                if self.ci.rerunning {
                                    "Rerunning…"
                                } else {
                                    "Rerun failed"
                                },
                                "Rerun failed jobs",
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.rerun_checks(cx))),
                        )
                    })
                    .child(
                        tagged("inbox-checks-refresh")
                            .role(Role::Button)
                            .aria_label("Refresh checks")
                            .size(px(24.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.1)))
                            .child(if self.ci.refreshing {
                                spinner("inbox-checks-refreshing", "loader", 14.)
                            } else {
                                icon("refresh-cw", 14.).into_any_element()
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.ci.refreshing {
                                    this.refresh_checks(cx);
                                }
                            })),
                    ),
            )
    }

    fn filter_bar(
        &self,
        counts: &Counts,
        total: usize,
        active: CheckFilter,
        cx: &mut Context<Self>,
    ) -> Div {
        let option = |filter: CheckFilter, id: &str, label: &str, count: usize| {
            let on = filter == active;
            let enabled = filter == CheckFilter::All || count > 0;
            tagged(id)
                .role(Role::Button)
                .aria_label(format!("{label}: {count}"))
                .aria_toggled(if on {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .px(px(10.))
                .py(px(4.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .text_color(content(if on { 1. } else { 0.5 }))
                .when(on, |d| d.bg(content(0.1)))
                .when(!enabled, |d| d.opacity(0.4))
                .when(enabled, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.text_color(content(1.)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ci.filter = filter;
                            this.ci.show_others = false;
                            cx.notify();
                        }))
                })
                .child(label.to_string())
                .child(div().text_color(content(0.4)).child(count.to_string()))
        };
        div()
            .py(px(8.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .gap(px(2.))
                    .p(px(2.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(content(0.07))
                    .child(option(
                        CheckFilter::Attention,
                        "inbox-checks-filter-attention",
                        "Needs attention",
                        counts.attention(),
                    ))
                    .child(option(
                        CheckFilter::All,
                        "inbox-checks-filter-all",
                        "All checks",
                        total,
                    )),
            )
            .when(counts.fail > 0, |d| {
                d.child(
                    div()
                        .text_size(px(10.))
                        .text_color(content(0.4))
                        .child("Failures first"),
                )
            })
    }

    fn repair_form(&self, item: &Item, checks: &Checks, cx: &mut Context<Self>) -> Stateful<Div> {
        let targets = self.ci.selection.targets(checks);
        let subject = if let [check] = targets.as_slice() {
            check.name.clone()
        } else {
            format!("{} failed checks", targets.len())
        };
        let blocked = self.repair_blocked();
        let busy = self.ci.preparing;
        div()
            .id("inbox-repair-form")
            .debug_selector(|| "inbox-repair-form".into())
            .role(Role::Dialog)
            .aria_label("Fix checks with AI")
            .mb(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(content(0.1))
            .child(
                div()
                    .p(px(12.))
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(icon("sparkles", 16.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(content(1.))
                                    .child("Fix with AI"),
                            )
                            .child(
                                div()
                                    .mt(px(2.))
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(content(0.45))
                                    .child(format!("{subject} · PR #{}", item.number)),
                            ),
                    )
                    .child(
                        tagged("inbox-repair-close")
                            .role(Role::Button)
                            .aria_label("Close fix picker")
                            .cursor_pointer()
                            .child(icon("x", 14.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ci.form = false;
                                cx.notify();
                            })),
                    ),
            )
            .when_some(self.ci.form_error.clone(), |d, text| {
                d.child(error(text).py(px(4.)))
            })
            .when(blocked && !busy, |d| {
                d.child(message("Wait for the latest checks before starting a fix.").py(px(4.)))
            })
            .child(
                div()
                    .px(px(12.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(content(0.07))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11.))
                            .child(div().text_color(content(0.65)).child("New project chat"))
                            .child(div().text_color(content(0.35)).child("CI details included")),
                    )
                    .child(
                        selectable_action(
                            "inbox-repair-start",
                            if busy { "Preparing..." } else { "Start fix" },
                            "Start fix",
                        )
                        .bg(content(0.1))
                        .when(blocked || busy, |d| d.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.start_repair(cx))),
                    ),
            )
    }

    fn repair_card(
        &self,
        ix: usize,
        group: &RepairGroup,
        checks: &Checks,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let lead = group.lead();
        let session = group.session_id;
        let open = self.ci.open_cards.contains(&session);
        let single = group.single().cloned();
        let can_show = single.as_ref().is_some_and(|item| {
            item.state == RepairState::Pass
                && checks
                    .checks
                    .iter()
                    .filter(|c| c.name == item.check.name && c.workflow == item.check.workflow)
                    .count()
                    == 1
        });
        div()
            .mb(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(content(0.1))
            .child(
                div()
                    .px(px(12.))
                    .py(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        tagged(format!("inbox-repair-card-{ix}"))
                            .role(Role::Button)
                            .aria_label(format!("Repair details for {}", group.title()))
                            .cursor_pointer()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(repair_icon(lead, format!("inbox-repair-mark-{ix}")))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().flex().flex_wrap().gap(px(12.)).children(
                                        group.status_parts().into_iter().map(|(state, text)| {
                                            div()
                                                .text_size(px(12.))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(repair_color(state))
                                                .child(text)
                                        }),
                                    ))
                                    .child(
                                        div()
                                            .mt(px(2.))
                                            .truncate()
                                            .text_size(px(11.))
                                            .text_color(content(0.65))
                                            .child(group.title()),
                                    ),
                            )
                            .child(icon(
                                if open {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                12.,
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.ci.open_cards.remove(&session) {
                                    this.ci.open_cards.insert(session);
                                }
                                cx.notify();
                            })),
                    )
                    .when(can_show, |d| {
                        let item = single.clone();
                        d.child(
                            selectable_action(
                                format!("inbox-repair-show-{ix}"),
                                "Show check",
                                "Show check",
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.ci.filter = CheckFilter::All;
                                    this.ci.form = false;
                                    if let Some(item) = &item {
                                        this.reveal_check(item, cx);
                                    }
                                },
                            )),
                        )
                    })
                    .when_some(session, |d, id| {
                        d.child(
                            selectable_action(
                                format!("inbox-repair-open-{ix}"),
                                "Open conversation",
                                "Open conversation",
                            )
                            .child(icon("message-square", 14.))
                            .on_click(
                                cx.listener(move |_, _, _, cx| cx.emit(Event::OpenSession(id))),
                            ),
                        )
                    }),
            )
            .when(open, |d| {
                d.child(
                    div()
                        .px(px(12.))
                        .py(px(10.))
                        .border_t_1()
                        .border_color(content(0.07))
                        .text_size(px(11.))
                        .text_color(content(0.55))
                        .child("Included checks")
                        .child(div().my(px(6.)).flex().flex_wrap().gap(px(6.)).children(
                            group.items.iter().map(|item| {
                                div()
                                    .px(px(8.))
                                    .py(px(4.))
                                    .rounded(px(4.))
                                    .bg(content(0.05))
                                    .text_color(content(0.75))
                                    .child(item.check.name.clone())
                            }),
                        ))
                        .child(format!(
                            "Latest PR commit: {}. Results appear in the checks below.",
                            checks.head_oid.chars().take(7).collect::<String>()
                        )),
                )
            })
            .into_any_element()
    }

    fn reveal_check(&mut self, repair: &RepairItem, cx: &mut Context<Self>) {
        let check = self.checks.as_ref().and_then(|checks| {
            checks
                .checks
                .iter()
                .find(|c| c.name == repair.check.name && c.workflow == repair.check.workflow)
                .cloned()
        });
        if let Some(check) = check
            && !self.ci.expanded.contains(&CheckRef::of(&check))
        {
            self.toggle_details(&check, cx);
        }
        cx.notify();
    }

    fn check_row(
        &self,
        ix: usize,
        check: &Check,
        checks: &Checks,
        groups: &[RepairGroup],
        item: &Item,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = CheckRef::of(check);
        let expanded = self.ci.expanded.contains(&key);
        let expandable = github_actions_job_id(check.url.as_deref(), &item.repo).is_some();
        let load = self.ci.details.get(&details_key(check));
        let loaded = match load {
            Some(DetailsLoad::Loaded(details)) => Some(details),
            _ => None,
        };
        let repair = find_check_repair(groups, check, checks);
        let failed = is_failed(check);
        let selected = self.ci.selection.contains(check);
        let (mark, ink) = check_mark(&check.state);
        let duration = check_duration(check.started_at.as_deref(), check.completed_at.as_deref());
        let subtitle = (repair.is_none() || expanded)
            .then(|| loaded.and_then(CheckDetails::subtitle))
            .flatten();
        let head = checks.head_oid.clone();
        let row_check = check.clone();
        div()
            .rounded(px(8.))
            .when(expanded, |d| d.bg(content(0.02)))
            .child(
                div()
                    .px(px(8.))
                    .py(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(if failed {
                        let toggled = row_check.clone();
                        let head = head.clone();
                        tagged(format!("inbox-check-select-{ix}"))
                            .role(Role::CheckBox)
                            .aria_label(format!("Select {}", check.name))
                            .aria_toggled(if selected {
                                gpui::Toggled::True
                            } else {
                                gpui::Toggled::False
                            })
                            .size(px(16.))
                            .flex_shrink_0()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(content(0.3))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(selected, |d| d.bg(content(0.15)).child(icon("check", 12.)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.ci.selection.toggle(&head, &toggled);
                                cx.notify();
                            }))
                            .into_any_element()
                    } else {
                        div().size(px(16.)).flex_shrink_0().into_any_element()
                    })
                    .child(icon(mark, 16.).text_color(ink))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(content(1.))
                                            .child(check.name.clone()),
                                    )
                                    .when(!check.workflow.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_size(px(11.))
                                                .text_color(content(0.4))
                                                .child(check.workflow.clone()),
                                        )
                                    }),
                            )
                            .when_some(subtitle, |d, text| {
                                d.child(
                                    div()
                                        .mt(px(2.))
                                        .truncate()
                                        .text_size(px(12.))
                                        .text_color(content(0.55))
                                        .child(text),
                                )
                            }),
                    )
                    .child(match repair {
                        Some(repair) => div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(repair_color(repair.state))
                            .child(repair_icon(
                                repair.state,
                                format!("inbox-check-repair-mark-{ix}"),
                            ))
                            .child(repair.state.label()),
                        None => div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(ink)
                            .child(state_label(&check.state)),
                    })
                    .when_some(duration, |d, text| {
                        d.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(10.))
                                .text_color(content(0.4))
                                .child(text),
                        )
                    })
                    .when(failed, |d| {
                        let target = row_check.clone();
                        d.child(
                            row_button(
                                format!("inbox-check-fix-{ix}"),
                                format!("Fix {} with AI", check.name),
                                "sparkles",
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if !this.repair_blocked() {
                                        this.open_form(Some(&target), cx);
                                    }
                                },
                            )),
                        )
                    })
                    .when(expandable, |d| {
                        let target = row_check.clone();
                        d.child(
                            row_button(
                                format!("inbox-check-expand-{ix}"),
                                format!(
                                    "{} {} details",
                                    if expanded { "Collapse" } else { "Expand" },
                                    check.name
                                ),
                                if expanded {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.toggle_details(&target, cx)),
                            ),
                        )
                    })
                    .when_some(check.url.clone(), |d, url| {
                        d.child(
                            row_button(
                                format!("inbox-check-link-{ix}"),
                                format!("View {} on GitHub", check.name),
                                "external-link",
                            )
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                    }),
            )
            .when(expanded, |d| {
                d.child(self.check_evidence(ix, check, &key, load, cx))
            })
            .into_any_element()
    }

    fn check_evidence(
        &self,
        ix: usize,
        check: &Check,
        key: &CheckRef,
        load: Option<&DetailsLoad>,
        cx: &mut Context<Self>,
    ) -> Div {
        let body = div()
            .pl(px(40.))
            .pr(px(12.))
            .pb(px(12.))
            .text_size(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.));
        match load {
            None | Some(DetailsLoad::Loading) => body.child(message("Loading steps…").p_0()),
            Some(DetailsLoad::Failed(text)) => {
                let target = check.clone();
                body.child(message("Could not load job details.").p_0())
                    .child(message(text.clone()).p_0().text_size(px(11.)))
                    .child(
                        selectable_action(
                            format!("inbox-check-details-retry-{ix}"),
                            "Retry details",
                            "Retry details",
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ci.details.remove(&details_key(&target));
                            this.load_details(&target, cx);
                        })),
                    )
            }
            Some(DetailsLoad::Loaded(details)) => {
                let all = self.ci.all_annotations.contains(key);
                let steps_open = self.ci.open_steps.contains(key);
                let annotations = details.annotations.len();
                let (more_key, steps_key) = (key.clone(), key.clone());
                body.children(
                    details
                        .annotations
                        .iter()
                        .take(if all { annotations } else { 5 })
                        .map(annotation_card),
                )
                .when(!all && annotations > 5, |d| {
                    d.child(
                        selectable_action(
                            format!("inbox-check-annotations-{ix}"),
                            format!("Show {} more annotations", annotations - 5),
                            "Show more annotations",
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ci.all_annotations.insert(more_key.clone());
                            cx.notify();
                        })),
                    )
                })
                .when(details.steps.is_empty(), |d| {
                    d.child(message("No steps reported for this job.").p_0())
                })
                .when(!details.steps.is_empty(), |d| {
                    let counts = Counts::of(details.steps.iter().map(|s| s.state.as_str()));
                    d.child(
                        tagged(format!("inbox-check-steps-{ix}"))
                            .role(Role::Button)
                            .aria_label("View run steps")
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .text_size(px(11.))
                            .text_color(content(0.5))
                            .hover(|s| s.text_color(content(1.)))
                            .child(icon(
                                if steps_open {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                12.,
                            ))
                            .child(div().flex_1().child("View run steps"))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(content(0.35))
                                    .child(counts.describe().unwrap_or_default()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.ci.open_steps.remove(&steps_key) {
                                    this.ci.open_steps.insert(steps_key.clone());
                                }
                                cx.notify();
                            })),
                    )
                    .when(steps_open, |d| {
                        d.children(details.steps.iter().map(step_row))
                    })
                })
                .when(
                    is_failed(check) && details.annotations.is_empty() && details.notice.is_none(),
                    |d| {
                        d.child(
                            message("No error annotations reported. View the full log on GitHub.")
                                .p_0(),
                        )
                    },
                )
                .when_some(details.notice.clone(), |d, text| {
                    d.child(message(text).p_0())
                })
            }
        }
    }
}

fn tagged(id: impl Into<SharedString>) -> Stateful<Div> {
    let id: SharedString = id.into();
    let selector = id.to_string();
    div().id(id).debug_selector(move || selector)
}
/// An [`action`] tests can find, whose accessible name outlasts its label.
fn selectable_action(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    name: impl Into<SharedString>,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let selector = id.to_string();
    action(id, label)
        .aria_label(name)
        .debug_selector(move || selector)
}
fn row_button(id: String, label: String, name: &'static str) -> Stateful<Div> {
    tagged(id)
        .role(Role::Button)
        .aria_label(label)
        .size(px(24.))
        .flex_shrink_0()
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(content(0.07)))
        .child(icon(name, 13.))
}
fn check_mark(state: &str) -> (&'static str, Hsla) {
    match state {
        "pass" => ("check-circle", rgb(0x4ade80).into()),
        "fail" => ("circle-x", rgb(0xf87171).into()),
        "pending" => ("loader", content(0.55)),
        "cancel" => ("minus", content(0.45)),
        "skipping" => ("circle-dashed", content(0.4)),
        _ => ("alert-circle", content(0.45)),
    }
}
fn repair_color(state: RepairState) -> Hsla {
    match state {
        RepairState::Pass => rgb(0x4ade80).into(),
        RepairState::Fail | RepairState::AgentError => rgb(0xf87171).into(),
        RepairState::Repairing | RepairState::Pending => rgb(0xfbbf24).into(),
        _ => content(0.55),
    }
}
fn repair_icon(state: RepairState, id: String) -> AnyElement {
    if state.spins() {
        return spinner(SharedString::from(id), "loader", 14.);
    }
    let name = match state {
        RepairState::Pass => "check-circle",
        RepairState::Fail | RepairState::AgentError => "circle-x",
        _ => "circle-dashed",
    };
    icon(name, 14.)
        .text_color(repair_color(state))
        .into_any_element()
}
fn annotation_card(annotation: &CheckAnnotation) -> Div {
    let failure = annotation.level == "failure";
    let (title, rest) = annotation
        .message
        .split_once('\n')
        .unwrap_or((annotation.message.as_str(), ""));
    div()
        .rounded(px(8.))
        .border_1()
        .border_color(content(0.1))
        .when(!annotation.path.is_empty(), |d| {
            d.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(content(0.07))
                    .truncate()
                    .text_color(content(0.65))
                    .child(annotation.location()),
            )
        })
        .child(
            div()
                .p(px(12.))
                .flex()
                .items_start()
                .gap(px(8.))
                .child(
                    icon(if failure { "circle-x" } else { "alert-circle" }, 12.).text_color(
                        if failure {
                            rgb(0xf87171)
                        } else {
                            rgb(0xfbbf24)
                        },
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_color(content(0.75)).child(title.to_string()))
                        .when(!rest.trim().is_empty(), |d| {
                            d.child(
                                div()
                                    .mt(px(6.))
                                    .font_family(crate::fonts::monospace())
                                    .text_size(px(11.))
                                    .text_color(content(0.65))
                                    .child(rest.to_string()),
                            )
                        }),
                ),
        )
}
fn step_row(step: &CheckStep) -> Div {
    let (mark, ink) = check_mark(&step.state);
    div()
        .px(px(8.))
        .py(px(4.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .gap(px(8.))
        .when(step.state == "fail", |d| d.bg(gpui::rgba(0xf8717110)))
        .child(icon(mark, 14.).text_color(ink))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(content(0.8))
                .child(step.name.clone()),
        )
        .when_some(
            check_duration(step.started_at.as_deref(), step.completed_at.as_deref()),
            |d, text| d.child(div().flex_shrink_0().text_color(content(0.45)).child(text)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions,
        point, size,
    };
    use std::{cell::RefCell, rc::Rc};

    const JOB_URL: &str = "https://github.com/acme/web/actions/runs/5/job";
    fn check(name: &str, state: &str, job: u64) -> Check {
        Check {
            name: name.into(),
            workflow: "CI".into(),
            state: state.into(),
            url: Some(format!("{JOB_URL}/{job}")),
            ..Default::default()
        }
    }
    fn fixture_checks() -> Checks {
        Checks {
            head_oid: "a".repeat(40),
            checks: vec![
                check("docs", "pass", 10),
                check("unit", "fail", 11),
                check("lint", "fail", 12),
                check("e2e", "pending", 13),
            ],
        }
    }
    fn job_details(_: &Service, _: &Item, job: &str) -> Result<CheckDetails, String> {
        if job == "12" {
            return Err("HTTP 502".into());
        }
        Ok(CheckDetails {
            steps: vec![
                CheckStep {
                    name: "Install".into(),
                    state: "pass".into(),
                    ..Default::default()
                },
                CheckStep {
                    name: "Run tests".into(),
                    state: "fail".into(),
                    ..Default::default()
                },
            ],
            annotations: vec![CheckAnnotation {
                path: "src/app.rs".into(),
                line: 4,
                message: "Expected 2, received 1".into(),
                level: "failure".into(),
            }],
            notice: None,
        })
    }
    fn backend() -> Backend {
        Backend {
            checks: |_, _| Ok(fixture_checks()),
            details: job_details,
            rerun: |_, _, runs| Ok(runs.len()),
        }
    }
    fn open(cx: &mut TestAppContext) -> WindowHandle<Inbox> {
        cx.update(|cx| cx.set_global(crate::settings::Preferences::default()));
        let store = gespan_core::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let store = std::sync::Arc::new(std::sync::Mutex::new(store));
        let window = cx.update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1100.), px(800.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let mut inbox =
                            Inbox::new_inner(store, vec![], "/missing/inbox".into(), false, cx);
                        let pr = Item {
                            provider: "github".into(),
                            kind: "pr".into(),
                            repo: "acme/web".into(),
                            number: 9,
                            state: "open".into(),
                            title: "Change".into(),
                            ..Default::default()
                        };
                        inbox.selected = Some(pr.key());
                        inbox.snapshot.items = vec![pr];
                        inbox.tab = Tab::Checks;
                        inbox.checks = Some(fixture_checks());
                        inbox.ci.backend = backend();
                        inbox
                    })
                },
            )
            .unwrap()
        });
        cx.run_until_parked();
        window
    }
    fn settle(visual: &mut VisualTestContext) {
        visual.run_until_parked();
        visual.refresh().unwrap();
    }
    fn shown(visual: &mut VisualTestContext, selector: &'static str) -> bool {
        settle(visual);
        visual.debug_bounds(selector).is_some()
    }
    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        settle(visual);
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        visual.simulate_click(bounds.center(), Default::default());
        settle(visual);
    }

    #[gpui::test]
    fn picked_checks_become_a_repair_whose_progress_follows_the_agent(cx: &mut TestAppContext) {
        let window = open(cx);
        let inbox = window.root(cx).unwrap();
        let started = Rc::new(RefCell::new(vec![]));
        cx.update(|cx| {
            let started = started.clone();
            cx.subscribe(&inbox, move |_, event: &Event, _| {
                if let Event::StartRepair { prompt, repair, .. } = event {
                    started.borrow_mut().push((prompt.clone(), repair.clone()));
                }
            })
            .detach();
        });
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        // Rows sort failures first: unit, lint, then the running job. Only
        // failures can be picked.
        assert!(shown(visual, "inbox-check-select-0"));
        assert!(shown(visual, "inbox-check-select-1"));
        assert!(!shown(visual, "inbox-check-select-2"));
        click(visual, "inbox-check-select-0");
        let label = window
            .update(visual, |inbox, _, _| fix_label(inbox.ci.selection.len()))
            .unwrap();
        assert_eq!(label, "Fix 1 selected");

        assert!(!shown(visual, "inbox-repair-form"));
        click(visual, "inbox-fix-checks");
        assert!(shown(visual, "inbox-repair-form"));
        click(visual, "inbox-repair-start");

        let (prompt, id) = started.borrow().first().cloned().expect("a repair started");
        assert!(
            prompt.contains(r#"Selected checks: ["CI/unit"]"#),
            "{prompt}"
        );
        assert!(
            prompt.contains("Expected 2, received 1"),
            "job evidence is sent"
        );
        assert!(!prompt.contains("CI/lint"), "unpicked checks stay out");
        window
            .update(visual, |inbox, _, _| {
                assert!(!inbox.ci.form);
                assert!(inbox.ci.selection.is_empty());
                assert_eq!(inbox.ci.repairs.items[0].phase, RepairPhase::Running);
            })
            .unwrap();

        // The card and the row follow the agent, then wait for new CI.
        assert!(shown(visual, "inbox-repair-card-0"));
        assert!(!shown(visual, "inbox-repair-open-0"), "no chat yet");
        window
            .update(visual, |inbox, _, cx| {
                inbox.repair_session_started(&id, 42, cx)
            })
            .unwrap();
        assert!(shown(visual, "inbox-repair-open-0"));
        let state = |inbox: &Inbox| {
            let item = inbox.item().unwrap();
            inbox.repair_groups(&item)[0].items[0].state
        };
        let repairing = window.update(visual, |inbox, _, _| state(inbox)).unwrap();
        assert_eq!(repairing, RepairState::Repairing);
        window
            .update(visual, |inbox, _, cx| inbox.repair_session_settled(42, cx))
            .unwrap();
        let waiting = window.update(visual, |inbox, _, _| state(inbox)).unwrap();
        assert_eq!(waiting, RepairState::Waiting);
        // A pushed fix whose new job passes reads as passed.
        window
            .update(visual, |inbox, _, cx| {
                let mut next = check("unit", "pass", 21);
                next.started_at = Some("2100-01-01T00:00:00Z".into());
                inbox.checks = Some(Checks {
                    head_oid: "b".repeat(40),
                    checks: vec![next],
                });
                cx.notify();
            })
            .unwrap();
        let passed = window.update(visual, |inbox, _, _| state(inbox)).unwrap();
        assert_eq!(passed, RepairState::Pass);
        assert!(shown(visual, "inbox-repair-show-0"));
    }

    #[gpui::test]
    fn a_chat_that_cannot_start_returns_to_the_form_with_its_error(cx: &mut TestAppContext) {
        let window = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        click(visual, "inbox-check-fix-1");
        let picked = window
            .update(visual, |inbox, _, _| {
                inbox.ci.selection.targets(&fixture_checks())
            })
            .unwrap();
        assert_eq!(picked.len(), 1, "a row's fix picks only that check");
        assert_eq!(picked[0].name, "lint");
        click(visual, "inbox-repair-start");
        window
            .update(visual, |inbox, _, cx| {
                let id = inbox.ci.repairs.items[0].id.clone();
                inbox.repair_session_failed(&id, "Choose a project folder".into(), cx);
                assert!(inbox.ci.repairs.items.is_empty(), "nothing is repairing");
            })
            .unwrap();
        assert!(shown(visual, "inbox-repair-form"));
        let failure = window
            .update(visual, |inbox, _, _| inbox.ci.form_error.clone())
            .unwrap();
        assert_eq!(failure.as_deref(), Some("Choose a project folder"));
    }

    #[gpui::test]
    fn a_checks_job_log_opens_and_a_failed_load_can_retry(cx: &mut TestAppContext) {
        let window = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        click(visual, "inbox-check-expand-0");
        assert!(shown(visual, "inbox-check-steps-0"));
        window
            .update(visual, |inbox, _, _| {
                let unit = check("unit", "fail", 11);
                assert!(inbox.ci.expanded.contains(&CheckRef::of(&unit)));
                assert!(matches!(
                    inbox.ci.details.get(&details_key(&unit)),
                    Some(DetailsLoad::Loaded(details)) if details.annotations.len() == 1
                ));
            })
            .unwrap();
        click(visual, "inbox-check-steps-0");
        let open = window
            .update(visual, |inbox, _, _| inbox.ci.open_steps.len())
            .unwrap();
        assert_eq!(open, 1);

        // The second job's request fails, and its retry succeeds.
        assert!(!shown(visual, "inbox-check-details-retry-1"));
        click(visual, "inbox-check-expand-1");
        assert!(shown(visual, "inbox-check-details-retry-1"));
        window
            .update(visual, |inbox, _, _| {
                inbox.ci.backend.details = |_, _, _| Ok(CheckDetails::default());
            })
            .unwrap();
        click(visual, "inbox-check-details-retry-1");
        assert!(!shown(visual, "inbox-check-details-retry-1"));
        assert!(
            !shown(visual, "inbox-check-steps-1"),
            "this job has no steps"
        );

        // Collapsing hides the evidence but keeps it for the next look.
        click(visual, "inbox-check-expand-0");
        assert!(!shown(visual, "inbox-check-steps-0"));
    }

    #[gpui::test]
    fn the_filter_folds_passing_checks_away(cx: &mut TestAppContext) {
        let window = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        // Needs attention shows three rows; the passing fourth is folded.
        assert!(shown(visual, "inbox-check-link-2"));
        assert!(!shown(visual, "inbox-check-link-3"));
        click(visual, "inbox-checks-others");
        assert!(shown(visual, "inbox-check-link-3"));
        click(visual, "inbox-checks-others");
        assert!(!shown(visual, "inbox-check-link-3"));
        click(visual, "inbox-checks-filter-all");
        assert!(shown(visual, "inbox-check-link-3"));
        assert!(!shown(visual, "inbox-checks-others"));
        click(visual, "inbox-checks-filter-attention");
        assert!(!shown(visual, "inbox-check-link-3"));
    }

    #[gpui::test]
    fn a_failed_refresh_keeps_the_rows_and_holds_back_a_fix(cx: &mut TestAppContext) {
        let window = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        click(visual, "inbox-check-select-0");
        window
            .update(visual, |inbox, _, _| {
                inbox.ci.backend.checks = |_, _| Err("gh failed".into());
            })
            .unwrap();
        click(visual, "inbox-checks-refresh");
        assert!(shown(visual, "inbox-check-select-0"), "rows stay on screen");
        window
            .update(visual, |inbox, _, _| {
                assert!(inbox.repair_blocked());
                assert_eq!(inbox.ci.stale_error.as_deref(), Some("gh failed"));
            })
            .unwrap();
        click(visual, "inbox-fix-checks");
        assert!(
            !shown(visual, "inbox-repair-form"),
            "wait for the latest checks"
        );

        // Fresh results on a new commit drop the picks made on the old one.
        window
            .update(visual, |inbox, _, _| {
                inbox.ci.backend.checks = |_, _| {
                    Ok(Checks {
                        head_oid: "c".repeat(40),
                        checks: vec![check("unit", "fail", 31)],
                    })
                };
            })
            .unwrap();
        click(visual, "inbox-checks-refresh");
        window
            .update(visual, |inbox, _, _| {
                assert!(!inbox.repair_blocked());
                assert!(inbox.ci.selection.is_empty());
                assert_eq!(inbox.checks.as_ref().unwrap().checks.len(), 1);
            })
            .unwrap();
    }

    #[gpui::test]
    fn rerunning_failed_jobs_reports_back_and_refreshes(cx: &mut TestAppContext) {
        let window = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |inbox, _, _| {
                inbox.ci.backend.checks = |_, _| {
                    Ok(Checks {
                        head_oid: "a".repeat(40),
                        checks: vec![check("unit", "pending", 11)],
                    })
                };
            })
            .unwrap();
        click(visual, "inbox-checks-rerun");
        window
            .update(visual, |inbox, _, _| {
                assert_eq!(
                    inbox.ci.rerun_notice.as_deref(),
                    Some(rerun_notice(1).as_str())
                );
                assert_eq!(
                    inbox.checks.as_ref().unwrap().checks[0].state,
                    "pending",
                    "the rerun's jobs appear right away"
                );
            })
            .unwrap();
        // A refused rerun keeps its error instead of the notice.
        window
            .update(visual, |inbox, _, cx| {
                inbox.checks = Some(fixture_checks());
                inbox.ci.backend.rerun = |_, _, _| Err("run is still in progress".into());
                cx.notify();
            })
            .unwrap();
        click(visual, "inbox-checks-rerun");
        window
            .update(visual, |inbox, _, _| {
                assert_eq!(inbox.ci.rerun_notice, None);
                assert_eq!(
                    inbox.ci.rerun_error.as_deref(),
                    Some("run is still in progress")
                );
            })
            .unwrap();
    }
}
