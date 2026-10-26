//! Where a linked card moves when its work starts, finishes or fails. The
//! rules and the engine live in `gespan-board-store` (`on_linked_work`), so
//! the native app and the Tauri app move cards the same way; by default a
//! card only moves forward (started to doing, completed to review or done).
use super::Service;

/// Something a card's linked work reported (`started`, `needs_input`,
/// `completed`, `failed`, `pr_opened`, `pr_merged`).
pub use gespan_board_store::LinkedEvent as LinkedWorkEvent;

/// Applies the boards' auto-move rules to every card linked to this work,
/// recording the event on each. One failing card does not stop the others;
/// the first error is returned. Returns the ids of the boards written.
pub fn on_linked_work(
    service: &mut Service,
    kind: &str,
    id: &str,
    event: LinkedWorkEvent,
) -> Result<Vec<String>, String> {
    service.linked_event(kind, id, event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Service;

    #[test]
    fn event_names_match_the_tauri_record_kinds() {
        assert_eq!(LinkedWorkEvent::Started.as_str(), "started");
        assert_eq!(LinkedWorkEvent::Completed.as_str(), "completed");
        assert_eq!(LinkedWorkEvent::Failed.as_str(), "failed");
    }

    fn service() -> (tempfile::TempDir, Service) {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::open(&dir.path().join("gespan.db")).unwrap();
        (dir, service)
    }

    /// The column a card ends up in, by name, and the activity kinds on it, newest first.
    fn where_is(service: &mut Service, board_id: &str, card_id: &str) -> (String, Vec<String>) {
        let detail = service.get(board_id).unwrap();
        let card = detail.cards.iter().find(|c| c.id == card_id).unwrap();
        let column = detail
            .columns
            .iter()
            .find(|c| c.id == card.column_id)
            .unwrap();
        let kinds = service
            .activity(board_id, Some(card_id), 20)
            .unwrap()
            .into_iter()
            .map(|row| row.kind)
            .collect();
        (column.name.clone(), kinds)
    }

    #[test]
    fn moves_each_linked_card_on_its_own_board_as_the_system_actor() {
        let (_dir, mut service) = service();
        let a = service.create("A").unwrap();
        let b = service.create("B").unwrap();
        let a_todo = service.get(&a.id).unwrap().columns[0].id.clone();
        let b_todo = service.get(&b.id).unwrap().columns[0].id.clone();
        let one = service.card_create(&a_todo, "One", "").unwrap();
        let two = service.card_create(&b_todo, "Two", "").unwrap();
        for card in [&one, &two] {
            service
                .card_link(&card.id, Some("session"), Some("s-1"))
                .unwrap();
        }
        on_linked_work(&mut service, "session", "s-1", LinkedWorkEvent::Started).unwrap();
        for (board, card) in [(&a, &one), (&b, &two)] {
            let (column, kinds) = where_is(&mut service, &board.id, &card.id);
            assert_eq!(column, "In Progress");
            assert_eq!(kinds[..2], ["started".to_string(), "moved".to_string()]);
            let moved = service
                .activity(&board.id, Some(&card.id), 20)
                .unwrap()
                .into_iter()
                .find(|row| row.kind == "moved")
                .unwrap();
            assert_eq!(moved.actor, "system");
        }
    }

    #[test]
    fn records_the_event_instead_of_moving_when_the_card_must_stay() {
        let (_dir, mut service) = service();
        let board = service.create("A").unwrap();
        let columns = service.get(&board.id).unwrap().columns;
        let card = service.card_create(&columns[3].id, "Shipped", "").unwrap();
        service
            .card_link(&card.id, Some("pilot"), Some("p-1"))
            .unwrap();
        on_linked_work(&mut service, "pilot", "p-1", LinkedWorkEvent::Started).unwrap();
        let (column, kinds) = where_is(&mut service, &board.id, &card.id);
        assert_eq!(column, "Done");
        assert_eq!(kinds.iter().filter(|k| *k == "started").count(), 1);
        assert!(!kinds.contains(&"moved".to_string()));
    }

    #[test]
    fn records_failed_runs_without_moving_the_card() {
        let (_dir, mut service) = service();
        let board = service.create("A").unwrap();
        let columns = service.get(&board.id).unwrap().columns;
        let card = service.card_create(&columns[1].id, "Work", "").unwrap();
        service
            .card_link(&card.id, Some("session"), Some("s-1"))
            .unwrap();
        on_linked_work(&mut service, "session", "s-1", LinkedWorkEvent::Failed).unwrap();
        let (column, kinds) = where_is(&mut service, &board.id, &card.id);
        assert_eq!(column, "In Progress");
        assert_eq!(kinds[0], "failed");
    }

    #[test]
    fn completed_work_lands_at_the_end_of_review() {
        let (_dir, mut service) = service();
        let board = service.create("A").unwrap();
        let columns = service.get(&board.id).unwrap().columns;
        service.card_create(&columns[2].id, "Already", "").unwrap();
        let card = service.card_create(&columns[1].id, "Work", "").unwrap();
        service
            .card_link(&card.id, Some("session"), Some("s-1"))
            .unwrap();
        on_linked_work(&mut service, "session", "s-1", LinkedWorkEvent::Completed).unwrap();
        let detail = service.get(&board.id).unwrap();
        let moved = detail.cards.iter().find(|c| c.id == card.id).unwrap();
        assert_eq!(moved.column_id, columns[2].id);
        assert_eq!(moved.position, 1);
    }

    #[test]
    fn follows_the_boards_auto_move_rules_set_through_the_service() {
        use crate::board::extras::SettingsPatch;
        let (_dir, mut service) = service();
        let board = service.create("A").unwrap();
        let columns = service.get(&board.id).unwrap().columns;
        let card = service.card_create(&columns[2].id, "Work", "").unwrap();
        service
            .card_link(&card.id, Some("session"), Some("s-1"))
            .unwrap();
        // Failed work goes back to the backlog on this board: a custom target
        // moves the card in either direction.
        let rules = format!(r#"{{"failed":{{"column":"{}"}}}}"#, columns[0].id);
        service
            .update_settings(
                &board.id,
                &SettingsPatch {
                    auto_move_json: Some(Some(rules)),
                    ..Default::default()
                },
            )
            .unwrap();
        let touched =
            on_linked_work(&mut service, "session", "s-1", LinkedWorkEvent::Failed).unwrap();
        assert_eq!(touched, vec![board.id.clone()]);
        let (column, kinds) = where_is(&mut service, &board.id, &card.id);
        assert_eq!(column, "Backlog");
        assert_eq!(kinds[..2], ["failed".to_string(), "moved".to_string()]);
        // PR events fire once per card.
        on_linked_work(&mut service, "session", "s-1", LinkedWorkEvent::PrOpened).unwrap();
        on_linked_work(&mut service, "session", "s-1", LinkedWorkEvent::PrOpened).unwrap();
        let (_, kinds) = where_is(&mut service, &board.id, &card.id);
        assert_eq!(kinds.iter().filter(|kind| *kind == "pr_opened").count(), 1);
    }

    #[test]
    fn does_nothing_for_a_link_with_no_cards() {
        let (_dir, mut service) = service();
        on_linked_work(&mut service, "session", "nobody", LinkedWorkEvent::Started).unwrap();
    }

    #[test]
    fn keeps_processing_other_cards_when_one_fails_then_reports_the_error() {
        let (_dir, mut service) = service();
        let board = service.create("A").unwrap();
        let columns = service.get(&board.id).unwrap().columns;
        let broken = service.card_create(&columns[0].id, "Broken", "").unwrap();
        let fine = service.card_create(&columns[0].id, "Fine", "").unwrap();
        for card in [&broken, &fine] {
            service
                .card_link(&card.id, Some("session"), Some("s-1"))
                .unwrap();
        }
        // Any activity write for the broken card aborts, so its move fails.
        service
            .conn
            .execute_batch(&format!(
                "CREATE TEMP TRIGGER refuse BEFORE INSERT ON board_activity \
                 WHEN NEW.card_id = '{}' BEGIN SELECT RAISE(ABORT, 'refused'); END;",
                broken.id
            ))
            .unwrap();
        let error =
            on_linked_work(&mut service, "session", "s-1", LinkedWorkEvent::Started).unwrap_err();
        assert!(error.contains("refused"), "{error}");
        assert_eq!(where_is(&mut service, &board.id, &fine.id).0, "In Progress");
        assert_eq!(where_is(&mut service, &board.id, &broken.id).0, "Backlog");
    }
}
