//! ChatPage.
use crate::settings::SettingsView;
use crate::theme::content;
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};

impl SettingsView {
    pub(super) fn chat_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.prefs.clone();
        let transcript =
            vec![
                // The title sits at the top of the two pictures.
                self.row_parts(
                    Some("transcript-layout"),
                    true,
                    "Transcript layout",
                    Some(self.description(
                        "Show prompts at full width or as right-aligned chat bubbles.",
                    )),
                    vec![self.transcript_layouts(&p.transcript_layout, cx)],
                    true,
                ),
                self.row(
                    Some("anchor-prompts"),
                    false,
                    "Anchor prompts to top",
                    Some("Place new prompts at the top, with replies below.".into()),
                    vec![self.toggle(
                        "anchor-prompts",
                        "Anchor prompts to top",
                        p.anchor_prompts,
                        false,
                        cx,
                        |this, on, cx| this.update(cx, |p| p.anchor_prompts = on),
                    )],
                ),
                self.row(
                    Some("paced-reveal"),
                    false,
                    "Smooth text reveal",
                    Some("Reveal incoming text gradually.".into()),
                    vec![self.toggle(
                        "paced-reveal",
                        "Smooth text reveal",
                        p.paced_reveal,
                        false,
                        cx,
                        |this, on, cx| this.update(cx, |p| p.paced_reveal = on),
                    )],
                ),
            ];
        let composer = vec![
            self.row(
                Some("follow-up"),
                true,
                "Follow-up behavior",
                Some("Queue follow-ups until the active turn finishes, or steer the active turn immediately.".into()),
                vec![self.segmented(
                    "follow-up",
                    "Follow-up behavior",
                    &p.follow_up,
                    &[("queue", "Queue"), ("steer", "Steer")],
                    cx,
                    |this, v, cx| this.update(cx, |p| p.follow_up = v.into()),
                )],
            ),
            self.row(
                Some("model-controls"),
                false,
                "Model controls",
                Some("Show model options beside the picker instead of inside the model menu.".into()),
                vec![self.segmented(
                    "model-controls",
                    "Model controls",
                    &p.model_controls,
                    &[("menu", "Menu"), ("beside", "Beside")],
                    cx,
                    |this, v, cx| this.update(cx, |p| p.model_controls = v.into()),
                )],
            ),
            self.row(
                Some("composer-autocorrect"),
                false,
                "Autocorrect",
                Some("Spell check and autocorrect prompts.".into()),
                vec![self.toggle(
                    "composer-autocorrect",
                    "Autocorrect",
                    p.autocorrect,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.autocorrect = on),
                )],
            ),
        ];
        let editor = vec![
            self.row(
                Some("file-tabs"),
                true,
                "File tabs",
                Some("Choose where files open.".into()),
                vec![self.segmented(
                    "file-tabs",
                    "File tabs",
                    &p.file_tabs,
                    &[("pane", "Beside chat"), ("workspace", "Top bar")],
                    cx,
                    |this, v, cx| this.update(cx, |p| p.file_tabs = v.into()),
                )],
            ),
            self.row(
                Some("file-word-wrap"),
                false,
                "Word wrap",
                Some("Wrap long lines to fit the editor.".into()),
                vec![self.toggle(
                    "file-word-wrap",
                    "Word wrap",
                    p.file_word_wrap,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.file_word_wrap = on),
                )],
            ),
            self.row(
                Some("show-excluded-files"),
                false,
                "Show excluded files",
                Some("Show files and folders Git excludes, such as build output and dependencies, in the explorer.".into()),
                vec![self.toggle(
                    "show-excluded-files",
                    "Show excluded files",
                    p.show_excluded_files,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.show_excluded_files = on),
                )],
            ),
            self.row(
                Some("format-on-save"),
                false,
                "Format on save",
                Some("Format supported files with Prettier before saving.".into()),
                vec![self.toggle(
                    "format-on-save",
                    "Format on save",
                    p.format_on_save,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.format_on_save = on),
                )],
            ),
        ];
        let review = vec![self.row(
            Some("diff-view"),
            true,
            "Diff view",
            Some("Review changes in the editor or in a combined diff.".into()),
            vec![self.segmented(
                "diff-view",
                "Diff view",
                &p.diff_view,
                &[("editor", "Editor"), ("unified", "Unified")],
                cx,
                |this, v, cx| this.update(cx, |p| p.diff_view = v.into()),
            )],
        )];
        let extras = vec![
            self.row(
                Some("composer-mascot"),
                true,
                "Composer mascot",
                Some("Show the project mascot while an agent works.".into()),
                vec![self.toggle(
                    "composer-mascot",
                    "Composer mascot",
                    p.composer_mascot,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.composer_mascot = on),
                )],
            ),
            self.row(
                Some("empty-session-games"),
                false,
                "Empty session games",
                Some("Show playable games in empty sessions.".into()),
                vec![self.toggle(
                    "empty-session-games",
                    "Empty session games",
                    p.empty_session_games,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.empty_session_games = on),
                )],
            ),
        ];
        vec![
            self.group(None, true, "Transcript", None, None, transcript),
            self.group(None, false, "Composer", None, None, composer),
            self.group(None, false, "Editor", None, None, editor),
            self.group(None, false, "Code review", None, None, review),
            self.group(None, false, "Extras", None, None, extras),
        ]
    }
}

impl SettingsView {
    /// Full width and Chat as small transcripts: your prompt tinted across
    /// the column, or as a bubble on the right.
    fn transcript_layouts(&self, value: &str, cx: &mut Context<Self>) -> AnyElement {
        let line = |width: f32, alpha: f32| {
            div()
                .h(px(6.))
                .w(gpui::relative(width))
                .rounded(px(3.))
                .bg(content(alpha))
        };
        // Your prompt, in the accent, so the two layouts read apart.
        let mine = || crate::theme::accent_color().opacity(0.55);
        div()
            .id("segmented-transcript-layout")
            .debug_selector(|| "settings-segmented-transcript-layout".into())
            .role(gpui::Role::RadioGroup)
            .aria_label("Transcript layout")
            .flex()
            .gap(px(14.))
            .children(
                [("full", "Full width"), ("chat", "Chat")].map(|(option, label)| {
                    let on = value == option;
                    let preview =
                        div()
                            .w(px(112.))
                            .h(px(72.))
                            .rounded(px(10.))
                            .p(px(8.))
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .bg(content(0.04))
                            .map(|d| {
                                if option == "chat" {
                                    d.child(div().flex().justify_end().child(
                                        div().w(px(56.)).h(px(9.)).rounded(px(5.)).bg(mine()),
                                    ))
                                } else {
                                    d.child(line(1., 0.).h(px(8.)).bg(mine()))
                                }
                            })
                            .child(line(0.92, 0.14))
                            .child(line(0.7, 0.1))
                            .child(line(0.85, 0.1));
                    crate::settings::widgets::picture_option(
                        SharedString::from(format!("segmented-transcript-layout-{option}")),
                        preview,
                        label,
                        on,
                    )
                    .debug_selector(move || format!("settings-transcript-layout-{option}"))
                    .role(gpui::Role::RadioButton)
                    .aria_label(label)
                    .aria_selected(on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update(cx, |p| p.transcript_layout = option.into())
                    }))
                }),
            )
            .into_any_element()
    }
}
