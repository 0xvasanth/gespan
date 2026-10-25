//! What the shell asks of the ticket trackers. A trait so gpui tests can swap
//! in a fake without exposing gespan-core's provider internals.
use gespan_core::tickets::{Resolved, TicketError, TicketRef, Tickets};

pub(crate) trait TicketBackend: Send + Sync {
    /// Never touches the network.
    fn resolve(&self, input: &str) -> Resolved;
    fn fetch(&self, ticket: &TicketRef) -> Result<TicketRef, TicketError>;
}

impl TicketBackend for Tickets {
    fn resolve(&self, input: &str) -> Resolved {
        Tickets::resolve(self, input)
    }

    fn fetch(&self, ticket: &TicketRef) -> Result<TicketRef, TicketError> {
        Tickets::fetch(self, ticket)
    }
}

#[cfg(test)]
pub(crate) mod fakes {
    use super::*;
    use gespan_core::{
        provider_boards::StateGroup,
        tickets::{Tracker, parse_ticket},
    };
    use std::sync::Mutex;

    /// Resolves like gespan-core's `Tickets` (which has its own tests) and
    /// answers every fetch from memory, so no test reaches a tracker.
    pub(crate) struct FakeTickets {
        pub(crate) connected: Mutex<Vec<Tracker>>,
        pub(crate) fetches: Mutex<Vec<String>>,
        pub(crate) offline: Mutex<bool>,
    }

    impl FakeTickets {
        pub(crate) fn new(connected: &[Tracker]) -> Self {
            Self {
                connected: Mutex::new(connected.to_vec()),
                fetches: Mutex::new(Vec::new()),
                offline: Mutex::new(false),
            }
        }

        pub(crate) fn disconnect(&self) {
            self.connected.lock().unwrap().clear();
        }

        pub(crate) fn fetch_count(&self) -> usize {
            self.fetches.lock().unwrap().len()
        }
    }

    impl TicketBackend for FakeTickets {
        fn resolve(&self, input: &str) -> Resolved {
            let Some(parsed) = parse_ticket(input) else {
                return Resolved::Invalid;
            };
            let connected = self.connected.lock().unwrap().clone();
            let ticket = |tracker, url| {
                Resolved::One(TicketRef {
                    tracker,
                    key: parsed.key.clone(),
                    url,
                    title: None,
                    status: None,
                    status_group: None,
                    fetched_at: None,
                })
            };
            match (parsed.tracker, connected.as_slice()) {
                (Some(tracker), _) if connected.contains(&tracker) => {
                    ticket(tracker, Some(input.trim().to_string()))
                }
                (Some(tracker), _) => Resolved::NotConnected(tracker),
                (None, []) => Resolved::Invalid,
                (None, [only]) => ticket(*only, None),
                (None, _) => Resolved::Ambiguous(connected),
            }
        }

        fn fetch(&self, ticket: &TicketRef) -> Result<TicketRef, TicketError> {
            self.fetches.lock().unwrap().push(ticket.key.clone());
            if !self.connected.lock().unwrap().contains(&ticket.tracker) {
                return Err(TicketError::NotConnected(ticket.tracker));
            }
            if *self.offline.lock().unwrap() {
                return Err(TicketError::Remote("Could not reach the tracker".into()));
            }
            Ok(TicketRef {
                url: Some(format!("https://tracker.test/{}", ticket.key)),
                title: Some("Fix login".into()),
                status: Some("In Progress".into()),
                status_group: Some(StateGroup::Started),
                fetched_at: Some(1_700_000_000_000),
                ..ticket.clone()
            })
        }
    }
}
