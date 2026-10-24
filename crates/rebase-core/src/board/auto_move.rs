//! Where a linked card moves when its work starts, finishes or fails.
//! Mirrors `src/features/board/model/autoMove.ts`.
use super::{Column, Service};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkedWorkEvent {
    Started,
    Completed,
    Failed,
}

impl LinkedWorkEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// The first column with `role` strictly right of `current`.
fn forward_column<'a>(columns: &'a [Column], current: &Column, role: &str) -> Option<&'a Column> {
    columns
        .iter()
        .filter(|column| column.role.as_deref() == Some(role) && column.position > current.position)
        .min_by_key(|column| column.position)
}

/// Where a card should go when its linked work reports `event`, or `None` to
/// stay. Cards only move right: started goes to doing, completed to review (or
/// done when the board has no review column). Failed work never moves a card.
pub fn auto_move_target<'a>(
    columns: &'a [Column],
    current: &Column,
    event: LinkedWorkEvent,
) -> Option<&'a Column> {
    match event {
        LinkedWorkEvent::Started => forward_column(columns, current, "doing"),
        LinkedWorkEvent::Completed => {
            let has_review = columns.iter().any(|c| c.role.as_deref() == Some("review"));
            forward_column(columns, current, if has_review { "review" } else { "done" })
        }
        LinkedWorkEvent::Failed => None,
    }
}

/// Applies the rule to every card linked to this work, recording the event on
/// each (after the move's own "moved" row, or alone when the card stays). One
/// failing card does not stop the others; the first error is returned.
pub fn on_linked_work(
    service: &mut Service,
    kind: &str,
    id: &str,
    event: LinkedWorkEvent,
) -> Result<(), String> {
    let cards = service.cards_for_link(kind, id)?;
    let mut columns_by_board: HashMap<String, Vec<Column>> = HashMap::new();
    let mut first_error = None;
    for card in cards {
        let result = (|| {
            if !columns_by_board.contains_key(&card.board_id) {
                let columns = service.get(&card.board_id)?.columns;
                columns_by_board.insert(card.board_id.clone(), columns);
            }
            let columns = &columns_by_board[&card.board_id];
            let target = columns
                .iter()
                .find(|column| column.id == card.column_id)
                .and_then(|current| auto_move_target(columns, current, event))
                .map(|column| column.id.clone());
            if let Some(target) = target {
                service.card_move(&card.id, &target, usize::MAX, "system")?;
            }
            service.card_record(&card.id, event.as_str(), None)
        })();
        if let Err(error) = result {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Column, Service};

    fn column(id: &str, position: i64, role: Option<&str>) -> Column {
        Column {
            id: id.into(),
            board_id: "b".into(),
            name: id.into(),
            position,
            role: role.map(Into::into),
        }
    }

    fn roles() -> Vec<Column> {
        vec![
            column("todo", 0, Some("todo")),
            column("doing", 1, Some("doing")),
            column("review", 2, Some("review")),
            column("done", 3, Some("done")),
        ]
    }

    fn target<'a>(columns: &'a [Column], from: &str, event: LinkedWorkEvent) -> Option<&'a str> {
        let current = columns.iter().find(|c| c.id == from).unwrap();
        auto_move_target(columns, current, event).map(|c| c.id.as_str())
    }

    #[test]
    fn event_names_match_the_tauri_record_kinds() {
        assert_eq!(LinkedWorkEvent::Started.as_str(), "started");
        assert_eq!(LinkedWorkEvent::Completed.as_str(), "completed");
        assert_eq!(LinkedWorkEvent::Failed.as_str(), "failed");
    }

    #[test]
    fn moves_started_work_from_todo_to_doing() {
        assert_eq!(
            target(&roles(), "todo", LinkedWorkEvent::Started),
            Some("doing")
        );
    }

    #[test]
    fn never_moves_started_work_backward() {
        assert_eq!(target(&roles(), "review", LinkedWorkEvent::Started), None);
        assert_eq!(target(&roles(), "done", LinkedWorkEvent::Started), None);
    }

    #[test]
    fn stays_put_when_started_work_is_already_in_doing() {
        assert_eq!(target(&roles(), "doing", LinkedWorkEvent::Started), None);
    }

    #[test]
    fn moves_completed_work_to_review_from_doing_or_todo() {
        assert_eq!(
            target(&roles(), "doing", LinkedWorkEvent::Completed),
            Some("review")
        );
        assert_eq!(
            target(&roles(), "todo", LinkedWorkEvent::Completed),
            Some("review")
        );
    }

    #[test]
    fn falls_back_to_done_when_there_is_no_review_column() {
        let columns: Vec<Column> = roles().into_iter().filter(|c| c.id != "review").collect();
        assert_eq!(
            target(&columns, "doing", LinkedWorkEvent::Completed),
            Some("done")
        );
    }

    #[test]
    fn does_not_move_completed_work_out_of_done_or_review() {
        assert_eq!(target(&roles(), "review", LinkedWorkEvent::Completed), None);
        assert_eq!(target(&roles(), "done", LinkedWorkEvent::Completed), None);
    }

    #[test]
    fn never_moves_a_failed_run() {
        for from in ["todo", "doing", "review", "done"] {
            assert_eq!(target(&roles(), from, LinkedWorkEvent::Failed), None);
        }
    }

    #[test]
    fn stays_put_on_started_when_the_board_has_no_doing_column() {
        let columns: Vec<Column> = roles().into_iter().filter(|c| c.id != "doing").collect();
        assert_eq!(target(&columns, "todo", LinkedWorkEvent::Started), None);
    }

    #[test]
    fn stays_put_on_completed_when_the_board_has_neither_review_nor_done() {
        let columns = vec![
            column("todo", 0, Some("todo")),
            column("doing", 1, Some("doing")),
        ];
        assert_eq!(target(&columns, "doing", LinkedWorkEvent::Completed), None);
    }

    #[test]
    fn handles_columns_passed_out_of_position_order() {
        let mut columns = roles();
        columns.reverse();
        assert_eq!(
            target(&columns, "todo", LinkedWorkEvent::Started),
            Some("doing")
        );
        assert_eq!(
            target(&columns, "doing", LinkedWorkEvent::Completed),
            Some("review")
        );
    }

    #[test]
    fn a_user_column_between_doing_and_review_moves_completed_work_to_review_only() {
        let columns = vec![
            column("todo", 0, Some("todo")),
            column("doing", 1, Some("doing")),
            column("qa", 2, None),
            column("review", 3, Some("review")),
            column("done", 4, Some("done")),
        ];
        assert_eq!(
            target(&columns, "qa", LinkedWorkEvent::Completed),
            Some("review")
        );
        assert_eq!(target(&columns, "qa", LinkedWorkEvent::Started), None);
    }

    #[test]
    fn treats_a_role_less_column_left_of_doing_as_before_it() {
        let columns = vec![
            column("inbox", 0, None),
            column("doing", 1, Some("doing")),
            column("done", 2, Some("done")),
        ];
        assert_eq!(
            target(&columns, "inbox", LinkedWorkEvent::Started),
            Some("doing")
        );
    }

    fn service() -> (tempfile::TempDir, Service) {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::open(&dir.path().join("rebase.db")).unwrap();
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
