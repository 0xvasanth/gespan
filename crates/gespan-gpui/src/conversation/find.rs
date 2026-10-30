//! TranscriptFind: a find bar over the open conversation with a match count,
//! next and previous. It matches whole messages, as ⌘K search does, and
//! highlights the query in the message it scrolls to.
use super::*;
use crate::right_panel::icon_action;
use std::time::Duration;

gpui::actions!(
    conversation,
    [
        /// Opens the find bar over the conversation.
        FindInTranscript,
        FindNextMatch,
        FindPreviousMatch
    ]
);

pub(crate) fn bind(cx: &mut gpui::App) {
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let context = Some("Session");
    cx.bind_keys([
        gpui::KeyBinding::new(&format!("{m}-f"), FindInTranscript, context),
        gpui::KeyBinding::new(&format!("{m}-g"), FindNextMatch, context),
        gpui::KeyBinding::new(&format!("{m}-shift-g"), FindPreviousMatch, context),
        gpui::KeyBinding::new("f3", FindNextMatch, context),
        gpui::KeyBinding::new("shift-f3", FindPreviousMatch, context),
    ]);
}

#[derive(Default)]
pub(super) struct State {
    field: Option<Entity<input::Input>>,
    pub(super) open: bool,
    /// Matching messages by their first source row, in transcript order.
    matches: Vec<usize>,
    active: usize,
    /// A pending re-read of the matches after the transcript changed.
    refresh: Option<gpui::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Conversation {
    fn find_field(&mut self, cx: &mut Context<Self>) -> Entity<input::Input> {
        if let Some(field) = &self.find.field {
            return field.clone();
        }
        let field = cx.new(|cx| {
            let mut field = input::Input::field(
                "Find in conversation",
                "Find in conversation",
                content(0.4),
                cx,
            );
            field.set_font_size(12.);
            field
        });
        self.find._subscriptions = vec![
            cx.subscribe(&field, |this, _, _: &input::Changed, cx| {
                this.find.active = 0;
                this.refresh_find(cx);
                this.show_find_match(cx);
            }),
            cx.subscribe(&field, |this, _, _: &input::Submitted, cx| {
                this.step_find(1, cx)
            }),
            // Streamed or newly loaded rows can add or remove matches.
            cx.observe(&self.transcript, |this, _, cx| this.transcript_changed(cx)),
        ];
        self.find.field = Some(field.clone());
        field
    }
    pub(crate) fn find_query(&self, cx: &gpui::App) -> String {
        self.find
            .field
            .as_ref()
            .map(|field| field.read(cx).content.trim().to_owned())
            .unwrap_or_default()
    }
    #[cfg(test)]
    pub(crate) fn find_open(&self) -> bool {
        self.find.open
    }
    /// The matching messages by source row, and the one shown.
    #[cfg(test)]
    pub(crate) fn find_state(&self) -> (&[usize], usize) {
        (&self.find.matches, self.find.active)
    }
    pub(crate) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.find_field(cx);
        self.find.open = true;
        // A Pilot pages its history; a find covers all of it.
        self.transcript
            .update(cx, |view, cx| view.load_all_history(cx));
        window.focus(&field.focus_handle(cx), cx);
        field.update(cx, |field, cx| {
            let end = field.content.len();
            field.select_range(0..end, cx)
        });
        self.refresh_find(cx);
        self.show_find_match(cx);
        cx.notify();
    }
    pub(crate) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.open = false;
        self.find.refresh = None;
        self.transcript.update(cx, |view, cx| view.clear_search(cx));
        window.focus(&self.input.focus_handle(cx), cx);
        cx.notify();
    }
    /// Moves to the next (1) or previous (-1) match, wrapping around.
    pub(crate) fn step_find(&mut self, direction: isize, cx: &mut Context<Self>) {
        if self.find.matches.is_empty() {
            return;
        }
        self.find.active = (self.find.active as isize + direction)
            .rem_euclid(self.find.matches.len() as isize) as usize;
        self.show_find_match(cx);
        cx.notify();
    }
    /// Reads the matches again, keeping the shown message when it still matches.
    fn refresh_find(&mut self, cx: &mut Context<Self>) {
        let query = self.find_query(cx);
        let shown = self.find.matches.get(self.find.active).copied();
        self.find.matches = self.transcript.read(cx).find_matches(&query);
        self.find.active = shown
            .and_then(|row| self.find.matches.iter().position(|m| *m == row))
            .unwrap_or_else(|| {
                self.find
                    .active
                    .min(self.find.matches.len().saturating_sub(1))
            });
        cx.notify();
    }
    fn show_find_match(&mut self, cx: &mut Context<Self>) {
        let query = self.find_query(cx);
        let row = self.find.matches.get(self.find.active).copied();
        self.transcript.update(cx, |view, cx| match row {
            Some(row) => view.reveal_search(row, &query, cx),
            None => view.clear_search(cx),
        });
    }
    fn transcript_changed(&mut self, cx: &mut Context<Self>) {
        if !self.find.open || self.find.refresh.is_some() || self.find_query(cx).is_empty() {
            return;
        }
        // A streaming answer changes the transcript every frame.
        self.find.refresh = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.find.refresh = None;
                if !this.find.open {
                    return;
                }
                let shown = this.find.matches.get(this.find.active).copied();
                this.refresh_find(cx);
                if this.find.matches.get(this.find.active).copied() != shown {
                    this.show_find_match(cx);
                }
            });
        }));
    }
    pub(super) fn find_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.find.open {
            return None;
        }
        let field = self.find.field.clone()?;
        let query = self.find_query(cx);
        let (count, total) = (self.find.matches.len(), self.find.active);
        let label = if query.is_empty() {
            String::new()
        } else if count == 0 {
            "No results".into()
        } else {
            format!("{} of {count}", total + 1)
        };
        let any = count > 0;
        Some(
            div()
                .absolute()
                .top_2()
                .right_3()
                .child(
                    div()
                        .id("transcript-find")
                        .role(Role::Search)
                        .aria_label("Find in conversation")
                        .occlude()
                        .w(px(360.))
                        .flex()
                        .items_center()
                        .gap_1()
                        .p_1()
                        .rounded_lg()
                        .border_1()
                        .border_color(content(0.1))
                        .bg(shell_fill())
                        .shadow_lg()
                        // The field is one line, so Shift+Enter goes back.
                        .capture_action(cx.listener(|this, _: &input::Newline, _, cx| {
                            this.step_find(-1, cx);
                            cx.stop_propagation();
                        }))
                        .child(
                            div()
                                .ml_1()
                                .flex_shrink_0()
                                .child(icon("search", 14.).text_color(content(0.5))),
                        )
                        .child(div().flex_1().min_w_0().child(field))
                        .child(
                            div()
                                .flex_shrink_0()
                                .min_w(px(70.))
                                .text_right()
                                .text_size(px(11.))
                                .text_color(content(0.5))
                                .font_family(crate::fonts::monospace())
                                .child(label),
                        )
                        .child(
                            icon_action("find-previous", "Previous match", "chevron-up", any)
                                .on_click(cx.listener(|this, _, _, cx| this.step_find(-1, cx))),
                        )
                        .child(
                            icon_action("find-next", "Next match", "chevron-down", any)
                                .on_click(cx.listener(|this, _, _, cx| this.step_find(1, cx))),
                        )
                        .child(icon_action("find-close", "Close find", "x", true).on_click(
                            cx.listener(|this, _, window, cx| this.close_find(window, cx)),
                        )),
                )
                .into_any_element(),
        )
    }
    /// The find bar's keys, on the session's root.
    pub(super) fn find_actions(
        &self,
        root: gpui::Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        root.on_action(
            cx.listener(|this, _: &FindInTranscript, window, cx| this.open_find(window, cx)),
        )
        .on_action(cx.listener(|this, _: &FindNextMatch, _, cx| {
            if this.find.open {
                this.step_find(1, cx)
            } else {
                cx.propagate()
            }
        }))
        .on_action(cx.listener(|this, _: &FindPreviousMatch, _, cx| {
            if this.find.open {
                this.step_find(-1, cx)
            } else {
                cx.propagate()
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::{store::Store, transcript::Row};
    use gpui::{TestAppContext, VisualTestContext, WindowHandle};
    use std::sync::Mutex;

    fn row(role: &str, text: &str) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first: true,
            sent_at: None,
        }
    }

    /// A conversation with four messages, two of them mentioning the footer,
    /// and the composer focused.
    fn conversation(cx: &mut TestAppContext) -> (WindowHandle<Conversation>, VisualTestContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let summary = store.create("claude", "/work/gespan").unwrap();
        let store = Arc::new(Mutex::new(store));
        let window = cx.update(|cx| {
            input::bind(cx);
            crate::popover::bind(cx);
            bind(cx);
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut view = Conversation::new(summary, store, cx);
                    view.loaded = true;
                    view.summary.has_messages = true;
                    view.transcript.update(cx, |t, cx| {
                        t.patch(
                            0,
                            vec![
                                row("user", "Review the Footer"),
                                row("assistant", "The header is fine."),
                                row("user", "And the sidebar?"),
                                row("assistant", "The footer needs a border."),
                            ],
                            cx,
                        )
                    });
                    view
                })
            })
            .unwrap()
        });
        window
            .update(cx, |c, w, cx| w.focus(&c.input.focus_handle(cx), cx))
            .unwrap();
        let visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        (window, visual)
    }

    /// The matches, the shown match, and the transcript's highlight.
    fn state(
        window: WindowHandle<Conversation>,
        cx: &mut VisualTestContext,
    ) -> (Vec<usize>, usize, Option<(usize, String)>) {
        window
            .update(cx, |c, _, cx| {
                let (matches, active) = c.find_state();
                (
                    matches.to_vec(),
                    active,
                    c.transcript.read(cx).search_target(),
                )
            })
            .unwrap()
    }

    #[gpui::test]
    fn find_steps_through_matching_messages_and_wraps(cx: &mut TestAppContext) {
        let (window, mut cx) = conversation(cx);
        let cmd = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.simulate_keystrokes(&format!("{cmd}-f"));
        cx.run_until_parked();
        window
            .update(&mut cx, |c, w, cx| {
                assert!(c.find_open());
                let field = c.find.field.clone().unwrap();
                assert!(
                    field.focus_handle(cx).is_focused(w),
                    "typing goes to the bar"
                );
            })
            .unwrap();
        // Case-insensitive, by message: the first and last messages match.
        cx.simulate_input("footer");
        cx.run_until_parked();
        assert_eq!(
            state(window, &mut cx),
            (vec![0, 3], 0, Some((0, "footer".into())))
        );
        cx.simulate_keystrokes("enter");
        assert_eq!(
            state(window, &mut cx),
            (vec![0, 3], 1, Some((3, "footer".into())))
        );
        // Past the last match, back to the first.
        cx.simulate_keystrokes("enter");
        assert_eq!(state(window, &mut cx).1, 0);
        cx.simulate_keystrokes("shift-enter");
        assert_eq!(state(window, &mut cx).1, 1);
        cx.simulate_keystrokes(&format!("{cmd}-g"));
        assert_eq!(state(window, &mut cx).1, 0);
        cx.simulate_keystrokes(&format!("{cmd}-shift-g f3"));
        assert_eq!(state(window, &mut cx).1, 0);
        // ⌘F again keeps the query and selects it for retyping.
        cx.simulate_keystrokes(&format!("{cmd}-f"));
        cx.simulate_input("sidebar");
        cx.run_until_parked();
        assert_eq!(
            state(window, &mut cx),
            (vec![2], 0, Some((2, "sidebar".into())))
        );
    }

    #[gpui::test]
    fn no_match_clears_the_highlight_and_escape_closes(cx: &mut TestAppContext) {
        let (window, mut cx) = conversation(cx);
        let cmd = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.simulate_keystrokes(&format!("{cmd}-f"));
        cx.simulate_input("header");
        cx.run_until_parked();
        assert_eq!(
            state(window, &mut cx),
            (vec![1], 0, Some((1, "header".into())))
        );
        cx.simulate_input("s");
        cx.run_until_parked();
        assert_eq!(state(window, &mut cx), (vec![], 0, None));
        // Stepping with nothing found does nothing.
        cx.simulate_keystrokes("enter");
        assert_eq!(state(window, &mut cx), (vec![], 0, None));
        cx.simulate_keystrokes("backspace");
        cx.run_until_parked();
        assert_eq!(state(window, &mut cx).2, Some((1, "header".into())));
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        window
            .update(&mut cx, |c, w, cx| {
                assert!(!c.find_open());
                assert!(
                    c.input.focus_handle(cx).is_focused(w),
                    "back to the composer"
                );
                assert_eq!(c.transcript.read(cx).search_target(), None);
            })
            .unwrap();
        // Closed, ⌘G is not the bar's any more.
        cx.simulate_keystrokes(&format!("{cmd}-g"));
        assert_eq!(state(window, &mut cx).2, None);
    }

    #[gpui::test]
    fn streamed_messages_join_the_matches_without_moving_the_shown_one(cx: &mut TestAppContext) {
        let (window, mut cx) = conversation(cx);
        let cmd = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.simulate_keystrokes(&format!("{cmd}-f"));
        cx.simulate_input("footer");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(state(window, &mut cx).1, 1);
        window
            .update(&mut cx, |c, _, cx| {
                c.transcript.update(cx, |t, cx| {
                    t.patch(4, vec![row("assistant", "Footer border added.")], cx)
                })
            })
            .unwrap();
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        assert_eq!(
            state(window, &mut cx),
            (vec![0, 3, 4], 1, Some((3, "footer".into())))
        );
    }

    #[gpui::test]
    fn a_custom_editor_find_shortcut_opens_the_transcript_find_too(cx: &mut TestAppContext) {
        let (window, mut cx) = conversation(cx);
        let cmd = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.update(|_, cx| {
            let mut prefs = crate::settings::Preferences::default();
            prefs
                .keybindings
                .insert("Editor: Find".into(), Some("alt-f".into()));
            crate::settings::apply_keybindings(&prefs, cx);
        });
        cx.simulate_keystrokes(&format!("{cmd}-f"));
        cx.run_until_parked();
        assert!(!window.update(&mut cx, |c, _, _| c.find_open()).unwrap());
        cx.simulate_keystrokes("alt-f");
        cx.run_until_parked();
        assert!(window.update(&mut cx, |c, _, _| c.find_open()).unwrap());
    }
}
