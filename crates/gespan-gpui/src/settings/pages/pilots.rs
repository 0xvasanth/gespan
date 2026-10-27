use crate::settings::SettingsView;
use crate::settings::{management::Target, widgets::icon_action};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
impl SettingsView {
    pub(super) fn pilots_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.prefs.clone();
        let mut rows=vec![
            self.row(Some("pilots-enabled"),true,"Show pilots",Some("Agents of your own on the project rail. Each works on the projects you give it, remembers what matters and picks up habits it runs on its own. Turn this off to hide them.".into()),vec![self.toggle("pilots-enabled","Show pilots",p.pilots,false,cx,|this,on,cx|this.update(cx,|p|p.pilots=on))]),
            self.row(Some("rail-pilots-pinned"),false,"Pin pilots to the icon rail",Some("When the project rail is collapsed to icons, show each Pilot at the top of the rail, above a divider, instead of inside the project picker.".into()),vec![self.toggle("rail-pilots-pinned","Pin pilots to the icon rail",p.rail_pilots_pinned,!p.pilots,cx,|this,on,cx|this.update(cx,|p|p.rail_pilots_pinned=on))])];
        if cfg!(target_os = "macos") {
            rows.push(self.row(Some("pilot-menu-bar-icon"),false,"Menu bar icon",Some("Chat with a Pilot or open the quick composer from the macOS menu bar. Turn this off to hide the icon.".into()),vec![self.toggle("pilot-menu-bar-icon","Menu bar icon",p.pilot_menu_bar_icon,false,cx,|this,on,cx|this.update(cx,|p|p.pilot_menu_bar_icon=on))]));
        }
        let mut roster = Vec::new();
        for (ix, pilot) in self.ctx.pilots.clone().into_iter().enumerate() {
            let description = match pilot.projects.split_last() {
                None => "No projects yet".to_owned(),
                Some((last, [])) => format!("Works on {last}"),
                Some((last, others)) => format!("Works on {} and {last}", others.join(", ")),
            };
            let label = div()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .debug_selector(|| "settings-pilot-mascot".into())
                        .size(px(16.))
                        .flex_shrink_0()
                        .child(crate::pilots::avatar::avatar(
                            &pilot.mascot,
                            pilot.color,
                            "idle",
                            16.,
                            true,
                        )),
                )
                .child(
                    div()
                        .debug_selector(|| "settings-pilot-name".into())
                        .min_w_0()
                        .truncate()
                        .child(pilot.name.clone()),
                );
            let reset = pilot.clone();
            let controls = vec![
                icon_action(
                    "reset-pilot",
                    format!("Reset {} to defaults", pilot.name),
                    "rotate-ccw",
                    28.,
                    14.,
                    0.4,
                    false,
                )
                .debug_selector(|| "settings-pilot-reset".into())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_management(Target::ResetPilot(reset.clone()), window, cx)
                }))
                .into_any_element(),
            ];
            roster.push(
                div()
                    .id(SharedString::from(format!("settings-pilot-{}", pilot.id)))
                    .child(self.row(None, ix == 0, label, Some(description.into()), controls))
                    .into_any_element(),
            );
        }
        if roster.is_empty() {
            roster.push(super::super::widgets::note("No pilots yet."));
        }
        vec![self.group(None,true,"Pilots",None,None,rows),self.group(Some("pilot-list"),false,"Your pilots",Some("Show or hide sessions started by each Pilot. Hidden sessions remain available from the Pilot's chat.".into()),None,roster)]
    }
}
