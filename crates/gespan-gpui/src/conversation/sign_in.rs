//! A provider that signed out mid-chat gets a Sign in card in place of its
//! error. Once signed in, the failed message runs again where it sits, so the
//! thread never shows it twice.
use super::*;
use crate::accounts::chat_signin::ChatSignIn;
use gpui::App;

#[derive(Default)]
pub(super) struct State {
    /// The running turn's prompt and settings, until it ends.
    launched: Option<(String, Settings)>,
    /// The turn that ended signed out.
    failed: Option<Failed>,
    /// A replay waiting for `launch_prompt` to pick it up.
    pub(super) replaying: Option<Settings>,
    code: Option<Entity<input::Input>>,
    _code_submit: Option<Subscription>,
    /// Repaints the card as the sign-in it started moves along.
    _progress: Option<Subscription>,
}
#[derive(Clone)]
struct Failed {
    kind: Kind,
    account: String,
    /// What to send again; gone after an app restart.
    replay: Option<(String, Settings)>,
    /// The user started the sign-in from this chat's card.
    started_here: bool,
    /// The provider is signed in again, by this card or elsewhere.
    signed_in: bool,
}

impl Conversation {
    pub(super) fn remember_launch(&mut self, prompt: &str, settings: &Settings) {
        self.sign_in.launched = Some((prompt.to_owned(), settings.clone()));
    }
    /// Settles the card when a turn ends.
    pub(super) fn turn_signed_out(&mut self, signed_out: bool) {
        let launched = self.sign_in.launched.take();
        self.sign_in.failed = signed_out.then(|| Failed {
            kind: Kind::from_id(&self.summary.provider),
            account: self.settings.account.id.clone(),
            replay: launched,
            started_here: false,
            signed_in: false,
        });
    }
    /// The signed-out turn this chat ends with, including one saved before a
    /// restart, whose message can no longer be sent again automatically.
    fn signed_out_turn(&mut self, cx: &App) -> Option<Failed> {
        if self.busy || self.is_remote() {
            return None;
        }
        if let Some(failed) = &self.sign_in.failed {
            return Some(failed.clone());
        }
        let kind = self.transcript.read(cx).signed_out_tail()?;
        (kind == Kind::from_id(&self.summary.provider)).then(|| Failed {
            kind,
            account: self.settings.account.id.clone(),
            replay: None,
            started_here: false,
            signed_in: false,
        })
    }
    fn begin_chat_sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(mut failed) = self.signed_out_turn(cx) else {
            return;
        };
        let Some(center) = crate::accounts::get(cx) else {
            return;
        };
        failed.started_here = true;
        self.sign_in.failed = Some(failed.clone());
        if let Some(code) = &self.sign_in.code {
            code.update(cx, |code, cx| code.set_text(String::new(), cx));
        }
        self.sign_in._progress = Some(cx.observe(&center, |_, _, cx| cx.notify()));
        center.update(cx, |center, cx| {
            center.begin_chat_auth(failed.kind, &failed.account, cx)
        });
        cx.notify();
    }
    /// The provider account signed in again, from this card or elsewhere.
    pub(super) fn signed_in_again(&mut self, cx: &mut Context<Self>) {
        let Some(mut failed) = self.signed_out_turn(cx) else {
            return;
        };
        if failed.started_here {
            self.replay_signed_out_turn(failed, cx);
        } else {
            // Signed in from Settings or another chat: this chat's message
            // goes again only when its user asks.
            failed.signed_in = true;
            self.sign_in.failed = Some(failed);
            cx.notify();
        }
    }
    fn replay_signed_out_turn(&mut self, failed: Failed, cx: &mut Context<Self>) {
        self.sign_in.failed = None;
        self.sign_in._progress = None;
        match failed.replay {
            Some((prompt, mut settings)) => {
                settings.replay_failed_turn = true;
                // The failed turn may already have created its worktree.
                settings.worktree_cwd = self.settings.worktree_cwd.clone();
                settings.new_worktree = self.settings.new_worktree;
                if let Some(turn) = settings.turn_model.as_mut() {
                    turn.started_at = Some(gespan_core::time::now_ms());
                }
                // The replay goes as what the failed turn was: the user's own
                // only when they had typed it.
                let typed = settings.user_typed;
                self.sign_in.replaying = Some(settings);
                self.launch_prompt(prompt, typed, cx);
            }
            None => {
                self.status = format!(
                    "Signed in to {}. Send your message again to continue.",
                    failed.kind.label()
                );
            }
        }
        cx.notify();
    }
    fn code_field(&mut self, cx: &mut Context<Self>) -> Entity<input::Input> {
        if let Some(field) = &self.sign_in.code {
            return field.clone();
        }
        let field = cx.new(|cx| {
            let mut field = input::Input::field("Sign-in code", "Paste code", content(0.4), cx);
            field.set_font_size(12.);
            field
        });
        self.sign_in._code_submit =
            Some(cx.subscribe(&field, |this, _, _: &input::Submitted, cx| {
                this.submit_sign_in_code(cx)
            }));
        self.sign_in.code = Some(field.clone());
        field
    }
    fn submit_sign_in_code(&mut self, cx: &mut Context<Self>) {
        let (Some(failed), Some(field), Some(center)) = (
            self.sign_in.failed.clone(),
            self.sign_in.code.clone(),
            crate::accounts::get(cx),
        ) else {
            return;
        };
        let Some(code) = field.update(cx, |field, cx| field.take(cx)) else {
            return;
        };
        center.update(cx, |center, _| {
            center.send_chat_code(failed.kind, &failed.account, code)
        });
        cx.notify();
    }
    pub(super) fn sign_in_card(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let failed = self.signed_out_turn(cx)?;
        let center = crate::accounts::get(cx);
        let (state, busy) = center
            .map(|center| {
                let center = center.read(cx);
                (
                    center.chat_sign_in(failed.kind, &failed.account),
                    center.auth_busy_elsewhere(failed.kind, &failed.account),
                )
            })
            .unwrap_or_default();
        let state = state.filter(|_| failed.started_here);
        let running = state.as_ref().is_some_and(ChatSignIn::running);
        let label = failed.kind.label();
        let title = if failed.signed_in {
            format!("Signed in to {label}")
        } else {
            format!("Signed out of {label}")
        };
        let detail: String = match &state {
            _ if failed.signed_in => match failed.replay {
                Some(_) => "Send your message again to carry on.".into(),
                None => "Send your message again to continue.".into(),
            },
            Some(state) if running && state.prompt.wants_code => {
                "Finish in your browser. If the page shows a code, paste it here.".into()
            }
            Some(state) if running && state.prompt.code.is_some() => {
                "Confirm this code on the sign-in page.".into()
            }
            Some(_) if running => "Waiting for you to finish signing in in your browser…".into(),
            Some(state) => format!(
                "Sign-in didn't finish: {}",
                state.error.as_deref().unwrap_or("try again")
            ),
            None if busy => "Another sign-in is in progress.".into(),
            None if failed.replay.is_some() => {
                "Sign in to continue. Your message will run again.".into()
            }
            None => "Sign in to continue.".into(),
        };
        let prompt = state.as_ref().map(|s| s.prompt.clone()).unwrap_or_default();
        let code_field = (running && prompt.wants_code).then(|| self.code_field(cx));
        let mut actions = div().flex().items_center().gap_2().flex_wrap();
        if failed.signed_in {
            actions = actions.child(
                crate::settings::widgets::primary_button(
                    "conversation-sign-in-replay",
                    "Send again",
                    failed.replay.is_none(),
                )
                .child("Send again")
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(failed) = this.sign_in.failed.clone() {
                        this.replay_signed_out_turn(failed, cx);
                    }
                })),
            );
        } else if running {
            if let Some(url) = prompt.url.clone() {
                actions = actions.child(
                    crate::settings::widgets::secondary_button(
                        "conversation-sign-in-open",
                        "Open sign-in page",
                        false,
                        false,
                    )
                    .child("Open sign-in page")
                    .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            actions = actions.child(
                crate::settings::widgets::secondary_button(
                    "conversation-sign-in-cancel",
                    "Cancel sign-in",
                    false,
                    false,
                )
                .child("Cancel")
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let (Some(failed), Some(center)) =
                        (this.sign_in.failed.clone(), crate::accounts::get(cx))
                    {
                        center.update(cx, |center, cx| {
                            center.cancel_chat_auth(failed.kind, &failed.account, cx)
                        });
                    }
                })),
            );
        } else {
            actions = actions.child(
                crate::settings::widgets::primary_button("conversation-sign-in", "Sign in", busy)
                    .child("Sign in")
                    .when(!busy, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| this.begin_chat_sign_in(cx)))
                    }),
            );
        }
        Some(
            div()
                .id("conversation-sign-in-card")
                .role(Role::Status)
                .aria_label(title.clone())
                .mx_2()
                .mb_2()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(content(0.08))
                .bg(content(0.04))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(provider_icon(failed.kind.id(), 20.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().text_size(px(13.)).child(title))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(content(0.55))
                                        .child(detail),
                                ),
                        )
                        .child(actions),
                )
                .when_some(prompt.code.clone().filter(|_| running), |d, code| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("conversation-sign-in-code")
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .bg(content(0.06))
                                    .font(gpui::font(crate::fonts::monospace()))
                                    .text_size(px(14.))
                                    .child(code.clone()),
                            )
                            .child(
                                crate::settings::widgets::secondary_button(
                                    "conversation-sign-in-copy",
                                    "Copy code",
                                    false,
                                    false,
                                )
                                .child("Copy")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        code.clone(),
                                    ))
                                }),
                            ),
                    )
                })
                .when_some(code_field, |d, field| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(28.))
                                    .px_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(content(0.12))
                                    .flex()
                                    .items_center()
                                    .child(field),
                            )
                            .child(
                                crate::settings::widgets::secondary_button(
                                    "conversation-sign-in-submit-code",
                                    "Submit code",
                                    false,
                                    false,
                                )
                                .child("Submit")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.submit_sign_in_code(cx)),
                                ),
                            ),
                    )
                })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::{store::Store, transcript::Row};
    use gpui::{TestAppContext, WindowHandle};
    use std::sync::Mutex;

    const SIGNED_OUT: &str = "Signed out of Codex. Your access token could not be refreshed.";

    fn row(role: &str, text: &str) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first: true,
            sent_at: None,
        }
    }
    /// A Codex chat whose saved transcript is `rows`. Its folder does not
    /// exist, so a launched turn fails before any provider runs.
    fn conversation(rows: Vec<Row>, cx: &mut TestAppContext) -> WindowHandle<Conversation> {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let summary = store.create("codex", "/missing/sign-in-project").unwrap();
        store.write_rows(summary.id, 0, &rows).unwrap();
        cx.update(|cx| {
            input::bind(cx);
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                    view.loaded = true;
                    view.transcript.update(cx, |v, cx| v.patch(0, rows, cx));
                    view
                })
            })
            .unwrap()
        })
    }
    /// The chat just ran "Fix the login", and it ended signed out.
    fn signed_out(cx: &mut TestAppContext) -> WindowHandle<Conversation> {
        let window = conversation(
            vec![row("user", "Fix the login"), row("error", SIGNED_OUT)],
            cx,
        );
        window
            .update(cx, |view, _, _| {
                view.remember_launch("Fix the login", &Settings::default());
                view.turn_signed_out(true);
            })
            .unwrap();
        window
    }

    #[gpui::test]
    fn a_signed_out_turn_offers_sign_in_and_the_error_reads_quietly(cx: &mut TestAppContext) {
        let window = signed_out(cx);
        window
            .update(cx, |view, _, cx| {
                let failed = view.signed_out_turn(cx).expect("a sign-in card");
                assert_eq!(failed.kind, Kind::Codex);
                assert!(failed.replay.is_some(), "the message can run again");
                assert!(view.sign_in_card(cx).is_some());
                let transcript = view.transcript.read(cx);
                assert_eq!(transcript.signed_out_tail(), Some(Kind::Codex));
                assert_eq!(
                    transcript.message_signed_out(1),
                    Some(true),
                    "the card carries the provider's wording, not the red row"
                );
            })
            .unwrap();
    }
    #[gpui::test]
    fn signing_in_here_replays_the_failed_turn_once(cx: &mut TestAppContext) {
        let window = signed_out(cx);
        turn_capture::start();
        window
            .update(cx, |view, _, cx| {
                view.sign_in.failed.as_mut().unwrap().started_here = true;
                view.account_authenticated(cx);
                assert!(view.busy);
                assert!(view.signed_out_turn(cx).is_none(), "the card is gone");
                // A second sign-in event must not send it again.
                view.account_authenticated(cx);
                assert_eq!(
                    view.transcript.read(cx).source_len(),
                    2,
                    "no second copy of the prompt"
                );
            })
            .unwrap();
        let id = window.update(cx, |view, _, _| view.summary.id).unwrap();
        assert_eq!(turn_capture::all(), [(id, "Fix the login".to_owned())]);
        cx.run_until_parked();
    }
    #[gpui::test]
    fn a_sign_in_from_elsewhere_waits_for_send_again(cx: &mut TestAppContext) {
        let window = signed_out(cx);
        turn_capture::start();
        window
            .update(cx, |view, _, cx| {
                view.account_authenticated(cx);
                assert!(!view.busy);
                let failed = view.signed_out_turn(cx).unwrap();
                assert!(failed.signed_in);
                assert!(turn_capture::all().is_empty());
                view.replay_signed_out_turn(failed, cx);
                assert!(view.busy);
            })
            .unwrap();
        assert_eq!(turn_capture::all().len(), 1);
        cx.run_until_parked();
    }
    #[gpui::test]
    fn without_a_sign_in_nothing_is_sent_again(cx: &mut TestAppContext) {
        // A cancelled or failed sign-in emits no Authenticated event.
        let window = signed_out(cx);
        turn_capture::start();
        cx.run_until_parked();
        window
            .update(cx, |view, _, cx| {
                assert!(view.signed_out_turn(cx).is_some());
                assert!(!view.busy);
            })
            .unwrap();
        assert!(turn_capture::all().is_empty());
    }
    #[gpui::test]
    fn after_a_restart_the_card_returns_but_asks_to_send_again(cx: &mut TestAppContext) {
        let window = conversation(
            vec![row("user", "Fix the login"), row("error", SIGNED_OUT)],
            cx,
        );
        turn_capture::start();
        window
            .update(cx, |view, _, cx| {
                let mut failed = view.signed_out_turn(cx).expect("saved sign-out");
                assert!(failed.replay.is_none());
                failed.started_here = true;
                view.sign_in.failed = Some(failed);
                view.account_authenticated(cx);
                assert!(!view.busy);
                assert_eq!(
                    view.status,
                    "Signed in to Codex. Send your message again to continue."
                );
            })
            .unwrap();
        assert!(turn_capture::all().is_empty());
    }
    #[gpui::test]
    fn other_failures_get_no_sign_in_card(cx: &mut TestAppContext) {
        for error in [
            "Usage limit reached: try again at 5pm",
            "unexpected status 401 Unauthorized",
            "Provider exited before completing the turn. Not logged in",
            // Signed out of another provider than this chat's.
            "Signed out of Claude Code. Not logged in · Please run /login",
        ] {
            let window = conversation(vec![row("user", "Hi"), row("error", error)], cx);
            window
                .update(cx, |view, _, cx| {
                    view.remember_launch("Hi", &Settings::default());
                    view.turn_signed_out(false);
                    assert!(view.signed_out_turn(cx).is_none(), "{error}");
                    assert!(view.sign_in_card(cx).is_none(), "{error}");
                })
                .unwrap();
        }
    }
}
