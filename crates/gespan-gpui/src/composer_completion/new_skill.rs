//! SkillPicker.tsx's "New skill" row and CreateSkillForm: a starter SKILL.md
//! named from the typed command, in the project or in your personal skills.
//! Creating it clears the command, refreshes the list and opens the file.
use super::*;
use gespan_core::skills::{is_valid_skill_name, slug_name};
use gpui::AnyElement;

impl Completion {
    /// The skills list, not files, notes or MCP servers.
    pub(super) fn offers_new_skill(&self) -> bool {
        !self.mcp_open
            && self
                .token
                .as_ref()
                .is_some_and(|token| token.trigger == Trigger::Skill)
    }
    /// A project skill needs a local project folder.
    fn project_skills(&self) -> bool {
        !self.cwd.is_empty() && self.cwd != "~"
    }
    pub(super) fn new_skill_row(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("completion-new-skill")
            .debug_selector(|| "completion-new-skill".into())
            .role(Role::Button)
            .aria_label("New skill")
            .flex_shrink_0()
            .mt_1()
            .px_2()
            .py_2()
            .rounded_md()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.))
            .text_color(content(0.7))
            .cursor_pointer()
            .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
            .child(icon("plus", 14.))
            .child("New skill")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.start_new_skill(window, cx)))
            .into_any_element()
    }
    pub(super) fn start_new_skill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.token.as_ref().map_or("", |token| token.query.as_str());
        let name = cx.new(|cx| {
            let mut input = input::Input::field("Skill name", "skill-name", content(0.4), cx);
            input.set_text(slug_name(query), cx);
            input
        });
        let subscriptions = [
            cx.observe(&name, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &name,
                window,
                |this, _, _: &input::Submitted, window, cx| this.create_new_skill(window, cx),
            ),
        ];
        window.focus(&name.focus_handle(cx), cx);
        self.creating = Some(NewSkill {
            name,
            personal: !self.project_skills(),
            error: None,
            busy: false,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }
    pub(super) fn cancel_new_skill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.creating = None;
        window.focus(&self.input.focus_handle(cx), cx);
        cx.notify();
    }
    fn new_skill_slug(&self, cx: &App) -> Option<String> {
        let form = self.creating.as_ref()?;
        Some(slug_name(&form.name.read(cx).content))
    }
    fn create_new_skill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slug) = self.new_skill_slug(cx) else {
            return;
        };
        let cwd = self.cwd.clone();
        let Some(form) = self.creating.as_mut() else {
            return;
        };
        if form.busy || !is_valid_skill_name(&slug) {
            return;
        }
        form.busy = true;
        form.error = None;
        let personal = form.personal;
        let work = cx
            .background_executor()
            .spawn(async move { gespan_core::skills::create_blank(&cwd, &slug, personal) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(path) => this.new_skill_created(path, window, cx),
                Err(error) => {
                    if let Some(form) = &mut this.creating {
                        form.busy = false;
                        form.error = Some(error);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
    fn new_skill_created(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.creating = None;
        // The command that opened the picker goes; the rest of the draft stays.
        if let Some(token) = self.token.take() {
            let mut range = token.range;
            if self.input.read(cx).content[range.end..].starts_with(' ') {
                range.end += 1;
            }
            self.input.update(cx, |input, cx| {
                input.replace_completion(range, "", window, cx)
            });
        }
        self.token = None;
        // The next "/" reads the skills again, with this one in them.
        self.loaded = None;
        window.focus(&self.input.focus_handle(cx), cx);
        window.dispatch_action(
            Box::new(crate::file_editor::OpenPath(
                path.to_string_lossy().into_owned(),
            )),
            cx,
        );
        cx.notify();
    }
    pub(super) fn new_skill_form(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.creating.as_ref()?;
        let slug = slug_name(&form.name.read(cx).content);
        let typed = !form.name.read(cx).content.trim().is_empty();
        let valid = is_valid_skill_name(&slug);
        let busy = form.busy;
        let project = self.project_skills();
        let scope = |personal: bool, label: &'static str, hint: &'static str| {
            let selected = form.personal == personal;
            let disabled = busy || (!personal && !project);
            div()
                .id(label)
                .role(Role::Button)
                .aria_label(label)
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .rounded_md()
                .px_2()
                .py(px(6.))
                .bg(content(if selected { 0.16 } else { 0.1 }))
                .text_color(content(if selected { 1. } else { 0.7 }))
                .when(disabled, |d| d.opacity(0.4))
                .child(div().text_size(px(12.)).child(label))
                .child(
                    div()
                        .truncate()
                        .font_family(crate::fonts::monospace())
                        .text_size(px(10.))
                        .text_color(content(0.4))
                        .child(hint),
                )
                .when(!disabled, |d| {
                    d.cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(form) = &mut this.creating {
                                form.personal = personal;
                            }
                            cx.notify();
                        }))
                })
        };
        let message = form.error.clone().map(|error| (error, 0.7)).or_else(|| {
            (typed && !valid).then(|| {
                (
                    "Use lowercase letters, numbers, and hyphens.".to_owned(),
                    0.5,
                )
            })
        });
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(label)
                .rounded_md()
                .px_2()
                .py_1()
                .text_size(px(12.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        Some(
            div()
                .id("completion-new-skill-form")
                .debug_selector(|| "completion-new-skill-form".into())
                .role(Role::Form)
                .aria_label("New skill")
                .px(px(10.))
                .py_2()
                .child(
                    div()
                        .mb_2()
                        .text_size(px(11.))
                        .text_color(content(0.5))
                        .child("Writes a starter SKILL.md you can edit."),
                )
                .child(
                    div()
                        .mb_2()
                        .px_2()
                        .py(px(6.))
                        .rounded_md()
                        .bg(content(0.1))
                        .font_family(crate::fonts::monospace())
                        .child(form.name.clone()),
                )
                .child(
                    div()
                        .mb_2()
                        .flex()
                        .gap_1()
                        .child(scope(false, "Project", ".agents/skills"))
                        .child(scope(true, "Personal", "~/.agents/skills")),
                )
                .when_some(message, |d, (text, alpha)| {
                    d.child(
                        div()
                            .mb_2()
                            .text_size(px(12.))
                            .text_color(content(alpha))
                            .child(text),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_1()
                        .child(
                            button("completion-new-skill-cancel", "Cancel")
                                .text_color(content(0.5))
                                .when(!busy, |d| {
                                    d.cursor_pointer()
                                        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.cancel_new_skill(window, cx)
                                        }))
                                })
                                .child("Cancel"),
                        )
                        .child(
                            button("completion-new-skill-create", "Create")
                                .bg(content(0.2))
                                .text_color(content(1.))
                                .when(!valid || busy, |d| d.opacity(0.4))
                                .when(valid && !busy, |d| {
                                    d.cursor_pointer().on_click(cx.listener(
                                        |this, _, window, cx| this.create_new_skill(window, cx),
                                    ))
                                })
                                .child(if busy { "Creating…" } else { "Create" }),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests;
