//! A card's ticket link: the chip on the card and in the drawer, "New card
//! from ticket…", and reading the ticket's title and status. Trackers are
//! only asked when a card is linked, when its drawer opens, and when the user
//! asks to refresh; never on a timer, never on the UI thread.
use super::BoardView;
use crate::{
    ticket_chip::{TicketChip, chip_element},
    tickets::TicketBackend,
};
use gespan_core::{
    board::Service,
    tickets::{TicketError, TicketRef},
};
use gpui::{AnyElement, Context, MouseButton, prelude::*};
use std::sync::Arc;

/// Opening a card again within this long of a read does not read it again.
const FRESH_MS: i64 = 60_000;

impl BoardView {
    /// The backend the ticket reads go through; the shell hands over its own.
    pub(crate) fn set_tickets(&mut self, backend: Arc<dyn TicketBackend>) {
        self.ticket_backend = Some(backend);
    }

    pub(crate) fn card_ticket(&self, card_id: &str) -> Option<&TicketRef> {
        self.card_tickets.get(card_id)
    }

    /// Whether the card's tracker is known to have no saved credentials.
    fn ticket_stale(&self, card_id: &str, ticket: &TicketRef) -> bool {
        self.stale_tickets.contains(card_id)
            || self
                .connected_trackers
                .as_ref()
                .is_some_and(|connected| !connected.contains(&ticket.tracker))
    }

