//! Automation: the agent switches and one auto-move rule per event.
use super::{button, heading};
use crate::board::BoardView;
use gespan_core::board::{
    auto_move::LinkedWorkEvent,
    extras::{RuleTarget, SettingsPatch, auto_move_rules},
};
use gpui::{AnyElement, Context, Role, div, prelude::*, px};
use serde_json::{Map, Value, json};

const UNREADABLE: &str = "Couldn't load settings";

/// The events a rule can be set for: the stored key and the label shown.
const EVENTS: [(&str, &str); 6] = [
    ("started", "Started"),
    ("needs_input", "Needs input"),
    ("completed", "Completed"),
    ("failed", "Failed"),
    ("pr_opened", "PR opened"),
    ("pr_merged", "PR merged"),
];

impl BoardView {
    /// Switch a flag. Single-field patch, and not while the settings could
    /// not be read (the switch shows neither on nor off then).
    fn toggle_agent_setting(&mut self, patch: SettingsPatch, cx: &mut Context<Self>) {
        if self.settings_loaded {
            self.patch_settings(patch, cx);
        }
    }

    /// Store one event's rule, building on the rules as stored right now (so
    /// a rule the other app just set, or a key this app doesn't know, stays).
    /// A failed save reloads, so the picker shows what is stored.
    fn set_auto_move_rule(&mut self, event: &'static str, rule: Value, cx: &mut Context<Self>) {
        let Some(board_id) = self.open_board_id.clone() else {
            return;
        };
        if !self.settings_loaded {
            return;
        }
        self.settings_write(cx, |service| {
            let stored = service.settings(&board_id)?;
            let mut map = match stored
                .auto_move_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            {
                Some(Value::Object(map)) => map,
                _ => Map::new(),
            };
            map.insert(event.to_string(), rule);
            service
                .update_settings(
                    &board_id,
                    &SettingsPatch {
                        auto_move_json: Some(Some(Value::Object(map).to_string())),
                        ..Default::default()
                    },
                )
                .map(drop)
        });
    }

    /// The rule the picker shows for an event. A column deleted since reads
    /// as Stay (and is not rewritten until a rule is picked).
    pub(crate) fn displayed_rule(&self, key: &str) -> RuleTarget {
        let rules = auto_move_rules(&self.board_settings);
        match rules.rule(event_of(key)) {
            RuleTarget::Column(id)
                if !self
                    .detail
                    .as_ref()
                    .is_some_and(|detail| detail.columns.iter().any(|column| &column.id == id)) =>
            {
                RuleTarget::Stay
            }
            other => other.clone(),
        }
    }

    pub(super) fn render_automation(&self, cx: &mut Context<Self>) -> AnyElement {
        let loaded = self.settings_loaded;
        let settings = self.board_settings.clone();
        let toggle = |id: &str, label: &str, on: bool, make: fn(bool) -> SettingsPatch| {
            // Unknown until the settings are read: neither on nor off.
            let shown_on = loaded && on;
            let element = button(id.to_string(), label, shown_on).role(Role::Switch);
            if loaded {
                element
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_agent_setting(make(!on), cx);
                    }))
                    .into_any_element()
            } else {
                element.opacity(0.45).cursor_default().into_any_element()
            }
        };
        let toggles = vec![
            toggle(
                "settings-toggle-run-with",
                "Run with…",
                settings.run_with,
                |v| SettingsPatch {
                    run_with: Some(v),
                    ..Default::default()
                },
            ),
            toggle(
                "settings-toggle-write-back",
                "Agent write-back",
                settings.write_back,
                |v| SettingsPatch {
                    write_back: Some(v),
                    ..Default::default()
                },
            ),
            toggle(
                "settings-toggle-ticket-status",
                "Agent may change ticket status",
                settings.ticket_status_changes,
                |v| SettingsPatch {
                    ticket_status_changes: Some(v),
                    ..Default::default()
                },
            ),
        ];
        let columns = self
            .detail
            .as_ref()
            .map(|detail| detail.columns.clone())
            .unwrap_or_default();
        let mut rows: Vec<AnyElement> = Vec::new();
        for (key, label) in EVENTS {
            let current = self.displayed_rule(key);
            let mut options: Vec<AnyElement> = Vec::new();
            let option =
                |id: String, text: &str, on: bool, rule: Value, options: &mut Vec<AnyElement>| {
                    let element = button(id, text, loaded && on);
                    options.push(if loaded {
                        element
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_auto_move_rule(key, rule.clone(), cx);
                            }))
                            .into_any_element()
                    } else {
                        element.opacity(0.45).cursor_default().into_any_element()
                    });
                };
            option(
                format!("settings-rule-{key}-default"),
                "Default",
                current == RuleTarget::Default,
                json!("default"),
                &mut options,
            );
            option(
                format!("settings-rule-{key}-stay"),
                "Stay",
                current == RuleTarget::Stay,
                json!("stay"),
                &mut options,
            );
            for (index, column) in columns.iter().enumerate() {
                option(
                    format!("settings-rule-{key}-column-{index}"),
                    &column.name,
                    current == RuleTarget::Column(column.id.clone()),
                    json!({ "column": column.id }),
                    &mut options,
                );
            }
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(crate::theme::content(0.8))
                            .child(label),
                    )
                    .child(div().flex().flex_wrap().gap(px(6.)).children(options))
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .when(!loaded, |d| {
                d.child(
                    div()
                        .id("settings-unreadable")
                        .debug_selector(|| "settings-unreadable".into())
                        .text_size(px(12.))
                        .text_color(crate::theme::diff_foreground(false))
                        .child(UNREADABLE),
                )
            })
            .child(heading("AGENTS"))
            .child(div().flex().flex_wrap().gap(px(6.)).children(toggles))
            .child(heading("WHEN A LINKED SESSION OR PR REPORTS"))
            .child(div().flex().flex_col().gap_3().children(rows))
            .into_any_element()
    }
}

fn event_of(key: &str) -> LinkedWorkEvent {
    use LinkedWorkEvent::*;
    match key {
        "started" => Started,
        "needs_input" => NeedsInput,
        "completed" => Completed,
        "failed" => Failed,
        "pr_opened" => PrOpened,
        _ => PrMerged,
    }
}
