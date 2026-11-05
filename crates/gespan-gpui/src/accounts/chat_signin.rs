//! The chat's sign-in card drives the same provider sign-in as Settings, with
//! the CLI's link, code and code prompt shown in the card.
use super::*;
use gespan_core::accounts::LoginPrompt;

/// A chat's view of its provider sign-in.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ChatSignIn {
    pub(crate) prompt: LoginPrompt,
    pub(crate) error: Option<String>,
    pub(crate) complete: bool,
}
impl ChatSignIn {
    pub(crate) fn running(&self) -> bool {
        !self.complete && self.error.is_none()
    }
}

impl Accounts {
    pub(crate) fn begin_chat_auth(&mut self, kind: Kind, id: &str, cx: &mut Context<Self>) {
        self.start_auth(kind, id, AuthAction::SignIn, None, Surface::Chat, cx);
    }
    fn chat(&self, kind: Kind, id: &str) -> Option<&SignIn> {
        self.signin.as_ref().filter(|s| {
            s.surface == Surface::Chat
                && s.kind == kind
                && s.profile.id == id
                && s.action == AuthAction::SignIn
        })
    }
    pub(crate) fn chat_sign_in(&self, kind: Kind, id: &str) -> Option<ChatSignIn> {
        self.chat(kind, id).map(|s| ChatSignIn {
            prompt: s.prompt.clone(),
            error: s.error.clone(),
            complete: s.complete,
        })
    }
    /// Another sign-in, from Settings or another chat, is still running.
    pub(crate) fn auth_busy_elsewhere(&self, kind: Kind, id: &str) -> bool {
        self.working
            || self
                .signin
                .as_ref()
                .is_some_and(|s| !s.complete && s.error.is_none())
                && self.chat(kind, id).is_none()
    }
    /// Types a code from the provider's page into the waiting CLI.
    pub(crate) fn send_chat_code(&mut self, kind: Kind, id: &str, code: String) -> bool {
        let code = code.trim().to_owned();
        if code.is_empty() {
            return false;
        }
        self.chat(kind, id)
            .and_then(|s| s.codes.as_ref())
            .is_some_and(|codes| codes.send(code).is_ok())
    }
    pub(crate) fn cancel_chat_auth(&mut self, kind: Kind, id: &str, cx: &mut Context<Self>) {
        if self
            .chat(kind, id)
            .is_none_or(|s| !s.complete && s.error.is_some())
        {
            return;
        }
        let signin = self.signin.as_mut().unwrap();
        if signin.complete {
            return;
        }
        signin.cancel.store(true, Ordering::Relaxed);
        signin.codes = None;
        signin.error = Some("Sign-in cancelled".into());
        cx.notify();
    }
}

#[cfg(test)]
#[path = "chat_signin_tests.rs"]
mod tests;
