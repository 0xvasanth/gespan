//! The Diagnostics page: local tools for reporting a problem. Nothing here
//! sends anything; the report sheet only opens a prefilled issue.
use crate::settings::SettingsView;
use gpui::{AnyElement, Context};

impl SettingsView {
    pub(super) fn diagnostics_page(&mut self, _cx: &mut Context<Self>) -> Vec<AnyElement> {
        Vec::new()
    }
}
