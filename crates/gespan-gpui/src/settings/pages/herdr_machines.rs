//! The herdr machines list under Settings → Connections. herdr keeps the
//! list; adding and upgrading run herdr in a terminal tab, where it asks its
//! own questions.
use crate::{
    input::{self, Input},
    settings::{
        Section, SettingsEvent, SettingsView,
        widgets::{glass_secondary, red},
    },
    theme::*,
};
use gespan_core::machines::{self, HERDR_VERSION, Machine, MachineView, Online};
use gpui::{
    AnyElement, Context, Div, Entity, FontWeight, Role, SharedString, div, prelude::*, px, rgb,
};
use std::collections::{HashMap, HashSet};

pub(super) const TARGET: &str = "herdr ssh target";
const EMPTY: &str = "Add one to run agents on another computer.";
const NOT_BUNDLED: &str = "herdr isn't bundled with this build";

#[derive(Default)]
pub(in crate::settings) struct HerdrMachinesState {
    /// Looking up the add command.
    pub(super) adding: bool,
    /// The add field's error.
    error: Option<String>,
    /// The machine whose removal is being confirmed.
    pub(super) removing: Option<String>,
    /// Machines with a herdr call in flight.
    busy: HashSet<String>,
    /// Machines whose herdr build is being fetched for an upgrade.
    preparing: HashSet<String>,
    /// The last failure per machine.
    pub(super) errors: HashMap<String, String>,
}

fn remove_question(label: &str) -> String {
    format!("Remove {label} from Gespan and herdr? Its agents keep running on the machine.")
}

/// The status line under a machine's name.
pub(super) fn status_line(view: &MachineView) -> String {
    if !view.machine.enabled {
        return "Off".into();
    }
    if let Some(version) = &view.version_mismatch {
        return format!("herdr {version} on this machine; Gespan uses {HERDR_VERSION}");
    }
    match &view.snapshot.online {
        Online::Online => format!(
            "Online · herdr {}",
            view.snapshot.version.as_deref().unwrap_or(HERDR_VERSION)
        ),
        Online::Connecting => "Connecting…".into(),
        Online::Offline { retry_in, .. } => {
            let secs = retry_in.as_secs_f64().ceil().max(1.) as u64;
            format!("Offline · retrying in {secs}s")
        }
        Online::NeedsSignIn { hint } => format!("Sign-in needed: run {hint} in a terminal"),
        Online::HerdrMissing => "herdr not installed".into(),
        Online::HerdrTooOld => "herdr on this machine is too old for Gespan".into(),
    }
}

/// The pill colour for a machine's status: online is fine, off is quiet,
/// offline is a problem, and the rest wait on someone.
pub(super) fn status_tone(view: &MachineView) -> crate::glass::Tone {
    use crate::glass::Tone;
    if !view.machine.enabled {
        return Tone::Muted;
    }
    if view.version_mismatch.is_some() {
        return Tone::Waiting;
    }
    match view.snapshot.online {
        Online::Online => Tone::Ok,
        Online::Connecting => Tone::Running,
        Online::Offline { .. } => Tone::Bad,
        _ => Tone::Waiting,
    }
}

/// The upgrade button's text. A machine without herdr has nothing to
/// upgrade, and one running a later herdr would go back to the pinned one,
/// so for both the button says what it installs rather than Upgrade.
fn upgrade_label(view: &MachineView) -> String {
    let missing = view.snapshot.online == Online::HerdrMissing;
    match &view.version_mismatch {
        Some(version) if machines::is_newer_than_pinned(version) => {
            format!("Install herdr {HERDR_VERSION}")
        }
        _ if missing => format!("Install herdr {HERDR_VERSION}"),
        _ => "Upgrade".into(),
    }
}

fn paragraph(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .line_height(px(19.5))
        .text_color(content(0.45))
        .child(text.into())
}

enum Upgrade {
    Run(String),
    NotBundled,
    Failed(String),
}

impl SettingsView {
    /// Re-reads the list whenever Connections opens.
    pub(in crate::settings) fn watch_herdr_machines(&mut self, cx: &mut Context<Self>) {
        if self.section == Section::Devices && crate::machines::visible(cx) {
            crate::machines::reload(cx);
        }
    }

