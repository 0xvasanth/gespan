//! LiveFoldTitle's "running in background": once the agent has answered with
//! commands or subagents still going, they sit on the composer's ledge above
//! the message queue until they finish or the agent picks the thread back up.
use super::*;
use rebase_core::provider::BackgroundTask;

const ROW_HEIGHT: f32 = 38.;
/// More rows than this scroll inside the card.
const VISIBLE_ROWS: f32 = 4.;
const LIVE: u32 = 0x4ade80;

#[derive(Default)]
pub(super) struct BackgroundTasks {
    pub tasks: Vec<BackgroundTask>,
    pub collapsed: bool,
    pub motion: composer_shelf::Motion,
}

impl Conversation {
    pub(super) fn set_background(&mut self, tasks: Vec<BackgroundTask>, cx: &mut Context<Self>) {
        let animate = !cx.reduce_motion();
        for task in &tasks {
            if !self.background.tasks.iter().any(|old| old.id == task.id) {
                self.background
                    .motion
                    .insert(&task.id, self.background.tasks.is_empty(), animate);
            }
        }
        self.background.tasks = tasks;
        cx.notify();
    }

    pub(super) fn clear_background(&mut self) {
        self.background.tasks.clear();
    }

    pub(super) fn background_shelf(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let width = self.composer_anchor.get().map(|bounds| bounds.size.width);
        let tasks = &self.background.tasks;
        self.background.motion.retain(
            tasks.iter().map(|task| task.id.as_str()),
            !cx.reduce_motion() && width.is_some(),
        );
        if tasks.is_empty() {
            return div().into_any_element();
        }
        // Over a queued message the card tucks in behind it, a deck of two.
        let inset = if self.follow_ups.state.messages.is_empty() {
            px(8.)
        } else {
            px(20.)
        };
        let card_width = (width.unwrap_or_default() - inset * 2.).max(px(0.));
        let row_width = (card_width - px(22.)).max(px(0.));
        let count = tasks.len();
        let collapsible = count > 1;
        let collapsed = collapsible && self.background.collapsed;
        let title = if count == 1 {
            "Running in background".to_owned()
        } else {
            format!("{count} tasks running in background")
        };
        let agent = Kind::from_id(&self.summary.provider).label();
        let header = div()
            .id("background-tasks-header")
            .debug_selector(|| "background-tasks-header".into())
            .role(Role::Button)
            .aria_label(title.clone())
            .when(collapsible, |d| d.aria_expanded(!collapsed))
            .h(px(30.))
            .flex()
            .items_center()
            .gap(px(8.))
            .when(collapsible, |d| {
                d.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                    this.background.collapsed = !this.background.collapsed;
                    cx.notify();
                }))
            })
            .child(
                div()
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::session_mark::spinner(
                        "background-tasks-spinner",
                        crate::pulse::Rate::Half,
                    )),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(content(0.8))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(title),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(content(0.35))
                    .child(if collapsed {
                        tasks
                            .iter()
                            .map(|task| task.description.as_str())
                            .collect::<Vec<_>>()
                            .join(" · ")
                    } else {
                        format!("{agent} continues when they finish")
                    }),
            )
            .when(collapsible, |d| {
                d.child(
                    div()
                        .size(px(20.))
                        .flex_shrink_0()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(content(0.45))
                        .hover(|s| s.bg(content(0.08)).text_color(content(0.9)))
                        .child(icon(
                            if collapsed {
                                "chevron-up"
                            } else {
                                "chevron-down-2"
                            },
                            12.,
                        )),
                )
            });
        let rows = div()
            .id("background-tasks-rows")
            .max_h(px(ROW_HEIGHT * VISIBLE_ROWS))
            .overflow_y_scroll()
            .children(tasks.iter().map(|task| {
                let row = background_row(task, cx);
                self.background.motion.row(&task.id, row, row_width, window)
            }));
        let card = composer_shelf::card("background-tasks-card", self.classic_layout)
            .role(Role::Group)
            .aria_label("Background tasks")
            .child(header)
            .when(!collapsed, |d| d.child(rows));
        composer_shelf::shelf("background-tasks")
            .px(inset)
            .child(self.background.motion.card(card, card_width, window))
            .into_any_element()
    }
}

