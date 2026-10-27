//! The app-wide Hold state every session and scheduler checks before starting work.
use gespan_core::hold::{HoldState, Phase};

/// `Some` from the moment Hold is confirmed until Resume has started every session.
#[derive(Default)]
pub(crate) struct Hold(pub Option<HoldState>);

impl gpui::Global for Hold {}

pub(crate) fn active(cx: &gpui::App) -> bool {
    cx.try_global::<Hold>().is_some_and(|hold| hold.0.is_some())
}

pub(crate) fn phase(cx: &gpui::App) -> Option<Phase> {
    cx.try_global::<Hold>()
        .and_then(|hold| hold.0.as_ref())
        .map(|state| state.phase)
}
