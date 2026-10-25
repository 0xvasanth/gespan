//! "Report an issue": the sheet, its exact preview of the bundle, and saving
//! the zip for the user to send on, by email or in a prefilled GitHub issue.
//! Gespan never sends it. The preview draws [`Bundle::entries`] and saving
//! writes [`Bundle::zip`], which is made from those entries, so what the user
//! reads is what is saved.
use crate::{
    input::{self, Input},
    markdown::PILOT,
    theme::*,
};
use gespan_core::diagnostics::{
    self, Bundle, Channel, Destination, Environment, Report, Selection, Sources, Stamp, System,
};
use gespan_redact::Redactor;
use gpui::{
    ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    ListHorizontalSizingBehavior, Render, Role, SharedString, Subscription,
    UniformListScrollHandle, Window, div, prelude::*, px, rgb, uniform_list,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

gpui::actions!(report, [ReportIssue]);

pub(crate) const REQUIRED: &str = "Add a title and say what happened.";
/// Says where each way out sends the report, and that nothing goes on its own.
pub(crate) const PUBLIC_WARNING: &str = "Gespan never sends this by itself. Both buttons save the zip to your Downloads folder for you to attach. Email goes privately to the Gespan maintainer. A GitHub issue needs a GitHub account, and the issue and its zip are public.";
const REPORT_JSON: &str = "report.json";
const PREVIEW_LINE: f32 = 17.;
const PREVIEW_HEIGHT: f32 = 220.;
/// Typing settles this long before the zip is rebuilt.
const REBUILD_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

/// What an entry point already knows: an error toast's text and the
/// `event_id`s logged just before it, or a crash's location.
#[derive(Clone, Debug, Default)]
pub(crate) struct Prefill {
    pub title: String,
    pub event_ids: Vec<String>,
}

/// Facts the app detects by running commands; tests pass them in.
pub(crate) struct Facts {
    pub system: System,
    pub providers: Vec<(String, Option<String>)>,
}

pub(crate) type Reveal = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

pub(crate) struct SheetContext {
    /// `None` until diagnostics have started: then there are no logs to add.
    pub log_dir: Option<PathBuf>,
    pub projects: Vec<String>,
    /// More folders to hide behind `<project-N>`: archived projects,
    /// session folders and session worktree folders.
    pub other_folders: Vec<String>,
    /// Where the scrubber reads this machine's known secrets.
    pub machine: diagnostics::Machine,
    pub sessions_count: usize,
    pub version: String,
    pub channel: Channel,
    pub downloads: Option<PathBuf>,
    pub destination: Destination,
    pub facts: Option<Facts>,
    pub reveal: Reveal,
}

/// One attachable entry in the preview list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub name: String,
    pub included: bool,
}

pub(crate) enum ReportEvent {
    Close,
}

/// How the user sends the saved zip on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Via {
    /// A `mailto:` link to the maintainer: private.
    Email,
    /// GitHub's prefilled bug form: public.
    GitHub,
}

/// Scrubbed sources and the redactor that scrubbed them, gathered once.
struct Gathered {
    system: System,
    sources: Sources,
    redactor: Redactor,
}

pub(crate) struct ReportSheet {
    pub(crate) title: Entity<Input>,
    pub(crate) description: Entity<Input>,
    pub(crate) steps: Entity<Input>,
    pub(crate) email: Entity<Input>,
    pub(crate) error: Option<String>,
    /// The saved zip's file name and where it was sent on, once saved.
    pub(crate) saved: Option<(String, Via)>,
    event_ids: Vec<String>,
    selection: Selection,
    stamp: Stamp,
    log_dir: Option<PathBuf>,
    downloads: Option<PathBuf>,
    destination: Destination,
    reveal: Reveal,
    gathered: Option<Arc<Gathered>>,
    /// The rebuild waiting or in flight; replacing it cancels the older one
    /// if it hasn't started.
    building: Option<gpui::Task<()>>,
    /// Finished builds, so tests can tell a settled rebuild from many.
    #[cfg(test)]
    pub(crate) builds: usize,
    bundle: Option<Bundle>,
    shown: Option<String>,
    /// The shown entry's lines, split once per bundle.
    shown_lines: Arc<Vec<SharedString>>,
    preview_scroll: UniformListScrollHandle,
    saving: bool,
    focus: FocusHandle,
    _changes: Vec<Subscription>,
}

impl EventEmitter<ReportEvent> for ReportSheet {}

