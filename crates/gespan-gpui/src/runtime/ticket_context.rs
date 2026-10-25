//! Tell the agent once per provider context which ticket its session is
//! linked to and what the gespan_ticket tools may do with it.
use gespan_core::{store::Store, tickets::TicketRef};

const VERSION: u8 = 1;

pub(super) struct TicketContext {
    key: String,
    text: String,
    may_change_status: bool,
}

impl TicketContext {
    pub(super) fn prepare(
        store: &Store,
        session: i64,
        ticket: &TicketRef,
        may_change_status: bool,
        provider_id: Option<&str>,
    ) -> Result<Option<Self>, String> {
        let context = Self {
            key: format!("ticket-context:{session}:{}", ticket.key),
            text: introduction(ticket, may_change_status),
            may_change_status,
        };
        if let Some(provider_id) = provider_id {
            let expected = context.receipt(provider_id);
            if store.setting(&context.key)?.as_deref() == Some(expected.as_str()) {
                return Ok(None);
            }
        }
        Ok(Some(context))
    }

    pub(super) fn prepend(&self, prompt: String) -> String {
        format!("{}\n\n{prompt}", self.text)
    }

    /// Call only after the introduction reached a successfully completed turn.
    pub(super) fn remember(&self, store: &Store, provider_id: Option<&str>) -> Result<(), String> {
        if let Some(provider_id) = provider_id.filter(|id| !id.trim().is_empty()) {
            store.save_setting(&self.key, &self.receipt(provider_id))?;
        }
        Ok(())
    }

    fn receipt(&self, provider_id: &str) -> String {
        serde_json::json!({
            "version": VERSION,
            "providerId": provider_id,
            "mayChangeStatus": self.may_change_status,
        })
        .to_string()
    }
}

fn introduction(ticket: &TicketRef, may_change_status: bool) -> String {
    let title = ticket
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(|title| format!(" (\"{title}\")"))
        .unwrap_or_default();
    format!(
        "This session is linked to {} ticket {}{title}. Use the gespan_ticket tools to read it, comment on it{}.",
        ticket.tracker.label(),
        ticket.key,
        if may_change_status {
            ", and change its status"
        } else {
            ""
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::tickets::Tracker;

    fn ticket(key: &str, title: Option<&str>) -> TicketRef {
        TicketRef {
            tracker: Tracker::Jira,
            key: key.into(),
            url: None,
            title: title.map(String::from),
            status: None,
            status_group: None,
            fetched_at: None,
        }
    }

    #[test]
    fn introduction_names_the_ticket_and_what_the_agent_may_do() {
        assert_eq!(
            introduction(&ticket("ENG-7", Some("Fix login")), true),
            "This session is linked to Jira ticket ENG-7 (\"Fix login\"). Use the gespan_ticket tools to read it, comment on it, and change its status."
        );
        assert_eq!(
            introduction(&ticket("ENG-7", None), false),
            "This session is linked to Jira ticket ENG-7. Use the gespan_ticket tools to read it, comment on it."
        );
        assert_eq!(
            introduction(&ticket("ENG-7", Some("  ")), true),
            "This session is linked to Jira ticket ENG-7. Use the gespan_ticket tools to read it, comment on it, and change its status."
        );
    }

    #[test]
    fn introduced_once_per_provider_context_ticket_and_permission() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let linked = ticket("ENG-7", Some("Fix login"));
        let prepare = |key: &TicketRef, may: bool, provider: Option<&str>| {
            TicketContext::prepare(&store, 5, key, may, provider).unwrap()
        };
        let first = prepare(&linked, true, None).expect("A new context needs the introduction");
        assert!(first.prepend("Do it".into()).ends_with("\n\nDo it"));
        // A failed or unbound turn leaves the session eligible.
        first.remember(&store, None).unwrap();
        assert!(prepare(&linked, true, Some("ctx-1")).is_some());
        first.remember(&store, Some("ctx-1")).unwrap();
        assert!(prepare(&linked, true, Some("ctx-1")).is_none());
        assert!(prepare(&linked, true, Some("ctx-2")).is_some());
        assert!(prepare(&linked, false, Some("ctx-1")).is_some());
        assert!(prepare(&ticket("ENG-8", None), true, Some("ctx-1")).is_some());
        assert!(
            TicketContext::prepare(&store, 6, &linked, true, Some("ctx-1"))
                .unwrap()
                .is_some()
        );
    }
}
