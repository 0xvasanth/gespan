//! Display: what a card face shows, density and swimlanes, for the board or
//! for the open saved view. The board default is a patch of `display_json`
//! built on the latest stored value; a view's own options go under
//! `layout_json.display`, mark the view changed, and save with it.
use super::{button, heading};
use crate::board::BoardView;
use gespan_core::board::extras::{Density, DisplayOptions, SettingsPatch, display_options};
use gpui::{AnyElement, Context, Role, div, prelude::*, px};

/// The card-face parts, as (key, label).
const FACE: [(&str, &str); 7] = [
    ("labels", "Labels"),
    ("priority", "Priority"),
    ("due", "Due date"),
    ("subtasks", "Subtasks"),
    ("comments", "Comments"),
    ("linkStatus", "Link status"),
    ("number", "Number"),
];
const AGENT_FIELDS: [(&str, &str); 4] = [
    ("provider", "Provider"),
    ("model", "Model"),
    ("cost", "Cost"),
    ("runTime", "Run time"),
];

fn face_flag<'a>(display: &'a mut DisplayOptions, key: &str) -> Option<&'a mut bool> {
    let face = &mut display.face;
    Some(match key {
        "labels" => &mut face.labels,
        "priority" => &mut face.priority,
        "due" => &mut face.due,
        "subtasks" => &mut face.subtasks,
        "comments" => &mut face.comments,
        "linkStatus" => &mut face.link_status,
        "number" => &mut face.number,
        _ => return None,
    })
}

fn flip(list: &mut Vec<String>, value: &str) {
    match list.iter().position(|item| item == value) {
        Some(at) => {
            list.remove(at);
        }
        None => list.push(value.to_string()),
    }
}

impl BoardView {
    /// The display options in force: the open saved view's own, else the
    /// board's. "All cards" always uses the board default.
    pub(crate) fn effective_display(&self) -> DisplayOptions {
        let board = display_options(&self.board_settings);
        if self.views.active.is_none() {
            return board;
        }
        self.layout_spec()
            .display
            .and_then(|value| serde_json::from_value::<DisplayOptions>(value).ok())
            .unwrap_or(board)
    }

    fn editing_view_display(&self) -> bool {
        self.views.active.is_some() && self.settings.as_ref().is_some_and(|page| page.display_view)
    }

    fn set_display_target(&mut self, view: bool, cx: &mut Context<Self>) {
        if let Some(page) = &mut self.settings {
            page.display_view = view;
        }
        cx.notify();
    }

