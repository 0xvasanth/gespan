//! Read-only notification snapshots and request-specific approval actions.

use super::*;
use crate::notifications::approvals::{InputKind, PendingInput};

impl Conversation {
    pub(crate) fn notification_was_cancelled(&self) -> bool {
        self.notification_cancelled
    }
    pub(crate) fn pending_notification_inputs(&self, cx: &gpui::App) -> Vec<PendingInput> {
        self.approvals
            .iter()
            .map(|approval| {
                let cwd = (!self.summary.cwd.starts_with("remote://"))
                    .then_some(self.summary.cwd.as_str());
                PendingInput::approval_in(approval, cwd)
            })
            .chain(self.questions.iter().map(|(card, _)| {
                let question = card.read(cx);
                PendingInput {
                    request_id: question.id().to_owned(),
                    kind: InputKind::Question,
                    label: question.notification_label().to_owned(),
                }
            }))
            .collect()
    }

    pub(crate) fn notification_reply_body(&self, cx: &gpui::App) -> String {
        self.transcript.read(cx).notification_reply_body()
    }

    /// Resolve the exact approval represented by the card, even if a newer
    /// request arrived or an older one was resolved while the card was visible.
    /// Questions must open their answer form; they cannot be approved generically.
    pub(crate) fn decide_notification_request(
        &mut self,
        request_id: &str,
        decision: Decision,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(index) = self
            .approvals
            .iter()
            .position(|approval| approval.id == request_id)
        else {
            return false;
        };
        if !matches!(
            decision,
            Decision::Allow | Decision::Session | Decision::Deny
        ) || (matches!(decision, Decision::Session) && !self.approvals[index].can_remember)
        {
            return false;
        }
        let Some(controls) = &self.controls else {
            self.status = "The provider is no longer waiting for this approval".into();
            cx.notify();
            return false;
        };
        if controls
            .send(Control {
                id: request_id.to_owned(),
                decision,
            })
            .is_err()
        {
            self.status = "The provider is no longer waiting for this approval".into();
            cx.notify();
            return false;
        }
        self.approvals.remove(index);
        self.status = if !self.approvals.is_empty() {
            "Waiting for approval"
        } else if !self.questions.is_empty() {
            "Waiting for your answer"
        } else {
            "Working…"
        }
        .into();
        // Request-level observers need an update even if another request remains.
        cx.emit(SessionChanged);
        cx.notify();
        true
    }
}

impl Conversation {
    /// The approvals waiting in this session, oldest first.
    pub(crate) fn pending_approvals(&self) -> impl Iterator<Item = &gespan_core::agent::Approval> {
        self.approvals.iter()
    }

    /// The question the session will take an answer for now: the front one.
    pub(crate) fn front_question(&self) -> Option<Entity<crate::questions::QuestionCard>> {
        self.questions.front().map(|(card, _)| card.clone())
    }

    /// Every loaded row of the session's history, older pages included.
    pub(crate) fn history_rows<'a>(&self, cx: &'a gpui::App) -> &'a [gespan_core::transcript::Row] {
        self.transcript.read(cx).history_rows()
    }

    #[cfg(test)]
    pub(crate) fn push_history_for_test(&mut self, role: &str, text: &str, cx: &mut Context<Self>) {
        let row = gespan_core::transcript::Row {
            role: role.into(),
            text: text.into(),
            first: true,
            sent_at: Some(1_000),
        };
        self.transcript.update(cx, |t, cx| {
            let at = t.source_rows().len();
            t.patch(at, vec![row], cx);
        });
    }

    #[cfg(test)]
    pub(crate) fn drop_questions_for_test(&mut self) {
        self.questions.clear();
    }
}
