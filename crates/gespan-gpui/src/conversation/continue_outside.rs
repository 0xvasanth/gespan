//! The composer's Continue outside button: the way to pick this chat up in a
//! terminal outside Gespan. The shell opens the panel.
use super::*;

/// The chip asked for the panel, to open above this anchor.
pub(crate) struct ContinueOutside(pub(crate) AnchorSlot);
impl EventEmitter<ContinueOutside> for Conversation {}

impl Conversation {
    /// Local sessions only: a remote project's agent runs on another machine.
    pub(super) fn continue_chip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let cwd = self.execution_cwd();
        if cwd.is_empty() || cwd.starts_with("remote://") {
            return None;
        }
        let agent = gespan_core::provider::Kind::from_id(&self.summary.provider).label();
        Some(
            div()
                .id("continue-outside")
                .debug_selector(|| "continue-outside".into())
                .role(Role::Button)
                .aria_label(format!("Continue this {agent} chat in your terminal"))
                .child(self.continue_anchor.probe())
                .cursor_pointer()
                .size(px(20.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .group("continue-outside")
                .hover(|s| s.bg(content(0.06)))
                .child(
                    icon("external-link", 13.)
                        .text_color(content(0.5))
                        .group_hover("continue-outside", |s| s.text_color(content(0.9))),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.emit(ContinueOutside(this.continue_anchor.clone()));
                }))
                .into_any_element(),
        )
    }
}
