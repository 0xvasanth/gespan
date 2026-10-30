//! The rail's "Meet Pilots" card (PilotIntro.tsx): it stands in for the Pilots
//! section while the user has none, and its switch hides Pilots altogether.
use super::{HidePilots, Pilots, avatar::avatar};
use crate::{
    conversation::HintTooltip,
    theme::{accent_color, content, shell_fill},
};
use gespan_core::project_appearance::{COLORS, MASCOTS};
use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, Div, FontWeight, Role, Stateful, Toggled,
    div, prelude::*, px, rgb,
};
use std::time::Duration;

/// Gap between the mascots lining up at the back.
const CROWD_STEP: f32 = 17.;
const HIDE_HINT: &str = "Hide pilots from the rail. Turn them back on in Settings › Pilots.";
const STAGE_HEIGHT: f32 = 48.;
const MARCH: Duration = Duration::from_millis(700);
const MARCH_STAGGER: Duration = Duration::from_millis(70);
/// The user's Pilot waits for the crowd to line up.
const DROP_DELAY: Duration = Duration::from_millis(1350);
const DROP: Duration = Duration::from_millis(650);

/// The rest of the mascots, each in its own color, to line up behind the Pilot.
pub(super) fn crowd(mascot: &str, color: u32) -> Vec<(&'static str, u32)> {
    let colors: Vec<u32> = COLORS[1..]
        .iter()
        .copied()
        .filter(|c| *c != color)
        .collect();
    MASCOTS
        .iter()
        .copied()
        .filter(|name| *name != mascot)
        .enumerate()
        .map(|(i, name)| (name, colors[i % colors.len()]))
        .collect()
}

/// Where each mascot stands, measured from the stage's center.
fn crowd_offset(index: usize, count: usize) -> f32 {
    (index as f32 - (count as f32 - 1.) / 2.) * CROWD_STEP
}

/// The crowd fills in from the outside, alternating sides.
fn crowd_order(index: usize, count: usize) -> usize {
    let left = (index as f32) < (count as f32 - 1.) / 2.;
    index.min(count - 1 - index) * 2 + usize::from(!left)
}

/// How far the Pilot is above its spot, part way through the whole sequence.
fn drop_height(delta: f32) -> f32 {
    let total = (DROP_DELAY + DROP).as_secs_f32();
    let t = ((delta * total - DROP_DELAY.as_secs_f32()) / DROP.as_secs_f32()).clamp(0., 1.);
    if t < 0.55 {
        let fall = 1. - t / 0.55;
        240. * (1. - (1. - fall) * (1. - fall))
    } else if t < 0.68 {
        0.
    } else if t < 0.84 {
        10. * (t - 0.68) / 0.16
    } else {
        10. * (1. - t) / 0.16
    }
}

fn stage(mascot: &str, color: u32, reduced: bool) -> Stateful<Div> {
    let crowd = crowd(mascot, color);
    let count = crowd.len();
    let mascots = crowd.into_iter().enumerate().map(|(i, (name, color))| {
        let offset = crowd_offset(i, count);
        let mark = div()
            .absolute()
            .bottom(px(16.))
            .left_1_2()
            .ml(px(offset - 7.))
            .opacity(0.45)
            .child(avatar(name, color, "idle", 14., true));
        if reduced {
            return mark.into_any_element();
        }
        let from = if offset < 0. { -140. } else { 140. };
        let delay = MARCH_STAGGER * crowd_order(i, count) as u32;
        let start = delay.as_secs_f32() / (delay + MARCH).as_secs_f32();
        mark.with_animation(
            ("pilot-intro-march", i),
            Animation::new(delay + MARCH),
            move |d, delta| {
                let t = ((delta - start) / (1. - start)).clamp(0., 1.);
                d.ml(px(offset - 7. + from * (1. - t) * (1. - t) * (1. - t)))
            },
        )
        .into_any_element()
    });
    let pilot = div()
        .absolute()
        .bottom(px(4.))
        .left_1_2()
        .ml(px(-16.))
        .child(avatar(mascot, color, "idle", 32., true));
    let pilot = if reduced {
        pilot.into_any_element()
    } else {
        pilot
            .with_animation(
                "pilot-intro-drop",
                Animation::new(DROP_DELAY + DROP),
                |d, delta| d.bottom(px(4. + drop_height(delta))),
            )
            .into_any_element()
    };
    div()
        .id("pilot-intro-stage")
        .relative()
        .h(px(STAGE_HEIGHT))
        .overflow_hidden()
        .children(mascots)
        .child(pilot)
}