impl Focusable for ReportSheet {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ReportSheet {
    pub(crate) fn new(prefill: Prefill, context: SheetContext, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| {
            let mut input = Input::field("Title", "A short summary", content(0.35), cx);
            input.set_text(prefill.title.clone(), cx);
            input
        });
        let description = cx.new(|cx| {
            Input::multiline_field(
                "What happened?",
                "What did you expect, and what happened instead?",
                cx,
            )
        });
        let steps =
            cx.new(|cx| Input::multiline_field("Steps to reproduce", "1.\n2.\n3.  (optional)", cx));
        let email = cx.new(|cx| {
            Input::field(
                "Contact email",
                "Optional. Only we see it, never the public issue.",
                content(0.35),
                cx,
            )
        });
        let changes = [&title, &description, &steps, &email]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |this, _, _: &input::Changed, cx| {
                    this.rebuild_after(REBUILD_DELAY, cx)
                })
            })
            .collect();
        let SheetContext {
            log_dir,
            projects,
            other_folders,
            machine,
            sessions_count,
            version,
            channel,
            downloads,
            destination,
            facts,
            reveal,
        } = context;
        let gather = {
            let log_dir = log_dir.clone();
            cx.background_executor().spawn(async move {
                gather(
                    log_dir,
                    Hidden {
                        projects,
                        other_folders,
                        machine,
                    },
                    sessions_count,
                    &version,
                    channel,
                    facts,
                )
            })
        };
        cx.spawn(async move |this, cx| {
            let gathered = gather.await;
            let _ = this.update(cx, |this, cx| {
                this.gathered = Some(Arc::new(gathered));
                this.rebuild(cx);
            });
        })
        .detach();
        Self {
            title,
            description,
            steps,
            email,
            error: None,
            saved: None,
            event_ids: prefill.event_ids,
            selection: Selection::default(),
            stamp: Stamp::now(),
            log_dir,
            downloads,
            destination,
            reveal,
            gathered: None,
            building: None,
            #[cfg(test)]
            builds: 0,
            bundle: None,
            shown: None,
            shown_lines: Arc::default(),
            preview_scroll: UniformListScrollHandle::new(),
            saving: false,
            focus: cx.focus_handle(),
            _changes: changes,
        }
    }

    #[cfg(test)]
    pub(crate) fn event_ids(&self) -> &[String] {
        &self.event_ids
    }

    #[cfg(test)]
    pub(crate) fn bundle(&self) -> Option<&Bundle> {
        self.bundle.as_ref()
    }

    /// An entry's text exactly as it is in the zip.
    pub(crate) fn preview_text(&self, name: &str) -> Option<String> {
        let entry = self.bundle.as_ref()?.entry(name)?;
        Some(String::from_utf8_lossy(&entry.bytes).into_owned())
    }

    /// Every entry the user can include or remove, in zip order.
    pub(crate) fn rows(&self) -> Vec<Row> {
        let Some(gathered) = &self.gathered else {
            return Vec::new();
        };
        let sources = &gathered.sources;
        let on = |flag: bool| self.selection.attach && flag;
        let mut rows = Vec::new();
        if !sources.logs.is_empty() {
            rows.push(row("logs.jsonl", on(self.selection.logs)));
        }
        for (name, _) in &sources.crashes {
            rows.push(row(
                &format!("crashes/{name}"),
                on(!self.selection.crashes_excluded.contains(name)),
            ));
        }
        if sources.counters.is_some() {
            rows.push(row("counters.json", on(self.selection.counters)));
        }
        if sources.environment.is_some() {
            rows.push(row("environment.json", on(self.selection.environment)));
        }
        rows
    }

    pub(crate) fn set_attach(&mut self, attach: bool, cx: &mut Context<Self>) {
        self.selection.attach = attach;
        self.rebuild(cx);
    }

    /// Puts an entry in the bundle or takes it out.
    pub(crate) fn toggle_entry(&mut self, name: &str, cx: &mut Context<Self>) {
        let selection = &mut self.selection;
        match name {
            "logs.jsonl" => selection.logs = !selection.logs,
            "counters.json" => selection.counters = !selection.counters,
            "environment.json" => selection.environment = !selection.environment,
            _ => {
                if let Some(file) = name.strip_prefix("crashes/")
                    && !selection.crashes_excluded.remove(file)
                {
                    selection.crashes_excluded.insert(file.to_owned());
                }
            }
        }
        self.rebuild(cx);
    }

    /// The contact email is only for the private intake; a fallback bundle
    /// goes into a public issue, so the field is hidden.
    pub(crate) fn shows_contact_email(&self) -> bool {
        self.destination == Destination::Intake
    }

    fn report(&self, cx: &Context<Self>) -> Report {
        let text = |input: &Entity<Input>| input.read(cx).content.to_string();
        Report {
            title: text(&self.title),
            description: text(&self.description),
            steps: text(&self.steps),
            contact_email: self.shows_contact_email().then(|| text(&self.email)),
            event_ids: self.event_ids.clone(),
        }
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.rebuild_after(std::time::Duration::ZERO, cx);
    }

    /// Rebuilds the zip in the background, once `delay` has passed without
    /// another change: deflating the log tail on every keystroke would stall
    /// typing. Save stays off until it's done.
    fn rebuild_after(&mut self, delay: std::time::Duration, cx: &mut Context<Self>) {
        let Some(gathered) = self.gathered.clone() else {
            return;
        };
        let report = self.report(cx);
        let selection = self.selection.clone();
        let destination = self.destination;
        let stamp = self.stamp.clone();
        self.building = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let built = cx
                .background_executor()
                .spawn(async move {
                    diagnostics::build(
                        &report,
                        &gathered.system,
                        &gathered.sources,
                        &selection,
                        destination,
                        &gathered.redactor,
                        &stamp,
                        diagnostics::MAX_BUNDLE_BYTES,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.building = None;
                #[cfg(test)]
                {
                    this.builds += 1;
                }
                match built {
                    Ok(bundle) => {
                        this.bundle = Some(bundle);
                        // An edit answers the last complaint.
                        this.error = None;
                        this.split_shown();
                    }
                    Err(error) => {
                        // Never leave an older zip ready to save.
                        this.bundle = None;
                        this.error = Some(format!("Could not build the report: {error}"));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Opens an entry's text in the preview, or closes it.
    pub(crate) fn show_entry(&mut self, name: &str, cx: &mut Context<Self>) {
        self.shown = if self.shown.as_deref() == Some(name) {
            None
        } else {
            Some(name.to_owned())
        };
        self.split_shown();
        cx.notify();
    }

    fn split_shown(&mut self) {
        let text = self
            .shown
            .as_deref()
            .and_then(|name| self.preview_text(name))
            .unwrap_or_default();
        self.shown_lines = Arc::new(
            text.lines()
                .map(|line| SharedString::from(if line.is_empty() { " " } else { line }.to_owned()))
                .collect(),
        );
    }

    /// Saves the previewed zip to Downloads, reveals it and opens a prefilled
    /// email or GitHub issue for the user to attach it to.
    pub(crate) fn submit(&mut self, via: Via, cx: &mut Context<Self>) {
        if self.saving || self.building.is_some() {
            return;
        }
        let report = self.report(cx);
        if report.title.trim().is_empty() || report.description.trim().is_empty() {
            self.error = Some(REQUIRED.into());
            cx.notify();
            return;
        }
        let Some(bundle) = self.bundle.clone() else {
            self.error = Some("The report is still being put together.".into());
            cx.notify();
            return;
        };
        let Some(downloads) = self.downloads.clone() else {
            self.error = Some("There is no Downloads folder to save the report in.".into());
            cx.notify();
            return;
        };
        self.saving = true;
        self.error = None;
        cx.notify();
        let reveal = self.reveal.clone();
        let log_dir = self.log_dir.clone();
        let crashes: Vec<String> = bundle
            .entries
            .iter()
            .filter(|entry| entry.name.starts_with("crashes/"))
            .map(|entry| entry.name.clone())
            .collect();
        let zip = bundle.clone();
        let save = cx.background_executor().spawn(async move {
            let path = diagnostics::save_bundle(&zip, &downloads).map_err(|e| e.to_string())?;
            // The user holds these records now, in the saved zip.
            if let Some(log_dir) = log_dir {
                diagnostics::remove_crashes(&log_dir, &crashes);
            }
            let revealed = reveal(&path);
            Ok::<_, String>((path, revealed))
        });
        let report = bundle.report();
        cx.spawn(async move |this, cx| {
            let result = save.await.map(|(path, revealed)| {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (name, revealed)
            });
            // The zip is saved even if the sheet was closed meanwhile, so the
            // email or issue still opens. The email names the saved file,
            // which may carry a `-1` suffix.
            if let Ok((name, _)) = &result {
                let url = match via {
                    Via::Email => diagnostics::mail_url(&report, name),
                    Via::GitHub => diagnostics::issue_url(&report),
                };
                cx.update(|cx| cx.open_url(&url));
            }
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok((name, revealed)) => {
                        this.saved = Some((name, via));
                        if let Err(error) = revealed {
                            this.error = Some(format!("Saved, but could not show it: {error}"));
                        }
                    }
                    Err(error) => this.error = Some(format!("Could not save the report: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn copy_report(&self, cx: &mut Context<Self>) {
        if let Some(text) = self.preview_text(REPORT_JSON) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
}

fn row(name: &str, included: bool) -> Row {
    Row {
        name: name.into(),
        included,
    }
}

/// What the scrubber hides: these folders and this machine's secrets.
struct Hidden {
    projects: Vec<String>,
    other_folders: Vec<String>,
    machine: diagnostics::Machine,
}

/// Reads and scrubs everything a report can attach. Runs off the UI thread.
fn gather(
    log_dir: Option<PathBuf>,
    hidden: Hidden,
    sessions_count: usize,
    version: &str,
    channel: Channel,
    facts: Option<Facts>,
) -> Gathered {
    let (system, providers) = match facts {
        Some(facts) => (facts.system, facts.providers),
        None => (
            diagnostics::system(version, channel),
            diagnostics::installed_providers(),
        ),
    };
    let environment = Environment {
        providers,
        projects_count: hidden.projects.len(),
        sessions_count,
    };
    let redactor = diagnostics::redactor_for(
        &hidden.machine,
        &diagnostics::report_roots(&hidden.projects, &hidden.other_folders),
    );
    let sources = match &log_dir {
        Some(dir) => {
            diagnostics::collect(dir, &redactor, chrono::Utc::now(), Some(environment), None)
        }
        // No logs folder yet: attach the environment alone.
        None => Sources {
            environment: Some(diagnostics::environment_json(&redactor, environment)),
            consent: diagnostics::default_consent(),
            ..Sources::default()
        },
    };
    Gathered {
        system,
        sources,
        redactor,
    }
}

fn label(text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(content(0.7))
        .child(text)
}

fn field(input: Entity<Input>) -> impl IntoElement {
    div()
        .w_full()
        .px_3()
        .py(px(7.))
        .rounded_md()
        .border_1()
        .border_color(content(0.1))
        .bg(content(0.05))
        .text_size(px(13.))
        .text_color(content(1.))
        .child(input)
}

fn checkbox(
    id: impl Into<gpui::ElementId>,
    label: SharedString,
    checked: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::CheckBox)
        .aria_label(label.clone())
        .aria_toggled(if checked {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .child(
            div()
                .size(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(if checked {
                    accent_color()
                } else {
                    content(0.3)
                })
                .when(checked, |d| {
                    d.bg(accent_color())
                        .child(icon("check", 10.).text_color(gpui::white()))
                }),
        )
        .child(label)
}

fn size(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f32 / 1024.),
        b => format!("{:.1} MB", b as f32 / (1024. * 1024.)),
    }
}

impl ReportSheet {
    fn preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(bundle) = &self.bundle else {
            return div()
                .text_size(px(12.))
                .text_color(content(0.5))
                .child("Gathering and scrubbing diagnostics…")
                .into_any_element();
        };
        let mut rows = vec![row(REPORT_JSON, true)];
        if self.selection.attach {
            rows.extend(self.rows());
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(rows.into_iter().enumerate().map(|(ix, row)| {
                let name = row.name.clone();
                let bytes = bundle.entry(&name).map_or(0, |entry| entry.bytes.len());
                let shown = self.shown.as_deref() == Some(name.as_str()) && row.included;
                let removable = name != REPORT_JSON;
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(12.))
                            .child(if removable {
                                let toggled = name.clone();
                                checkbox(("report-entry", ix), name.clone().into(), row.included)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_entry(&toggled, cx)
                                    }))
                                    .into_any_element()
                            } else {
                                div().pl(px(22.)).child(name.clone()).into_any_element()
                            })
                            .child(div().flex_1())
                            .when(row.included, |d| {
                                let toggled = name.clone();
                                d.child(div().text_color(content(0.45)).child(size(bytes)))
                                    .child(
                                        div()
                                            .id(("report-show", ix))
                                            .role(Role::Button)
                                            .cursor_pointer()
                                            .text_color(accent_color())
                                            .child(if shown { "Hide" } else { "Show" })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.show_entry(&toggled, cx);
                                            })),
                                    )
                            }),
                    )
                    .when(shown, |d| d.child(self.entry_text()))
            }))
            .into_any_element()
    }

    /// The shown entry's lines, a viewport at a time. Long lines scroll
    /// sideways: every byte the zip holds can be read.
    fn entry_text(&self) -> impl IntoElement {
        let text = self.shown_lines.clone();
        let count = text.len();
        let longest = (0..count).max_by_key(|&ix| text[ix].chars().count());
        div()
            .id("report-entry-text")
            .h(px((count as f32 * PREVIEW_LINE).min(PREVIEW_HEIGHT)))
            .rounded_md()
            .bg(content(0.04))
            .px_2()
            .child(
                uniform_list("report-entry-lines", count, move |range, _, _| {
                    range
                        .map(|ix| {
                            div()
                                .id(("report-line", ix))
                                .debug_selector(move || format!("report-line-{ix}"))
                                .h(px(PREVIEW_LINE))
                                .text_size(px(11.))
                                .line_height(px(PREVIEW_LINE))
                                .font_family(PILOT)
                                .text_color(content(0.8))
                                .whitespace_nowrap()
                                .child(text[ix].clone())
                        })
                        .collect()
                })
                .with_width_from_item(longest)
                .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                .track_scroll(&self.preview_scroll)
                .size_full(),
            )
    }
}

impl Render for ReportSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let max_height = (window.viewport_size().height.as_f32() - 96.).max(240.);
        let attach = self.selection.attach;
        let busy = self.saving || self.building.is_some() || self.bundle.is_none();
        let actions = if let Some((saved, via)) = &self.saved {
            let opened = match via {
                Via::Email => "email",
                Via::GitHub => "GitHub issue",
            };
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .id("report-saved")
                        .debug_selector(|| "report-saved".into())
                        .role(Role::Status)
                        .text_size(px(12.))
                        .text_color(content(0.8))
                        .child(format!(
                            "Saved {saved} to Downloads. Drag it into the {opened} that just opened."
                        )),
                )
                .child(
                    crate::modal::primary_button("report-done", "Done")
                        .w_full()
                        .cursor_pointer()
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ReportEvent::Close))),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(crate::modal::disabled(
                    crate::modal::primary_button(
                        "report-email",
                        if self.saving {
                            "Saving…"
                        } else {
                            "Save zip and email"
                        },
                    )
                    .debug_selector(|| "report-email".into())
                    .w_full()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.submit(Via::Email, cx))),
                    busy,
                ))
                .child(crate::modal::disabled(
                    crate::modal::ghost_button("report-save", "Save zip and open GitHub")
                        .debug_selector(|| "report-save".into())
                        .w_full()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.submit(Via::GitHub, cx))),
                    busy,
                ))
                .child(
                    crate::modal::ghost_button("report-cancel", "Cancel")
                        .debug_selector(|| "report-cancel".into())
                        .w_full()
                        .cursor_pointer()
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ReportEvent::Close))),
                )
        };
        div()
            .id("report-sheet")
                        .debug_selector(|| "report-sheet".into())
            .role(Role::Dialog)
            .aria_label("Report an issue")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(ReportEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .max_h(px(max_height))
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .text_size(px(12.))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(content(1.))
                    .child("Report an issue"),
            )
            .child(div().flex().flex_col().gap_1p5().child(label("Title")).child(field(self.title.clone())))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(label("What happened?"))
                    .child(field(self.description.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(label("Steps to reproduce"))
                    .child(field(self.steps.clone())),
            )
            .when(self.shows_contact_email(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .child(label("Contact email"))
                        .child(field(self.email.clone())),
                )
            })
            .child(
                checkbox("report-attach", "Attach diagnostics (recommended)".into(), attach)
                    .text_size(px(12.))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_attach(!attach, cx))),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(content(0.5))
                    .child("Scrubbed on this computer. This is exactly what the zip will hold; untick anything you don't want to send."),
            )
            .child(self.preview(cx))
            .when(self.destination == Destination::Fallback, |d| {
                d.child(
                    div()
                        .id("report-public-warning")
                        .debug_selector(|| "report-public-warning".into())
                        .role(Role::Note)
                        .text_size(px(11.))
                        .line_height(px(16.))
                        .text_color(content(0.7))
                        .child(PUBLIC_WARNING),
                )
            })
            .when_some(self.error.clone(), |d, error| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .id("report-sheet-error")
                        .debug_selector(|| "report-sheet-error".into())
                                .role(Role::Alert)
                                .text_size(px(11.))
                                .text_color(rgb(0xf87171))
                                .child(error),
                        )
                        .when(self.bundle.is_some(), |d| {
                            d.child(
                                crate::modal::ghost_button("report-copy", "Copy report")
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.copy_report(cx))),
                            )
                        }),
                )
            })
            .child(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use std::io::Read;
    use std::sync::Mutex;

    fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut file = archive.by_index(i).unwrap();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                (file.name().to_owned(), bytes)
            })
            .collect()
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        logs: PathBuf,
        downloads: PathBuf,
        revealed: Arc<Mutex<Vec<PathBuf>>>,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        let crashes = gespan_core::diagnostics::crash_dir(&logs);
        std::fs::create_dir_all(&crashes).unwrap();
        let today = chrono::Utc::now();
        std::fs::write(
            logs.join(gespan_core::diagnostics::file_name(today.date_naive())),
            format!(
                "{{\"ts\":\"{}\",\"level\":\"error\",\"message\":\"git failed in /Users/alice/code\"}}\n",
                today.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            ),
        )
        .unwrap();
        std::fs::write(
            crashes.join("2026-10-25T09-15-30.123Z.json"),
            "{\"message\":\"boom\"}",
        )
        .unwrap();
        Fixture {
            downloads: dir.path().join("Downloads"),
            logs,
            _dir: dir,
            revealed: Arc::default(),
        }
    }

    fn sheet(
        f: &Fixture,
        destination: Destination,
        cx: &mut TestAppContext,
    ) -> Entity<ReportSheet> {
        let revealed = f.revealed.clone();
        let context = SheetContext {
            log_dir: Some(f.logs.clone()),
            projects: vec!["/Users/alice/code".into()],
            other_folders: Vec::new(),
            machine: diagnostics::Machine::default(),
            sessions_count: 3,
            version: "0.1.0".into(),
            channel: Channel::Release,
            downloads: Some(f.downloads.clone()),
            destination,
            facts: Some(Facts {
                system: System {
                    version: "0.1.0".into(),
                    build_channel: "release",
                    os: "macos".into(),
                    os_version: "15.1".into(),
                    arch: "aarch64".into(),
                    locale: "en-GB".into(),
                },
                providers: vec![("codex".into(), Some("0.48.0".into()))],
            }),
            reveal: Arc::new(move |path: &Path| {
                revealed.lock().unwrap().push(path.to_owned());
                Ok(())
            }),
        };
        let prefill = Prefill {
            title: String::new(),
            event_ids: vec!["0b7f1c2e-5a4d-4e8b-9f3a-2c6d1e0a9b47".into()],
        };
        let sheet = cx.new(|cx| ReportSheet::new(prefill, context, cx));
        cx.run_until_parked();
        sheet
    }

    fn type_into(sheet: &Entity<ReportSheet>, cx: &mut TestAppContext, title: &str, what: &str) {
        sheet.update(cx, |sheet, cx| {
            sheet
                .title
                .update(cx, |input, cx| input.set_text(title.into(), cx));
            sheet
                .description
                .update(cx, |input, cx| input.set_text(what.into(), cx));
        });
        settle(cx);
    }

    /// Lets typing settle and the rebuild finish.
    fn settle(cx: &mut TestAppContext) {
        cx.executor().advance_clock(REBUILD_DELAY);
        cx.run_until_parked();
    }

    /// Each keystroke used to deflate the whole log tail again; typing now
    /// settles first, and a keystroke cancels the build it supersedes.
    #[gpui::test]
    fn typing_quickly_builds_the_zip_once(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "W", "x");
        let before = sheet.read_with(cx, |sheet, _| sheet.builds);
        for title in ["Wo", "Wor", "Work"] {
            sheet.update(cx, |sheet, cx| {
                sheet
                    .title
                    .update(cx, |input, cx| input.set_text(title.into(), cx));
            });
            cx.run_until_parked();
        }
        sheet.read_with(cx, |sheet, _| {
            assert_eq!(sheet.builds, before, "nothing built while typing");
            assert_eq!(sheet.bundle().unwrap().report()["title"], "W");
        });
        settle(cx);
        sheet.read_with(cx, |sheet, _| {
            assert_eq!(sheet.builds, before + 1);
            assert_eq!(sheet.bundle().unwrap().report()["title"], "Work");
        });
    }

    #[gpui::test]
    fn preview_text_is_the_zip_bytes(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "Worktree fails", "Nothing happens.");
        sheet.read_with(cx, |sheet, _| {
            let bundle = sheet.bundle().expect("built once sources are in");
            let zipped = unzip(bundle.zip());
            let names: Vec<_> = zipped.iter().map(|(name, _)| name.as_str()).collect();
            assert_eq!(
                names,
                [
                    "report.json",
                    "logs.jsonl",
                    "crashes/2026-10-25T09-15-30.123Z.json",
                    "environment.json"
                ]
            );
            for (name, bytes) in &zipped {
                assert_eq!(
                    sheet.preview_text(name).unwrap().as_bytes(),
                    &bytes[..],
                    "{name}"
                );
            }
            let logs = sheet.preview_text("logs.jsonl").unwrap();
            assert!(logs.contains("git failed in <project-1>"), "{logs}");
            assert!(!logs.contains("alice"));
        });
    }

    #[gpui::test]
    fn typing_updates_report_json_in_the_preview(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "First", "x");
        type_into(&sheet, cx, "Second title", "what");
        sheet.read_with(cx, |sheet, _| {
            let report: serde_json::Value =
                serde_json::from_str(&sheet.preview_text("report.json").unwrap()).unwrap();
            assert_eq!(report["title"], "Second title");
            assert_eq!(report["description"], "what");
            assert_eq!(
                report["event_ids"],
                serde_json::json!(["0b7f1c2e-5a4d-4e8b-9f3a-2c6d1e0a9b47"])
            );
        });
    }

    #[gpui::test]
    fn the_bundle_is_rebuilt_off_the_ui_thread(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "Before", "x");
        sheet.update(cx, |sheet, cx| {
            sheet
                .title
                .update(cx, |input, cx| input.set_text("After".into(), cx));
        });
        // The change has been handled, but the zip is rebuilt in the
        // background: the UI thread hasn't built it yet.
        sheet.read_with(cx, |sheet, _| {
            assert_eq!(sheet.bundle().unwrap().report()["title"], "Before")
        });
        settle(cx);
        sheet.read_with(cx, |sheet, _| {
            assert_eq!(sheet.bundle().unwrap().report()["title"], "After")
        });
    }

    #[gpui::test]
    fn long_lines_scroll_instead_of_being_cut(cx: &mut TestAppContext) {
        let f = fixture();
        let long = format!("{{\"message\":\"{}\"}}", "x".repeat(400));
        let day = chrono::Utc::now().date_naive();
        std::fs::write(f.logs.join(gespan_core::diagnostics::file_name(day)), &long).unwrap();
        let sheet = sheet(&f, Destination::Fallback, cx);
        let window = cx.add_window(|_, _| Holder(sheet.clone()));
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        sheet.update(visual, |sheet, cx| sheet.show_entry("logs.jsonl", cx));
        visual.run_until_parked();
        visual.refresh().unwrap();
        let line = visual
            .debug_bounds("report-line-0")
            .expect("the line is drawn");
        // 400+ monospace characters at 11 px are far wider than the sheet.
        assert!(line.size.width > gpui::px(1500.), "{:?}", line.size.width);
    }

    #[gpui::test]
    fn closing_while_saving_still_opens_github(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "t", "d");
        sheet.update(cx, |sheet, cx| sheet.submit(Via::GitHub, cx));
        drop(sheet);
        cx.run_until_parked();
        assert!(
            cx.opened_url().is_some(),
            "the zip is saved, so the issue opens"
        );
        assert_eq!(std::fs::read_dir(&f.downloads).unwrap().count(), 1);
    }

    struct Holder(Entity<ReportSheet>);
    impl Render for Holder {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    #[gpui::test]
    fn unticking_attach_leaves_only_report_json(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "t", "d");
        sheet.update(cx, |sheet, cx| sheet.set_attach(false, cx));
        cx.run_until_parked();
        sheet.read_with(cx, |sheet, _| {
            let bundle = sheet.bundle().unwrap();
            assert_eq!(bundle.entries.len(), 1);
            assert_eq!(unzip(bundle.zip()).len(), 1);
        });
    }

    #[gpui::test]
    fn removing_an_entry_drops_it_from_files(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "t", "d");
        sheet.update(cx, |sheet, cx| {
            sheet.toggle_entry("logs.jsonl", cx);
            sheet.toggle_entry("crashes/2026-10-25T09-15-30.123Z.json", cx);
        });
        cx.run_until_parked();
        sheet.read_with(cx, |sheet, _| {
            assert_eq!(
                sheet.bundle().unwrap().report()["files"],
                serde_json::json!(["environment.json"])
            );
            // Removed entries stay listed, so they can be put back.
            assert!(
                sheet
                    .rows()
                    .iter()
                    .any(|row| row.name == "logs.jsonl" && !row.included)
            );
        });
        sheet.update(cx, |sheet, cx| sheet.toggle_entry("logs.jsonl", cx));
        cx.run_until_parked();
        sheet.read_with(cx, |sheet, _| {
            assert_eq!(
                sheet.bundle().unwrap().report()["files"],
                serde_json::json!(["logs.jsonl", "environment.json"])
            );
        });
    }

    #[gpui::test]
    fn title_and_description_are_required(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "  ", "Something broke");
        sheet.update(cx, |sheet, cx| sheet.submit(Via::GitHub, cx));
        cx.run_until_parked();
        sheet.read_with(cx, |sheet, _| {
            assert_eq!(sheet.error.as_deref(), Some(REQUIRED));
            assert!(sheet.saved.is_none());
        });
        assert!(!f.downloads.exists());
        assert!(cx.opened_url().is_none());
    }

    #[gpui::test]
    fn contact_email_is_hidden_in_fallback(cx: &mut TestAppContext) {
        let f = fixture();
        let fallback = sheet(&f, Destination::Fallback, cx);
        fallback.read_with(cx, |sheet, _| assert!(!sheet.shows_contact_email()));
        let intake = sheet(&f, Destination::Intake, cx);
        intake.read_with(cx, |sheet, _| assert!(sheet.shows_contact_email()));
    }

    #[gpui::test]
    fn submit_saves_the_previewed_zip_and_opens_the_prefilled_url(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "Worktree fails", "Nothing happens.");
        let previewed = sheet.read_with(cx, |sheet, _| sheet.bundle().unwrap().clone());
        sheet.update(cx, |sheet, cx| sheet.submit(Via::GitHub, cx));
        cx.run_until_parked();
        let saved = f.downloads.join(&previewed.file_name);
        assert_eq!(std::fs::read(&saved).unwrap(), previewed.zip());
        assert_eq!(*f.revealed.lock().unwrap(), vec![saved]);
        let url = url::Url::parse(&cx.opened_url().expect("opens GitHub")).unwrap();
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs["title"], "Worktree fails");
        assert_eq!(pairs["what-happened"], "Nothing happens.");
        assert_eq!(pairs["os"], "macOS");
        // The crash record now lives in the saved zip.
        assert!(
            !gespan_core::diagnostics::crash_dir(&f.logs)
                .join("2026-10-25T09-15-30.123Z.json")
                .exists()
        );
        sheet.read_with(cx, |sheet, _| {
            assert_eq!(
                sheet.saved,
                Some((previewed.file_name.clone(), Via::GitHub))
            );
            assert!(sheet.error.is_none());
        });
    }

    /// The opened `mailto:` link's address, subject and body.
    fn opened_mail(cx: &TestAppContext) -> (String, String, String) {
        let opened = cx.opened_url().expect("opens the mail app");
        let url = url::Url::parse(&opened).unwrap();
        assert_eq!(url.scheme(), "mailto", "{opened}");
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        (
            url.path().to_owned(),
            pairs["subject"].clone(),
            pairs["body"].clone(),
        )
    }

    #[gpui::test]
    fn email_saves_the_previewed_zip_and_opens_a_mail_to_the_maintainer(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "Worktree fails", "Nothing happens.");
        let previewed = sheet.read_with(cx, |sheet, _| sheet.bundle().unwrap().clone());
        sheet.update(cx, |sheet, cx| sheet.submit(Via::Email, cx));
        cx.run_until_parked();
        let saved = f.downloads.join(&previewed.file_name);
        assert_eq!(std::fs::read(&saved).unwrap(), previewed.zip());
        assert_eq!(*f.revealed.lock().unwrap(), vec![saved]);
        let (to, subject, body) = opened_mail(cx);
        assert_eq!(to, "itparser%2Bgespan@gmail.com");
        assert_eq!(subject, "[Gespan 0.1.0] Worktree fails");
        assert!(
            body.starts_with("What happened:\r\nNothing happens.\r\n"),
            "{body}"
        );
        assert!(body.contains("OS: macOS 15.1 (aarch64)"), "{body}");
        assert!(body.contains("Event IDs: 0b7f1c2e-5a4d-4e8b-9f3a-2c6d1e0a9b47"));
        assert!(body.contains(&previewed.file_name), "{body}");
        // The crash record now lives in the saved zip, as with GitHub.
        assert!(
            !gespan_core::diagnostics::crash_dir(&f.logs)
                .join("2026-10-25T09-15-30.123Z.json")
                .exists()
        );
        sheet.read_with(cx, |sheet, _| {
            assert_eq!(sheet.saved, Some((previewed.file_name.clone(), Via::Email)));
            assert!(sheet.error.is_none());
        });
    }

    #[gpui::test]
    fn email_names_the_file_actually_saved(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "t", "d");
        let name = sheet.read_with(cx, |sheet, _| sheet.bundle().unwrap().file_name.clone());
        std::fs::create_dir_all(&f.downloads).unwrap();
        std::fs::write(f.downloads.join(&name), b"an older report").unwrap();
        sheet.update(cx, |sheet, cx| sheet.submit(Via::Email, cx));
        cx.run_until_parked();
        let suffixed = name.replace(".zip", "-1.zip");
        let (_, _, body) = opened_mail(cx);
        assert!(
            body.contains(&format!("attach {suffixed} from")),
            "the email must point at the new zip, not the older one: {body}"
        );
    }

    #[gpui::test]
    fn email_keeps_scrubbed_text_out_of_the_mail(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(
            &sheet,
            cx,
            "git fails in /Users/alice/code",
            "token sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789",
        );
        sheet.update(cx, |sheet, cx| sheet.submit(Via::Email, cx));
        cx.run_until_parked();
        let (_, subject, body) = opened_mail(cx);
        for text in [&subject, &body] {
            assert!(
                !text.contains("alice") && !text.contains("sk-ant"),
                "{text}"
            );
        }
        assert!(subject.ends_with("git fails in <project-1>"), "{subject}");
    }

    #[gpui::test]
    fn email_also_needs_a_title_and_what_happened(cx: &mut TestAppContext) {
        let f = fixture();
        let sheet = sheet(&f, Destination::Fallback, cx);
        type_into(&sheet, cx, "A title", "   ");
        sheet.update(cx, |sheet, cx| sheet.submit(Via::Email, cx));
        cx.run_until_parked();
        sheet.read_with(cx, |sheet, _| {
            assert_eq!(sheet.error.as_deref(), Some(REQUIRED));
            assert!(sheet.saved.is_none());
        });
        assert!(!f.downloads.exists());
        assert!(cx.opened_url().is_none());
    }
}