fn background_row(
    task: &BackgroundTask,
    cx: &mut Context<Conversation>,
) -> gpui::Stateful<gpui::Div> {
    let started_at = task.started_at;
    let tool_id = task.tool_id.clone();
    div()
        .id(SharedString::from(format!("background-{}", task.id)))
        .debug_selector({
            let id = task.id.clone();
            move || format!("background-task-{id}")
        })
        .role(if tool_id.is_some() {
            Role::Button
        } else {
            Role::Group
        })
        .aria_label(format!("Background task: {}", task.description))
        .min_h(px(ROW_HEIGHT))
        .when_some(tool_id, |d, tool_id| {
            d.cursor_pointer()
                .hover(|s| s.bg(content(0.04)))
                .on_click(cx.listener(move |this, _, _, cx| this.reveal_tool(&tool_id, cx)))
        })
        .flex()
        .items_center()
        .gap(px(10.))
        .border_t_1()
        .border_color(content(0.05))
        .child(
            div()
                .size(px(24.))
                .flex_shrink_0()
                .rounded(px(7.))
                .bg(content(0.07))
                .text_color(content(0.6))
                .flex()
                .items_center()
                .justify_center()
                .child(icon(if task.agent { "bot" } else { "terminal" }, 13.)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .text_color(content(0.8))
                        .child(task.description.clone()),
                )
                .when(task.latest.is_some() || task.steps > 0, |d| {
                    let id = task.id.clone();
                    let steps_id = task.id.clone();
                    d.child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(8.))
                            .text_size(px(11.))
                            .text_color(content(0.4))
                            .when_some(
                                task.latest
                                    .clone()
                                    .filter(|latest| !latest.trim().is_empty()),
                                |d, latest| {
                                    d.child(
                                        div()
                                            .debug_selector(move || {
                                                format!("background-latest-{id}")
                                            })
                                            .min_w_0()
                                            .flex_1()
                                            .truncate()
                                            .child(latest),
                                    )
                                },
                            )
                            .when(task.steps > 0, |d| {
                                d.child(
                                    div()
                                        .debug_selector(move || {
                                            format!("background-steps-{steps_id}")
                                        })
                                        .flex_shrink_0()
                                        .child(steps_label(task.steps)),
                                )
                            }),
                    )
                })
                .when_some(task.command.clone(), |d, command| {
                    d.child(
                        div()
                            .truncate()
                            .font_family(crate::markdown::PILOT)
                            .text_size(px(11.))
                            .text_color(content(0.4))
                            .child(format!("$ {}", first_line(&command))),
                    )
                }),
        )
        .child(crate::pulse::looping(
            SharedString::from(format!("background-clock-{}", task.id)),
            std::time::Duration::from_millis(2_000),
            crate::pulse::Rate::Low,
            move |phase| {
                let ms = rebase_core::time::now_ms().saturating_sub(started_at);
                // A slow breath: the dot swells and its halo fades out.
                let breath = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos();
                let live: gpui::Hsla = gpui::rgb(LIVE).into();
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child(elapsed(ms)),
                    )
                    .child(
                        div()
                            .size(px(12.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(live.opacity(0.25 * (1. - breath)))
                            .child(
                                div()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(live.opacity(0.65 + 0.35 * breath)),
                            ),
                    )
            },
        ))
}

fn steps_label(steps: u32) -> String {
    format!("{steps} {}", if steps == 1 { "step" } else { "steps" })
}

fn first_line(command: &str) -> &str {
    command
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(command)
        .trim()
}