    /// Forgets state for machines no longer listed, so a late report about
    /// a removed machine leaves nothing behind.
    pub(in crate::settings) fn herdr_machines_changed(
        &mut self,
        machines: &Entity<crate::machines::Machines>,
        cx: &mut Context<Self>,
    ) {
        let ids: HashSet<String> = machines
            .read(cx)
            .views()
            .iter()
            .map(|v| v.machine.id.clone())
            .collect();
        let herdr = &mut self.herdr;
        herdr.errors.retain(|id, _| ids.contains(id));
        herdr.busy.retain(|id| ids.contains(id));
        herdr.preparing.retain(|id| ids.contains(id));
        if herdr.removing.as_ref().is_some_and(|id| !ids.contains(id)) {
            herdr.removing = None;
        }
        cx.notify();
    }

    fn herdr_target_field(&mut self, cx: &mut Context<Self>) -> Entity<Input> {
        if let Some(input) = self.fields.get(TARGET) {
            return input.clone();
        }
        let input = cx.new(|cx| {
            let mut input = Input::field(
                "herdr machine SSH target",
                "ssh target, for example user@host",
                content(0.35),
                cx,
            );
            input.set_font_size(13.);
            input
        });
        self._subscriptions
            .push(cx.subscribe(&input, |this, _, _: &input::Changed, cx| {
                this.herdr.error = None;
                cx.notify()
            }));
        self._subscriptions
            .push(cx.subscribe(&input, |this, _, _: &input::Submitted, cx| {
                this.add_herdr_machine(cx)
            }));
        self.fields.insert(TARGET, input.clone());
        input
    }

    fn herdr_target(&self, cx: &Context<Self>) -> String {
        self.fields
            .get(TARGET)
            .map(|input| input.read(cx).content.trim().to_owned())
            .unwrap_or_default()
    }

    fn herdr_found(&self, cx: &Context<Self>) -> Option<bool> {
        crate::machines::get(cx).and_then(|m| m.read(cx).herdr_found())
    }