    /// The chip for a card's ticket. `place` names it for ids and selectors.
    pub(super) fn ticket_chip(
        &self,
        place: String,
        card_id: &str,
        compact: bool,
        opens_card: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let ticket = self.card_tickets.get(card_id)?;
        let chip = TicketChip::new(ticket, self.ticket_stale(card_id, ticket));
        let url = ticket.url.clone();
        let id = card_id.to_string();
        Some(
            chip_element(&place, &chip, compact)
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    match &url {
                        Some(url) => cx.open_url(url),
                        // Nothing to open: on the card face, the chip opens the card.
                        None if opens_card => this.open_card_detail(id.clone(), window, cx),
                        None => {}
                    }
                }))
                .into_any_element(),
        )
    }

    /// Which trackers have saved credentials, read off the UI thread so the
    /// chip can flag a disconnected one without touching the keychain.
    pub(super) fn load_connected_trackers(&mut self, cx: &mut Context<Self>) {
        let Some(backend) = self.ticket_backend.clone() else {
            return;
        };
        let work = cx
            .background_executor()
            .spawn(async move { backend.connected() });
        cx.spawn(async move |this, cx| {
            let connected = work.await;
            let _ = this.update(cx, |this, cx| {
                this.connected_trackers = Some(connected.into_iter().collect());
                this.stale_tickets.clear();
                cx.notify();
            });
        })
        .detach();
    }

    /// Links the card to a ticket, replacing its current one, then reads the
    /// ticket's title and status.
    pub(crate) fn link_ticket(&mut self, card_id: &str, ticket: TicketRef, cx: &mut Context<Self>) {
        self.stale_tickets.remove(card_id);
        let id = card_id.to_string();
        if self.write(cx, |service| service.set_card_ticket(&id, Some(&ticket))) {
            self.load_connected_trackers(cx);
            self.refresh_card_ticket(card_id, true, false, cx);
        }
        self.refresh_detail_lists();
        cx.notify();
    }

    pub(crate) fn unlink_ticket(&mut self, card_id: &str, cx: &mut Context<Self>) {
        let id = card_id.to_string();
        self.ticket_requests.remove(card_id);
        self.stale_tickets.remove(card_id);
        self.write(cx, |service| service.set_card_ticket(&id, None));
        self.refresh_detail_lists();
        cx.notify();
    }

    /// "New card from ticket…": a card in the first To do step (else the first
    /// step), titled with the key until the ticket's title arrives. Returns
    /// the new card's id.
    pub(crate) fn create_card_from_ticket(
        &mut self,
        ticket: TicketRef,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let created = self
            .service()
            .and_then(|service| make_card(service, &ticket));
        let id = match created {
            Ok(id) => Some(id),
            Err(error) => {
                self.error = Some(error);
                None
            }
        };
        self.reload(cx);
        if let Some(id) = &id {
            self.load_connected_trackers(cx);
            self.refresh_card_ticket(id, true, false, cx);
        }
        id
    }

    /// Reads the card's ticket. A drawer opening passes `force` false and
    /// skips a ticket read within the last minute; Refresh forces the read.
    /// `report` shows a failure to the user.
    pub(crate) fn refresh_card_ticket(
        &mut self,
        card_id: &str,
        force: bool,
        report: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(backend) = self.ticket_backend.clone() else {
            return;
        };
        let Some(ticket) = self.card_tickets.get(card_id).cloned() else {
            return;
        };
        if !force {
            let now = gespan_core::time::now_ms() as i64;
            if ticket
                .fetched_at
                .is_some_and(|at| (0..FRESH_MS).contains(&(now - at)))
            {
                return;
            }
            let reading = self
                .ticket_requests
                .get(card_id)
                .is_some_and(|(_, tracker, key)| *tracker == ticket.tracker && *key == ticket.key);
            if reading {
                return;
            }
        }
        self.ticket_sequence += 1;
        let sequence = self.ticket_sequence;
        self.ticket_requests.insert(
            card_id.to_string(),
            (sequence, ticket.tracker, ticket.key.clone()),
        );
        let requested = ticket.clone();
        let work = cx
            .background_executor()
            .spawn(async move { backend.fetch(&requested) });
        let id = card_id.to_string();
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.card_ticket_read(&id, sequence, &ticket, result, report, cx)
            });
        })
        .detach();
    }

    fn card_ticket_read(
        &mut self,
        card_id: &str,
        sequence: u64,
        requested: &TicketRef,
        result: Result<TicketRef, TicketError>,
        report: bool,
        cx: &mut Context<Self>,
    ) {
        // A newer read, a relink or an unlink owns the outcome.
        if self
            .ticket_requests
            .get(card_id)
            .map(|(latest, ..)| *latest)
            != Some(sequence)
        {
            return;
        }
        self.ticket_requests.remove(card_id);
        let id = card_id.to_string();
        let current = self
            .service()
            .and_then(|service| service.card_ticket(&id))
            .ok()
            .flatten();
        if !current.is_some_and(|c| c.tracker == requested.tracker && c.key == requested.key) {
            return;
        }
        match result {
            Ok(fresh) => {
                self.stale_tickets.remove(card_id);
                let key = requested.key.clone();
                self.write(cx, |service| {
                    service.set_card_ticket(&id, Some(&fresh))?;
                    retitle_from_ticket(service, &id, &key, &fresh)
                });
                self.refresh_detail_lists();
            }
            Err(error) => {
                if matches!(error, TicketError::NotConnected(_)) {
                    self.stale_tickets.insert(card_id.to_string());
                }
                if report {
                    self.error = Some(error.to_string());
                } else {
                    tracing::warn!(card = card_id, %error, "ticket refresh failed");
                }
            }
        }
        cx.notify();
    }
}

fn make_card(service: &mut Service, ticket: &TicketRef) -> Result<String, String> {
    let board = service.default_board()?;
    let detail = service.get(&board.id)?;
    let column = detail
        .columns
        .iter()
        .find(|column| column.role.as_deref() == Some("todo"))
        .or(detail.columns.first())
        .ok_or("The board has no steps")?;
    let card = service.card_create(&column.id, &ticket.key, "")?;
    service.set_card_ticket(&card.id, Some(ticket))?;
    Ok(card.id)
}

/// The card is titled with the ticket's key until the title is known; a title
/// the user has since changed is left alone.
fn retitle_from_ticket(
    service: &mut Service,
    card_id: &str,
    key: &str,
    fresh: &TicketRef,
) -> Result<(), String> {
    let Some(title) = fresh
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return Ok(());
    };
    let card = service.card(card_id)?;
    if card.title == key {
        service.card_update(card_id, title, &card.body)?;
    }
    Ok(())
}