    /// Change the display options being edited. The board default is built on
    /// what is stored now, so two quick toggles (or the other app's edit in
    /// between) never lose a write.
    fn edit_display(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut DisplayOptions) + 'static,
    ) {
        // The defaults on screen are not the stored ones while settings are
        // unreadable: neither a view override nor the board default is built
        // from them.
        if !self.settings_loaded {
            return;
        }
        if self.editing_view_display() {
            let mut display = self.effective_display();
            change(&mut display);
            let value = serde_json::to_value(&display).ok();
            self.edit_layout(cx, |spec| spec.display = value);
            return;
        }
        let Some(board_id) = self.open_board_id.clone() else {
            return;
        };
        self.settings_write(cx, move |service| {
            let mut display = display_options(&service.settings(&board_id)?);
            change(&mut display);
            let json = serde_json::to_string(&display).map_err(|error| error.to_string())?;
            service
                .update_settings(
                    &board_id,
                    &SettingsPatch {
                        display_json: Some(Some(json)),
                        ..Default::default()
                    },
                )
                .map(drop)
        });
    }

    pub(super) fn render_display(&self, cx: &mut Context<Self>) -> AnyElement {
        let view_target = self.editing_view_display();
        let shown = if view_target {
            self.effective_display()
        } else {
            display_options(&self.board_settings)
        };
        let mut targets = vec![
            button(
                "settings-display-target-board".into(),
                "Board default",
                !view_target,
            )
            .on_click(cx.listener(|this, _, _, cx| this.set_display_target(false, cx)))
            .into_any_element(),
        ];
        if let Some(active) = &self.views.active
            && let Some(view) = self.views.saved.iter().find(|view| &view.id == active)
        {
            targets.push(
                button(
                    "settings-display-target-view".into(),
                    &format!("This view ({})", view.name),
                    view_target,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_display_target(true, cx)))
                .into_any_element(),
            );
        }
        let toggle = |id: String, label: &str, on: bool| button(id, label, on).role(Role::Switch);
        let face: Vec<AnyElement> = FACE
            .iter()
            .map(|&(key, label)| {
                let mut probe = shown.clone();
                let on = face_flag(&mut probe, key).is_some_and(|flag| *flag);
                toggle(format!("settings-display-face-{key}"), label, on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_display(cx, move |display| {
                            if let Some(flag) = face_flag(display, key) {
                                *flag = !*flag;
                            }
                        })
                    }))
                    .into_any_element()
            })
            .collect();
        let fields: Vec<AnyElement> = self
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let id = field.id.clone();
                let on = shown.face.fields.contains(&field.id);
                toggle(format!("settings-display-field-{index}"), &field.name, on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let id = id.clone();
                        this.edit_display(cx, move |display| flip(&mut display.face.fields, &id))
                    }))
                    .into_any_element()
            })
            .collect();
        let agent: Vec<AnyElement> = AGENT_FIELDS
            .iter()
            .map(|&(key, label)| {
                let on = shown.face.agent_fields.iter().any(|field| field == key);
                toggle(format!("settings-display-agent-{key}"), label, on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_display(cx, move |display| {
                            flip(&mut display.face.agent_fields, key)
                        })
                    }))
                    .into_any_element()
            })
            .collect();
        let density: Vec<AnyElement> = [
            (Density::Comfortable, "comfortable", "Comfortable"),
            (Density::Compact, "compact", "Compact"),
        ]
        .into_iter()
        .map(|(value, key, label)| {
            button(
                format!("settings-display-density-{key}"),
                label,
                shown.density == value,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.edit_display(cx, move |display| display.density = value)
            }))
            .into_any_element()
        })
        .collect();
        // Swimlanes: none, the built-in groupings, then each select field.
        let mut lanes: Vec<(String, String, String)> = ["none", "priority", "label", "project"]
            .iter()
            .map(|key| {
                let label = match *key {
                    "none" => "No lanes",
                    "priority" => "Priority",
                    "label" => "Label",
                    _ => "Project",
                };
                (key.to_string(), key.to_string(), label.to_string())
            })
            .collect();
        for (index, field) in self.fields.iter().enumerate() {
            if field.kind == "select" {
                lanes.push((
                    format!("field-{index}"),
                    format!("field:{}", field.id),
                    field.name.clone(),
                ));
            }
        }
        let lanes: Vec<AnyElement> = lanes
            .into_iter()
            .map(|(id, value, label)| {
                let on = shown.swimlane == value;
                button(format!("settings-display-swimlane-{id}"), &label, on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let value = value.clone();
                        this.edit_display(cx, move |display| display.swimlane = value)
                    }))
                    .into_any_element()
            })
            .collect();
        let wrap =
            |children: Vec<AnyElement>| div().flex().flex_wrap().gap(px(6.)).children(children);
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("APPLIES TO"))
            .child(wrap(targets))
            .child(heading("CARD FACE"))
            .child(wrap(face))
            .child(heading("FIELDS ON THE CARD"))
            .child(wrap(fields))
            .child(heading("AGENT FIELDS"))
            .child(wrap(agent))
            .child(heading("DENSITY"))
            .child(wrap(density))
            .child(heading("SWIMLANES"))
            .child(wrap(lanes))
            .into_any_element()
    }
}
