//! The look card. Placeholder until the card is built to the canvas.
use super::{card::SetupCard, ui};
use gpui::{AnyElement, Context, Window, prelude::*};

pub(super) fn render(card: &mut SetupCard, _: &mut Window, _: &mut Context<SetupCard>) -> AnyElement {
    let _ = card;
    ui::card(28., 22.).into_any_element()
}