impl Pilots {
    /// `projects` are what the first Pilot starts working on.
    pub(super) fn intro_card(
        &mut self,
        projects: Vec<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (mascot, color) = self.roster.next_look();
        let mut ink = shell_fill();
        ink.a = 1.;
        div()
            .id("pilot-intro")
            .debug_selector(|| "pilot-intro".into())
            .role(Role::Group)
            .aria_label("Meet Pilots")
            .mx_2()
            .mb_2()
            .overflow_hidden()
            .rounded_lg()
            .bg(content(0.03))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(10.))
                    .pr_2()
                    .pt_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(1.))
                            .child("Meet Pilots"),
                    )
                    .child(
                        div()
                            .id("pilot-intro-hide")
                            .debug_selector(|| "pilot-intro-hide".into())
                            .role(Role::Switch)
                            .aria_label("Show pilots")
                            .aria_toggled(Toggled::True)
                            .relative()
                            .h(px(16.))
                            .w(px(28.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(accent_color())
                            .cursor_pointer()
                            .tooltip(|_, cx| {
                                cx.new(|_| HintTooltip(HIDE_HINT)).into()
                            })
                            .child(
                                div()
                                    .absolute()
                                    .top(px(2.))
                                    .left(px(14.))
                                    .size(px(12.))
                                    .rounded_full()
                                    .bg(rgb(0xffffff)),
                            )
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(HidePilots);
                            })),
                    ),
            )
            .child(stage(mascot, color, cx.reduce_motion()))
            .child(
                div()
                    .px(px(10.))
                    .pt(px(6.))
                    .text_size(px(11.))
                    .line_height(px(15.))
                    .text_color(content(0.55))
                    .child(
                        "Agents of your own that work across your projects and run habits on their own.",
                    ),
            )
            .child(
                div().p_2().child(
                    div()
                        .id("pilot-intro-create")
                        .debug_selector(|| "pilot-intro-create".into())
                        .role(Role::Button)
                        .aria_label("Create pilot")
                        .h(px(28.))
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .bg(content(1.))
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ink)
                        .hover(|s| s.opacity(0.85))
                        .child("Create pilot")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.create(projects.clone(), window, cx);
                        })),
                ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn crowd_is_every_other_mascot_and_never_wears_the_pilots_color() {
        let (mascot, color) = (MASCOTS[3], COLORS[4]);
        let crowd = crowd(mascot, color);
        let names: HashSet<_> = crowd.iter().map(|(name, _)| *name).collect();
        assert_eq!(names.len(), MASCOTS.len() - 1);
        assert!(!names.contains(mascot));
        assert!(crowd.iter().all(|(_, c)| *c != color));
    }

    #[test]
    fn crowd_fills_in_from_both_edges_with_no_two_arriving_together() {
        let count = MASCOTS.len() - 1;
        let order: Vec<_> = (0..count).map(|i| crowd_order(i, count)).collect();
        assert_eq!(order.iter().collect::<HashSet<_>>().len(), count);
        assert_eq!(order[0], 0);
        assert_eq!(order[count - 1], 1);
        assert_eq!(crowd_offset(0, count), -crowd_offset(count - 1, count));
    }

    #[test]
    fn pilot_waits_off_stage_then_lands_on_its_spot() {
        let delay = DROP_DELAY.as_secs_f32() / (DROP_DELAY + DROP).as_secs_f32();
        assert_eq!(drop_height(0.), 240.);
        assert_eq!(drop_height(delay), 240.);
        assert!(drop_height(delay + (1. - delay) * 0.3) < 240.);
        assert_eq!(drop_height(1.), 0.);
    }
}
