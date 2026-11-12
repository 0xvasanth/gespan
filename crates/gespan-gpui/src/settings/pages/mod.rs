//! One file per Settings page.
mod appearance;
mod archive;
mod browser;
mod chat;
mod connections;
mod diagnostics;
mod dictation;
mod general;
mod herdr_machines;
mod inbox;
mod keybindings;
mod mcp;
mod mcp_catalog;
mod mobile;
mod providers;
mod services;
mod skills;
mod worktrees;

#[cfg(test)]
pub(super) use appearance::appearance_summary;
pub(super) use appearance::parse_hex;
pub(crate) use appearance::{ACCENTS, swatch};
pub(super) use connections::ConnectionsState;
pub(super) use diagnostics::DiagnosticsState;
#[cfg(test)]
pub(super) use diagnostics::{CounterRow, kind_tone};
pub(super) use dictation::DictationState;
#[cfg(test)]
pub(super) use general::version_pill;
pub(super) use herdr_machines::HerdrMachinesState;
#[cfg(test)]
pub(super) use keybindings::{BINDINGS, by_area, key_caps};
pub(crate) use keybindings::{command_shortcut, shortcut_label};
pub(super) use mobile::MobileState;
#[cfg(test)]
pub(super) use providers::install_command;
#[cfg(test)]
pub(super) use skills::skill_bucket;
pub(super) use worktrees::WorktreeScripts;

use super::{Section, SettingsView};
use gpui::{AnyElement, Context, Window};

impl SettingsView {
    pub(super) fn page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        match self.section {
            Section::General => self.general_page(cx),
            Section::Diagnostics => self.diagnostics_page(cx),
            Section::Devices => {
                // Devices holds machines, herdr machines and phones.
                let mut page = self.connections_page(cx);
                page.extend(self.herdr_machines_page(cx));
                page.extend(self.mobile_page(cx));
                page
            }
            Section::Services => self.services_page(cx),
            Section::Appearance => self.appearance_page(window, cx),
            Section::Keybindings => self.keybindings_page(cx),
            Section::Dictation => self.dictation_page(cx),
            Section::Chat => self.chat_page(cx),
            Section::Providers => self.providers_page(cx),
            Section::Mcp => self.mcp_page(cx),
            Section::Browser => self.browser_page(cx),
            Section::Skills => self.skills_page(cx),
            Section::Inbox => self.inbox_page(cx),
            Section::Archive => self.archive_page(cx),
            Section::Worktrees => self.worktrees_page(cx),
        }
    }
}