    fn add_herdr_machine(&mut self, cx: &mut Context<Self>) {
        let target = self.herdr_target(cx);
        if self.herdr.adding || target.is_empty() || self.herdr_found(cx) == Some(false) {
            return;
        }
        self.herdr.adding = true;
        self.herdr.error = None;
        let lookup = target.clone();
        let work = cx
            .background_executor()
            .spawn(async move { machines::add_command(&lookup, None) });
        cx.spawn(async move |this, cx| {
            let command = work.await;
            let _ = this.update(cx, |this, cx| {
                this.herdr.adding = false;
                match command {
                    Some(command) => {
                        if let Some(input) = this.fields.get(TARGET) {
                            input.update(cx, |input, cx| input.set_text(String::new(), cx));
                        }
                        cx.emit(SettingsEvent::RunMachineCommand {
                            title: format!("Add {target}"),
                            command,
                        });
                    }
                    None => {
                        if let Some(machines) = crate::machines::get(cx) {
                            machines.update(cx, |m, cx| m.set_herdr_found(false, cx));
                        }
                        this.herdr.error = Some(NOT_BUNDLED.into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn upgrade_herdr_machine(&mut self, machine: Machine, cx: &mut Context<Self>) {
        let id = machine.id.clone();
        if !self.herdr.preparing.insert(id.clone()) {
            return;
        }
        self.herdr.errors.remove(&id);
        let label = machine.label.clone();
        let work = cx.background_executor().spawn(async move {
            match machines::prepare_remote_binary(&machine) {
                Err(error) => Upgrade::Failed(error),
                Ok(path) => match machines::upgrade_command(&machine, &path) {
                    Some(command) => Upgrade::Run(command),
                    None => Upgrade::NotBundled,
                },
            }
        });
        cx.spawn(async move |this, cx| {
            let upgrade = work.await;
            let _ = this.update(cx, |this, cx| {
                // Removed meanwhile: nothing to show it on.
                if !this.herdr.preparing.remove(&id) {
                    return;
                }
                match upgrade {
                    Upgrade::Run(command) => cx.emit(SettingsEvent::RunMachineCommand {
                        title: format!("Upgrade {label}"),
                        command,
                    }),
                    Upgrade::NotBundled => {
                        this.herdr.errors.insert(id, NOT_BUNDLED.into());
                    }
                    Upgrade::Failed(error) => {
                        this.herdr
                            .errors
                            .insert(id, format!("Couldn't prepare herdr: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Runs one blocking herdr change off the UI thread, then reloads.
    fn change_herdr_machine(
        &mut self,
        id: String,
        failure: String,
        change: impl FnOnce() -> Result<(), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if !self.herdr.busy.insert(id.clone()) {
            return;
        }
        self.herdr.errors.remove(&id);
        let work = cx.background_executor().spawn(async move { change() });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.herdr.busy.remove(&id);
                match result {
                    Ok(()) => {
                        if this.herdr.removing.as_ref() == Some(&id) {
                            this.herdr.removing = None;
                        }
                    }
                    Err(error) => {
                        this.herdr.errors.insert(id, format!("{failure}: {error}"));
                    }
                }
                crate::machines::reload(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn herdr_switch(&self, view: &MachineView, cx: &mut Context<Self>) -> AnyElement {
        let id = view.machine.id.clone();
        let on = view.machine.enabled;
        let disabled = self.herdr.busy.contains(&id);
        let label = view.machine.label.clone();
        let selector = id.clone();
        div()
            .id(SharedString::from(format!("herdr-enabled-{id}")))
            .debug_selector(move || format!("herdr-enabled-{selector}"))
            .role(Role::Switch)
            .aria_label(format!("Use {label}"))
            .aria_toggled(if on {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .relative()
            .h(px(20.))
            .w(px(36.))
            .flex_shrink_0()
            .rounded_full()
            .bg(if on { accent_color() } else { content(0.2) })
            .when(disabled, |d| d.opacity(0.4))
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left(px(if on { 18. } else { 2. }))
                    .size(px(16.))
                    .rounded_full()
                    .bg(rgb(0xffffff)),
            )
            .when(!disabled, |d| {
                d.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let (id, failure) = (id.clone(), format!("Couldn't change {label}"));
                        let target = id.clone();
                        this.change_herdr_machine(
                            id,
                            failure,
                            move || machines::set_enabled(&target, !on),
                            cx,
                        );
                    }))
            })
            .into_any_element()
    }

    fn herdr_machine_row(&self, view: &MachineView, cx: &mut Context<Self>) -> Div {
        let id = view.machine.id.clone();
        let label = view.machine.label.clone();
        let preparing = self.herdr.preparing.contains(&id);
        let busy = self.herdr.busy.contains(&id) || preparing;
        let upgrade = view.version_mismatch.is_some() || view.needs_install;
        let status = if preparing {
            format!("Preparing herdr {HERDR_VERSION}…")
        } else {
            status_line(view)
        };
        let (row, upgrade_id, remove_id) = (id.clone(), id.clone(), id.clone());
        let machine = view.machine.clone();
        let online = view.machine.enabled && view.snapshot.online == Online::Online;
        // Only an enabled machine has a connection to retry.
        let retry = view.machine.enabled && !online;
        let tone = if preparing {
            crate::glass::Tone::Running
        } else {
            status_tone(view)
        };
        div()
            .border_b_1()
            .border_color(crate::glass::hairline())
            .child(
                div()
                    .id(SharedString::from(format!("herdr-machine-{id}")))
                    .debug_selector(move || format!("herdr-machine-{row}"))
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .px_4()
                    .py(px(14.))
                    .child(
                        div()
                            .size(px(32.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(9.))
                            .bg(content(0.07))
                            .border_1()
                            .border_color(crate::glass::edge())
                            .child(icon("globe", 16.).text_color(content(if online {
                                0.8
                            } else {
                                0.45
                            }))),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(label.clone()),
                                    )
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("herdr-status-{id}")))
                                            .role(Role::Status)
                                            .min_w_0()
                                            .child(crate::glass::status(status, tone)),
                                    ),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .truncate()
                                    .font_family(crate::fonts::monospace())
                                    .text_size(px(11.5))
                                    .text_color(content(0.55))
                                    .child(view.machine.target.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(retry, |d| {
                                let (selector, retry_id) = (id.clone(), id.clone());
                                d.child(
                                    glass_secondary(
                                        SharedString::from(format!("herdr-retry-{id}")),
                                        format!("Retry {label}"),
                                        false,
                                        false,
                                    )
                                    .debug_selector(move || format!("herdr-retry-{selector}"))
                                    .child("Retry")
                                    .on_click(cx.listener(
                                        move |_, _, _, cx| {
                                            crate::machines::refresh(&retry_id, cx);
                                        },
                                    )),
                                )
                            })
                            .when(upgrade, |d| {
                                let selector = upgrade_id.clone();
                                let text = upgrade_label(view);
                                let aria = match text.as_str() {
                                    "Upgrade" => format!("Upgrade herdr on {label}"),
                                    install => format!("{install} on {label}"),
                                };
                                d.child(
                                    glass_secondary(
                                        SharedString::from(format!("herdr-upgrade-{id}")),
                                        aria,
                                        false,
                                        busy,
                                    )
                                    .debug_selector(move || format!("herdr-upgrade-{selector}"))
                                    .child(text)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            if !this.herdr.busy.contains(&machine.id) {
                                                this.upgrade_herdr_machine(machine.clone(), cx);
                                            }
                                        },
                                    )),
                                )
                            })
                            .child(self.herdr_switch(view, cx))
                            .child({
                                let selector = remove_id.clone();
                                glass_secondary(
                                    SharedString::from(format!("herdr-remove-{id}")),
                                    format!("Remove {label}"),
                                    true,
                                    busy,
                                )
                                .debug_selector(move || format!("herdr-remove-{selector}"))
                                .child("Remove")
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if this.herdr.busy.contains(&remove_id) {
                                            return;
                                        }
                                        this.herdr.removing = Some(remove_id.clone());
                                        cx.notify();
                                    },
                                ))
                            }),
                    ),
            )
            .when_some(self.herdr.errors.get(&id), |d, error| {
                d.child(
                    div()
                        .id(SharedString::from(format!("herdr-error-{id}")))
                        .role(Role::Alert)
                        .mx_4()
                        .mb_4()
                        .rounded_lg()
                        .bg(red(0.05))
                        .p_3()
                        .text_size(px(12.))
                        .line_height(px(19.5))
                        .text_color(red(1.))
                        .child(error.clone()),
                )
            })
            .when(self.herdr.removing.as_ref() == Some(&id), |d| {
                d.child(self.herdr_remove_confirmation(view, cx))
            })
    }

    fn herdr_remove_confirmation(&self, view: &MachineView, cx: &mut Context<Self>) -> AnyElement {
        let id = view.machine.id.clone();
        let label = view.machine.label.clone();
        let disabled = self.herdr.busy.contains(&id);
        div()
            .id("herdr-remove-confirm")
            .debug_selector(|| "herdr-remove-confirm".into())
            .role(Role::Group)
            .aria_label(format!("Confirm removing {label}"))
            .border_t_1()
            .border_color(content(0.1))
            .bg(content(0.03))
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(remove_question(&label)),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        glass_secondary(
                            "herdr-remove-yes",
                            format!("Remove {label}"),
                            true,
                            disabled,
                        )
                        .debug_selector(|| "herdr-remove-yes".into())
                        .child("Remove")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let target = id.clone();
                            this.change_herdr_machine(
                                id.clone(),
                                format!("Couldn't remove {label}"),
                                move || machines::remove(&target),
                                cx,
                            );
                        })),
                    )
                    .child(
                        glass_secondary("herdr-remove-cancel", "Cancel", false, disabled)
                            .debug_selector(|| "herdr-remove-cancel".into())
                            .child("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.herdr.removing = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(in crate::settings) fn herdr_machines_page(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if !crate::machines::visible(cx) {
            return Vec::new();
        }
        let Some(machines) = crate::machines::get(cx) else {
            return Vec::new();
        };
        let (views, list_error, loaded, found) = {
            let m = machines.read(cx);
            (
                m.views().to_vec(),
                m.list_error().map(str::to_owned),
                m.loaded(),
                m.herdr_found(),
            )
        };
        let mut page = vec![
            div()
                .id("herdr-machines")
                .debug_selector(|| "herdr-machines".into())
                .relative()
                .child(self.reveal_probe("herdr-machines"))
                .px(px(4.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("herdr machines"),
                )
                .into_any_element(),
        ];
        // One card: each machine (or the empty line), then the field that
        // adds one as its last row.
        let mut card = crate::glass::card().overflow_hidden();
        if !views.is_empty() {
            for view in &views {
                card = card.child(self.herdr_machine_row(view, cx));
            }
        } else if loaded && list_error.is_none() {
            card = card.child(crate::settings::widgets::empty(
                "herdr-machines-empty",
                "terminal",
                "No herdr machines yet",
                EMPTY,
            ));
        }
        let input = self.herdr_target_field(cx);
        let not_bundled = found == Some(false);
        let add_disabled = self.herdr.adding || not_bundled || self.herdr_target(cx).is_empty();
        card = card.child(
            div()
                .debug_selector(|| "herdr-add-row".into())
                // Machine rows draw their own rule beneath them.
                .when(views.is_empty() && loaded, |d| {
                    d.border_t_1().border_color(crate::glass::hairline())
                })
                .px_4()
                .py(px(12.))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    crate::glass::field()
                        .min_w_0()
                        .flex_1()
                        .child(div().min_w_0().flex_1().child(input)),
                )
                .child(
                    glass_secondary("herdr-add", "Add", false, add_disabled)
                        .debug_selector(|| "herdr-add".into())
                        .h(px(32.))
                        .child(icon("plus", 14.))
                        .child("Add")
                        .on_click(cx.listener(|this, _, _, cx| this.add_herdr_machine(cx))),
                ),
        );
        page.push(card.into_any_element());
        if let Some(error) = list_error {
            page.push(
                div()
                    .id("herdr-machines-error")
                    .debug_selector(|| "herdr-machines-error".into())
                    .role(Role::Alert)
                    .rounded_lg()
                    .bg(red(0.05))
                    .p_3()
                    .text_size(px(12.))
                    .line_height(px(19.5))
                    .text_color(red(1.))
                    .child(error)
                    .into_any_element(),
            );
        }
        if self.herdr.adding {
            page.push(
                paragraph(format!("Preparing herdr {HERDR_VERSION}…"))
                    .mt_2()
                    .id("herdr-add-preparing")
                    .debug_selector(|| "herdr-add-preparing".into())
                    .into_any_element(),
            );
        } else if not_bundled {
            page.push(
                paragraph(NOT_BUNDLED)
                    .mt_2()
                    .id("herdr-not-bundled")
                    .debug_selector(|| "herdr-not-bundled".into())
                    .into_any_element(),
            );
        } else if let Some(error) = self.herdr.error.clone() {
            page.push(
                paragraph(error)
                    .mt_2()
                    .text_color(red(1.))
                    .id("herdr-add-error")
                    .role(Role::Alert)
                    .into_any_element(),
            );
        }
        page
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Preferences, Section, SettingsContext, SettingsEvent};
    use gespan_core::machines::{HERDR_VERSION, Machine, Online, Snapshot};
    use gpui::{
        AppContext, Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle,
        WindowOptions, point, px, size,
    };
    use std::{cell::RefCell, rc::Rc, time::Duration};

    fn machine(id: &str, online: Online, version: Option<&str>, enabled: bool) -> MachineView {
        let needs_install = matches!(online, Online::HerdrMissing | Online::HerdrTooOld);
        MachineView {
            machine: Machine {
                id: id.into(),
                label: format!("{id} box"),
                target: format!("me@{id}"),
                session: "default".into(),
                enabled,
            },
            version_mismatch: version.filter(|v| *v != HERDR_VERSION).map(str::to_owned),
            needs_install,
            snapshot: Snapshot {
                online,
                version: version.map(str::to_owned),
                ..Default::default()
            },
        }
    }

    /// The (title, command) pairs Settings asked the shell to run.
    type Commands = Rc<RefCell<Vec<(String, String)>>>;

    fn window(
        views: Vec<MachineView>,
        shown: bool,
        cx: &mut TestAppContext,
    ) -> (WindowHandle<SettingsView>, Commands) {
        cx.update(|cx| {
            if shown {
                crate::machines::show_for_test(cx);
            }
            crate::machines::install_for_test(views, cx);
        });
        let window = cx.update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1200.), px(2400.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let mut view = SettingsView::new(
                            Section::Devices,
                            Preferences::default(),
                            SettingsContext::default(),
                            cx,
                        );
                        view.discovery_task = None;
                        view.discovery_loading = false;
                        view
                    })
                },
            )
            .unwrap()
        });
        let commands = Rc::new(RefCell::new(Vec::new()));
        let seen = commands.clone();
        let view = window.update(cx, |_, _, cx| cx.entity()).unwrap();
        cx.update(|cx| {
            cx.subscribe(&view, move |_, event: &SettingsEvent, _| {
                if let SettingsEvent::RunMachineCommand { title, command } = event {
                    seen.borrow_mut().push((title.clone(), command.clone()));
                }
            })
            .detach()
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        (window, commands)
    }

    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on the page"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
    }

    #[test]
    fn herdr_machines_status_line_names_every_connection_state() {
        let line =
            |online, version: Option<&str>| status_line(&machine("m", online, version, true));
        assert_eq!(
            line(Online::Online, Some(HERDR_VERSION)),
            format!("Online · herdr {HERDR_VERSION}")
        );
        assert_eq!(line(Online::Connecting, None), "Connecting…");
        assert_eq!(
            line(
                Online::Offline {
                    reason: "Connection refused".into(),
                    retry_in: Duration::from_secs(8),
                },
                None
            ),
            "Offline · retrying in 8s"
        );
        // A wait under a second still reads as a whole second.
        assert_eq!(
            line(
                Online::Offline {
                    reason: String::new(),
                    retry_in: Duration::from_millis(300),
                },
                None
            ),
            "Offline · retrying in 1s"
        );
        assert_eq!(
            line(
                Online::NeedsSignIn {
                    hint: "ssh me@m".into()
                },
                None
            ),
            "Sign-in needed: run ssh me@m in a terminal"
        );
        assert_eq!(line(Online::HerdrMissing, None), "herdr not installed");
        assert_eq!(
            line(Online::HerdrTooOld, None),
            "herdr on this machine is too old for Gespan"
        );
        // A different version wins over Online: the user must act on it.
        assert_eq!(
            line(Online::Online, Some("0.9.4")),
            format!("herdr 0.9.4 on this machine; Gespan uses {HERDR_VERSION}")
        );
        // A machine that is off has no connection, so it is not connecting.
        assert_eq!(
            status_line(&machine("m", Online::Connecting, None, false)),
            "Off"
        );
    }

    #[test]
    fn herdr_machines_upgrade_reads_install_when_herdr_is_missing_or_ahead() {
        let label =
            |version: Option<&str>, online| upgrade_label(&machine("m", online, version, true));
        assert_eq!(label(Some("0.9.4"), Online::Online), "Upgrade");
        assert_eq!(
            label(None, Online::HerdrMissing),
            format!("Install herdr {HERDR_VERSION}")
        );
        assert_eq!(label(None, Online::HerdrTooOld), "Upgrade");
        // Numbers, not text: 0.9.10 is ahead of 0.9.5.
        for ahead in ["0.9.10", "0.10.0", "1.0.0"] {
            assert_eq!(
                label(Some(ahead), Online::Online),
                format!("Install herdr {HERDR_VERSION}"),
                "{ahead}"
            );
        }
    }

    #[gpui::test]
    fn herdr_machines_empty_state_invites_adding_one(cx: &mut TestAppContext) {
        let (window, _) = window(Vec::new(), true, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("herdr-machines").is_some());
        assert!(visual.debug_bounds("herdr-machines-empty").is_some());
        assert!(visual.debug_bounds("herdr-add-row").is_some());
    }

    #[gpui::test]
    fn herdr_machines_rows_offer_upgrade_only_when_herdr_differs(cx: &mut TestAppContext) {
        let views = vec![
            machine("ok", Online::Online, Some(HERDR_VERSION), true),
            machine("old", Online::Online, Some("0.9.4"), true),
            machine("bare", Online::HerdrMissing, None, true),
            machine("stale", Online::HerdrTooOld, None, true),
            machine(
                "away",
                Online::Offline {
                    reason: "timed out".into(),
                    retry_in: Duration::from_secs(8),
                },
                Some(HERDR_VERSION),
                true,
            ),
        ];
        let (window, _) = window(views, true, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("herdr-machines-empty").is_none());
        for row in [
            "herdr-machine-ok",
            "herdr-machine-old",
            "herdr-machine-bare",
            "herdr-machine-stale",
            "herdr-machine-away",
        ] {
            assert!(visual.debug_bounds(row).is_some(), "{row}");
        }
        assert!(visual.debug_bounds("herdr-upgrade-old").is_some());
        assert!(visual.debug_bounds("herdr-upgrade-bare").is_some());
        assert!(visual.debug_bounds("herdr-upgrade-stale").is_some());
        assert!(visual.debug_bounds("herdr-upgrade-ok").is_none());
        assert!(visual.debug_bounds("herdr-upgrade-away").is_none());
        // Every row can be turned off and removed.
        assert!(visual.debug_bounds("herdr-enabled-ok").is_some());
        assert!(visual.debug_bounds("herdr-remove-away").is_some());
    }

    #[gpui::test]
    fn herdr_machines_retry_is_offered_only_while_a_machine_waits(cx: &mut TestAppContext) {
        let views = vec![
            machine("ok", Online::Online, Some(HERDR_VERSION), true),
            machine(
                "signin",
                Online::NeedsSignIn {
                    hint: "ssh me@signin".into(),
                },
                None,
                true,
            ),
            machine(
                "away",
                Online::Offline {
                    reason: "timed out".into(),
                    retry_in: Duration::from_secs(8),
                },
                None,
                true,
            ),
            machine("bare", Online::HerdrMissing, None, true),
            // Off: there is no connection to retry.
            machine("off", Online::Connecting, None, false),
        ];
        let (window, _) = window(views, true, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        for retry in ["herdr-retry-signin", "herdr-retry-away", "herdr-retry-bare"] {
            assert!(visual.debug_bounds(retry).is_some(), "{retry}");
        }
        assert!(visual.debug_bounds("herdr-retry-ok").is_none());
        assert!(visual.debug_bounds("herdr-retry-off").is_none());
        click(visual, "herdr-retry-signin");
        let machines = visual.update(|_, cx| crate::machines::get(cx).unwrap());
        let refreshed = machines.read_with(visual, |m, _| m.refreshes.clone());
        assert_eq!(refreshed, ["signin"]);
    }

    #[gpui::test]
    fn herdr_machines_add_says_it_is_preparing_until_the_command_is_ready(cx: &mut TestAppContext) {
        let (window, _) = window(Vec::new(), true, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("herdr-add-preparing").is_none());
        // Fetching the machine's build can take minutes on a slow link.
        window
            .update(visual, |view, _, cx| {
                view.herdr.adding = true;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("herdr-add-preparing").is_some());
        window
            .update(visual, |view, _, cx| {
                view.herdr.adding = false;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("herdr-add-preparing").is_none());
    }

    #[gpui::test]
    fn herdr_machines_add_needs_a_target_and_a_herdr(cx: &mut TestAppContext) {
        let (window, commands) = window(Vec::new(), true, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        click(visual, "herdr-add");
        window
            .update(visual, |view, _, _| assert!(!view.herdr.adding))
            .unwrap();
        // Spaces alone are no target.
        window
            .update(visual, |view, _, cx| {
                view.fields[TARGET].update(cx, |i, cx| i.set_text("   ".into(), cx))
            })
            .unwrap();
        visual.run_until_parked();
        click(visual, "herdr-add");
        window
            .update(visual, |view, _, _| assert!(!view.herdr.adding))
            .unwrap();
        assert!(visual.debug_bounds("herdr-not-bundled").is_none());

        // Without a herdr there is nothing to run, and the page says why.
        let machines = visual.update(|_, cx| crate::machines::get(cx).unwrap());
        machines.update(visual, |m, cx| m.set_herdr_found(false, cx));
        window
            .update(visual, |view, _, cx| {
                view.fields[TARGET].update(cx, |i, cx| i.set_text("me@box".into(), cx))
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("herdr-not-bundled").is_some());
        click(visual, "herdr-add");
        window
            .update(visual, |view, _, _| assert!(!view.herdr.adding))
            .unwrap();
        assert!(commands.borrow().is_empty());
    }

    #[gpui::test]
    fn herdr_machines_remove_asks_first_and_cancel_keeps_the_machine(cx: &mut TestAppContext) {
        let (window, _) = window(
            vec![machine("m1", Online::Online, Some(HERDR_VERSION), true)],
            true,
            cx,
        );
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("herdr-remove-confirm").is_none());
        click(visual, "herdr-remove-m1");
        assert!(visual.debug_bounds("herdr-remove-confirm").is_some());
        assert_eq!(
            remove_question("m1 box"),
            "Remove m1 box from Gespan and herdr? Its agents keep running on the machine."
        );
        click(visual, "herdr-remove-cancel");
        assert!(visual.debug_bounds("herdr-remove-confirm").is_none());
        assert!(visual.debug_bounds("herdr-machine-m1").is_some());
    }

    #[gpui::test]
    fn herdr_machines_a_late_change_for_a_removed_machine_is_harmless(cx: &mut TestAppContext) {
        let (window, _) = window(
            vec![
                machine("m1", Online::Online, Some(HERDR_VERSION), true),
                machine("m2", Online::Connecting, None, true),
            ],
            true,
            cx,
        );
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        click(visual, "herdr-remove-m1");
        window
            .update(visual, |view, _, _| {
                view.herdr.errors.insert("m1".into(), "old failure".into());
            })
            .unwrap();
        // The registry reports again after m1 is gone.
        let machines = visual.update(|_, cx| crate::machines::get(cx).unwrap());
        visual.update(|_, cx| {
            crate::machines::set_for_test(
                &machines,
                vec![machine("m2", Online::Online, Some(HERDR_VERSION), true)],
                None,
                cx,
            )
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("herdr-machine-m1").is_none());
        assert!(visual.debug_bounds("herdr-remove-confirm").is_none());
        assert!(visual.debug_bounds("herdr-machine-m2").is_some());
        window
            .update(visual, |view, _, _| {
                assert_eq!(view.herdr.removing, None);
                assert!(view.herdr.errors.is_empty());
            })
            .unwrap();
    }

    #[gpui::test]
    fn herdr_machines_list_error_is_shown(cx: &mut TestAppContext) {
        let (window, _) = window(Vec::new(), true, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let machines = visual.update(|_, cx| crate::machines::get(cx).unwrap());
        visual.update(|_, cx| {
            crate::machines::set_for_test(&machines, Vec::new(), Some("herdr failed".into()), cx)
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("herdr-machines-error").is_some());
        // An unreadable list is not an empty one.
        assert!(visual.debug_bounds("herdr-machines-empty").is_none());
    }

    #[gpui::test]
    fn herdr_machines_page_is_absent_when_hidden(cx: &mut TestAppContext) {
        let (window, _) = window(
            vec![machine("m1", Online::Online, Some(HERDR_VERSION), true)],
            false,
            cx,
        );
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("herdr-machines").is_none());
        assert!(visual.debug_bounds("herdr-machine-m1").is_none());
    }

    #[gpui::test]
    fn herdr_machines_opening_connections_reloads_the_list(cx: &mut TestAppContext) {
        let (window, _) = window(Vec::new(), true, cx);
        let machines = cx.update(|cx| crate::machines::get(cx).unwrap());
        let reloads = |cx: &mut TestAppContext| machines.read_with(cx, |m, _| m.reloads);
        assert_eq!(reloads(cx), 1, "opening on Connections reloads once");
        window
            .update(cx, |view, _, cx| view.select_section(Section::General, cx))
            .unwrap();
        assert_eq!(reloads(cx), 1);
        window
            .update(cx, |view, _, cx| view.select_section(Section::Devices, cx))
            .unwrap();
        assert_eq!(reloads(cx), 2);
    }
}
