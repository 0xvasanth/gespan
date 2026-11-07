//! The herdr machines list under Settings → Connections. herdr keeps the
//! list; adding and upgrading run herdr in a terminal tab, where it asks its
//! own questions.
use crate::{
    input::{self, Input},
    settings::{
        Section, SettingsEvent, SettingsView,
        widgets::{red, secondary_button},
    },
    theme::*,
};
use gespan_core::machines::{self, HERDR_VERSION, Machine, MachineView, Online};
use gpui::{
    AnyElement, Context, Div, Entity, FontWeight, Role, SharedString, div, prelude::*, px, rgb,
};
use std::collections::{HashMap, HashSet};

pub(super) const TARGET: &str = "herdr ssh target";
const EMPTY: &str = "No herdr machines yet. Add one to run agents on another computer.";
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
        if self.section == Section::Connections && crate::machines::visible(cx) {
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
        div()
            .border_b_1()
            .border_color(content(0.1))
            .child(
                div()
                    .id(SharedString::from(format!("herdr-machine-{id}")))
                    .debug_selector(move || format!("herdr-machine-{row}"))
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_4()
                    .child(icon("globe", 20.).text_color(content(if online { 0.7 } else { 0.35 })))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(label.clone()),
                            )
                            .child(paragraph(view.machine.target.clone()).mt_1())
                            .child(
                                paragraph(status)
                                    .mt_1()
                                    .text_color(content(0.5))
                                    .id(SharedString::from(format!("herdr-status-{id}")))
                                    .role(Role::Status),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(upgrade, |d| {
                                let selector = upgrade_id.clone();
                                d.child(
                                    secondary_button(
                                        SharedString::from(format!("herdr-upgrade-{id}")),
                                        format!("Upgrade herdr on {label}"),
                                        false,
                                        busy,
                                    )
                                    .debug_selector(move || format!("herdr-upgrade-{selector}"))
                                    .child("Upgrade")
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
                                secondary_button(
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
                        secondary_button(
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
                        secondary_button("herdr-remove-cancel", "Cancel", false, disabled)
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
                .mt_8()
                .child(self.reveal_probe("herdr-machines"))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("herdr machines"),
                )
                .into_any_element(),
        ];
        if !views.is_empty() {
            let mut list = div()
                .mt_4()
                .rounded_xl()
                .border_1()
                .border_color(content(0.1))
                .overflow_hidden();
            for view in &views {
                list = list.child(self.herdr_machine_row(view, cx));
            }
            page.push(list.into_any_element());
        } else if loaded && list_error.is_none() {
            page.push(
                div()
                    .id("herdr-machines-empty")
                    .debug_selector(|| "herdr-machines-empty".into())
                    .mt_4()
                    .rounded_xl()
                    .border_1()
                    .border_color(content(0.15))
                    .px_5()
                    .py_8()
                    .text_center()
                    .text_size(px(13.))
                    .text_color(content(0.45))
                    .child(EMPTY)
                    .into_any_element(),
            );
        }
        if let Some(error) = list_error {
            page.push(
                div()
                    .id("herdr-machines-error")
                    .debug_selector(|| "herdr-machines-error".into())
                    .role(Role::Alert)
                    .mt_4()
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
        let input = self.herdr_target_field(cx);
        let not_bundled = found == Some(false);
        let add_disabled = self.herdr.adding || not_bundled || self.herdr_target(cx).is_empty();
        page.push(
            div()
                .mt_4()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .h(px(36.))
                        .min_w_0()
                        .flex_1()
                        .rounded_lg()
                        .border_1()
                        .border_color(content(0.15))
                        .bg(content(0.03))
                        .px_3()
                        .flex()
                        .items_center()
                        .child(input),
                )
                .child(
                    secondary_button("herdr-add", "Add", false, add_disabled)
                        .debug_selector(|| "herdr-add".into())
                        .child(icon("plus", 16.))
                        .child("Add")
                        .on_click(cx.listener(|this, _, _, cx| this.add_herdr_machine(cx))),
                )
                .into_any_element(),
        );
        if not_bundled {
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
                            Section::Connections,
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

    #[gpui::test]
    fn herdr_machines_empty_state_invites_adding_one(cx: &mut TestAppContext) {
        let (window, _) = window(Vec::new(), true, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("herdr-machines").is_some());
        assert!(visual.debug_bounds("herdr-machines-empty").is_some());
        assert_eq!(
            EMPTY,
            "No herdr machines yet. Add one to run agents on another computer."
        );
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
            .update(cx, |view, _, cx| {
                view.select_section(Section::Connections, cx)
            })
            .unwrap();
        assert_eq!(reloads(cx), 2);
    }
}