/// A ticking clock, so seconds stay padded once minutes appear.
fn elapsed(ms: u64) -> String {
    let seconds = ms / 1000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let (minutes, rest) = (seconds / 60, seconds % 60);
    if minutes < 60 {
        return format!("{minutes}m {rest:02}s");
    }
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};
    use rebase_core::store::Store;
    use std::sync::Mutex;

    fn task(id: &str, agent: bool) -> BackgroundTask {
        BackgroundTask {
            id: id.into(),
            description: format!("Task {id}"),
            agent,
            command: (!agent).then(|| "npm run dev".into()),
            started_at: rebase_core::time::now_ms(),
            tool_id: None,
            latest: None,
            steps: 0,
        }
    }

    #[gpui::test]
    fn background_work_sits_on_the_ledge_steers_new_messages_and_clears_on_stop(
        cx: &mut TestAppContext,
    ) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let summary = store.create("claude", "/tmp").unwrap();
        let window = cx.update(|cx| {
            input::bind(cx);
            cx.set_reduce_motion(true);
            cx.set_global(crate::settings::Preferences {
                follow_up: "queue".into(),
                ..Default::default()
            });
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                    view.loaded = true;
                    view.summary.has_messages = true;
                    view.busy = true;
                    view.set_background(vec![task("dev", false), task("review", true)], cx);
                    view
                })
            })
            .unwrap()
        });
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("background-task-dev").is_some());
        assert!(visual.debug_bounds("background-task-review").is_some());

        let header = visual.debug_bounds("background-tasks-header").unwrap();
        visual.simulate_click(header.center(), Default::default());
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("background-task-dev").is_none());
        assert!(visual.debug_bounds("background-tasks-header").is_some());

        window
            .update(visual, |view, _, cx| {
                view.enqueue_follow_up("Is the server up?".into(), None, cx);
                assert!(
                    view.follow_ups.state.messages[0].steer,
                    "a message must not wait behind a server that never ends"
                );
                view.stop(cx);
                assert!(view.background.tasks.is_empty());
            })
            .unwrap();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("background-tasks-header").is_none());
    }

    fn agent_row(id: &str) -> rebase_core::transcript::Row {
        let mut call = rebase_core::tool::ToolCall::new(id, "Agent");
        call.panel = Some(rebase_core::tool::tasks::Panel::Agent(
            rebase_core::tool::tasks::AgentRun {
                name: Some("Review".into()),
                ..Default::default()
            },
        ));
        call.state = rebase_core::tool::ToolState::Completed;
        call.output = "done".into();
        rebase_core::transcript::Row {
            role: "tool".into(),
            text: serde_json::to_string(&call).unwrap(),
            first: true,
            sent_at: None,
        }
    }

    #[gpui::test]
    fn a_linked_row_reveals_its_block_and_an_unlinked_row_does_nothing(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let summary = store.create("claude", "/tmp").unwrap();
        let linked = BackgroundTask {
            tool_id: Some("send-1".into()),
            latest: Some("Reading src/auth.rs".into()),
            steps: 5,
            ..task("linked", true)
        };
        let plain = task("plain", true);
        let window = cx.update(|cx| {
            input::bind(cx);
            cx.set_reduce_motion(true);
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                    view.loaded = true;
                    view.summary.has_messages = true;
                    view.busy = true;
                    view.transcript.update(cx, |transcript, cx| {
                        let row = |role: &str, text: &str| rebase_core::transcript::Row {
                            role: role.into(),
                            text: text.into(),
                            first: true,
                            sent_at: None,
                        };
                        transcript.patch(
                            0,
                            vec![
                                row("user", "Go"),
                                agent_row("send-1"),
                                row("assistant", "Ok"),
                            ],
                            cx,
                        );
                    });
                    view.set_background(vec![linked, plain], cx);
                    view
                })
            })
            .unwrap()
        });
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        // The row shows the latest activity and its step count.
        assert!(visual.debug_bounds("background-latest-linked").is_some());
        assert!(visual.debug_bounds("background-steps-linked").is_some());
        assert!(visual.debug_bounds("background-steps-plain").is_none());
        assert_eq!(steps_label(5), "5 steps");
        assert_eq!(steps_label(1), "1 step");

        let is_open = |visual: &mut VisualTestContext| {
            window
                .update(visual, |view, _, cx| {
                    view.transcript
                        .read(cx)
                        .agent_open_for_test("agent:1:send-1")
                })
                .unwrap()
        };
        let plain_row = visual.debug_bounds("background-task-plain").unwrap();
        visual.simulate_click(plain_row.center(), Default::default());
        visual.refresh().unwrap();
        assert!(!is_open(visual), "a row without a block id is not a link");

        let linked_row = visual.debug_bounds("background-task-linked").unwrap();
        visual.simulate_click(linked_row.center(), Default::default());
        visual.refresh().unwrap();
        assert!(is_open(visual), "the click opens the block it points at");
    }

    #[test]
    fn elapsed_reads_as_a_running_clock() {
        assert_eq!(elapsed(4_200), "4s");
        assert_eq!(elapsed(72_000), "1m 12s");
        assert_eq!(elapsed(3_660_000), "1h 01m");
        assert_eq!(first_line("\n  npm run dev\nsecond"), "npm run dev");
    }
}
