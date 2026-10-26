//! Where a linked card moves when its work reports an event, per the board's
//! auto-move rules (`board_settings.auto_move_json`), and applying that to
//! every card linked to the work.
//!
//! The rules are a JSON object keyed by event name (`started`,
//! `needs_input`, `completed`, `failed`, `pr_opened`, `pr_merged`), each
//! `"default"`, `"stay"` or `{"column": id}`. "Default" is the forward-only
//! rule both apps used before rules existed.
use crate::ops::NewActivity;
use crate::ops::{db, get_board, get_card, is_archived, log_activity, move_card_in, write_tx};
use crate::settings::board_settings;
use crate::Column;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Something a card's linked work reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkedEvent {
    Started,
    NeedsInput,
    Completed,
    Failed,
    PrOpened,
    PrMerged,
}

impl LinkedEvent {
    /// The rule key and the activity kind recorded for the event.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::NeedsInput => "needs_input",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::PrOpened => "pr_opened",
            Self::PrMerged => "pr_merged",
        }
    }

    /// PR events fire at most once per card.
    fn once_per_card(self) -> bool {
        matches!(self, Self::PrOpened | Self::PrMerged)
    }
}

/// What one rule does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleTarget {
    /// The forward-only rule: started goes to doing, completed to review (or
    /// done without a review column), anything else stays.
    Default,
    Stay,
    /// Move to this column, in either direction. A column that no longer
    /// exists on the card's board means stay.
    Column(String),
}

/// One rule per event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoMoveRules {
    pub started: RuleTarget,
    pub needs_input: RuleTarget,
    pub completed: RuleTarget,
    pub failed: RuleTarget,
    pub pr_opened: RuleTarget,
    pub pr_merged: RuleTarget,
}

impl Default for AutoMoveRules {
    fn default() -> Self {
        Self {
            started: RuleTarget::Default,
            needs_input: RuleTarget::Stay,
            completed: RuleTarget::Default,
            failed: RuleTarget::Stay,
            pr_opened: RuleTarget::Stay,
            pr_merged: RuleTarget::Stay,
        }
    }
}

impl AutoMoveRules {
    pub fn rule(&self, event: LinkedEvent) -> &RuleTarget {
        match event {
            LinkedEvent::Started => &self.started,
            LinkedEvent::NeedsInput => &self.needs_input,
            LinkedEvent::Completed => &self.completed,
            LinkedEvent::Failed => &self.failed,
            LinkedEvent::PrOpened => &self.pr_opened,
            LinkedEvent::PrMerged => &self.pr_merged,
        }
    }

    fn rule_mut(&mut self, event: LinkedEvent) -> &mut RuleTarget {
        match event {
            LinkedEvent::Started => &mut self.started,
            LinkedEvent::NeedsInput => &mut self.needs_input,
            LinkedEvent::Completed => &mut self.completed,
            LinkedEvent::Failed => &mut self.failed,
            LinkedEvent::PrOpened => &mut self.pr_opened,
            LinkedEvent::PrMerged => &mut self.pr_merged,
        }
    }
}

const EVENTS: [LinkedEvent; 6] = [
    LinkedEvent::Started,
    LinkedEvent::NeedsInput,
    LinkedEvent::Completed,
    LinkedEvent::Failed,
    LinkedEvent::PrOpened,
    LinkedEvent::PrMerged,
];

fn parse_rule(value: &Value) -> Option<RuleTarget> {
    match value {
        Value::String(word) if word == "default" => Some(RuleTarget::Default),
        Value::String(word) if word == "stay" => Some(RuleTarget::Stay),
        Value::Object(map) if map.len() == 1 => map
            .get("column")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(|id| RuleTarget::Column(id.to_string())),
        _ => None,
    }
}

/// The board's auto-move rules. Missing or invalid JSON (or JSON that is not
/// an object) reads as the defaults; a rule that can't be read reads as that
/// event's default; unknown keys are ignored.
pub fn auto_move_rules(settings: &crate::BoardSettings) -> AutoMoveRules {
    let mut rules = AutoMoveRules::default();
    let Some(Value::Object(map)) = settings
        .auto_move_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
    else {
        return rules;
    };
    for event in EVENTS {
        if let Some(rule) = map.get(event.as_str()).and_then(parse_rule) {
            *rules.rule_mut(event) = rule;
        }
    }
    rules
}

/// The first column with `role` strictly right of `current`.
fn forward_column<'a>(columns: &'a [Column], current: &Column, role: &str) -> Option<&'a Column> {
    columns
        .iter()
        .filter(|column| column.role.as_deref() == Some(role) && column.position > current.position)
        .min_by_key(|column| column.position)
}

fn default_target<'a>(
    columns: &'a [Column],
    current: &Column,
    event: LinkedEvent,
) -> Option<&'a Column> {
    match event {
        LinkedEvent::Started => forward_column(columns, current, "doing"),
        LinkedEvent::Completed => {
            let has_review = columns
                .iter()
                .any(|column| column.role.as_deref() == Some("review"));
            forward_column(columns, current, if has_review { "review" } else { "done" })
        }
        _ => None,
    }
}

/// Where a card in `current` goes when its linked work reports `event`, or
/// `None` to stay. `columns` are the card's board's columns.
pub fn auto_move_target<'a>(
    columns: &'a [Column],
    current: &Column,
    event: LinkedEvent,
    rules: &AutoMoveRules,
) -> Option<&'a Column> {
    match rules.rule(event) {
        RuleTarget::Default => default_target(columns, current, event),
        RuleTarget::Stay => None,
        RuleTarget::Column(id) => columns
            .iter()
            .find(|column| &column.id == id && column.id != current.id),
    }
}

/// Applies the board's rule to one card and records the event, all in one
/// write transaction. Returns whether it wrote anything.
fn apply(conn: &Connection, card_id: &str, event: LinkedEvent) -> Result<Option<String>, String> {
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    if is_archived(&tx, card_id)? {
        return Ok(None);
    }
    if event.once_per_card() {
        let fired: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM board_activity WHERE card_id = ?1 AND kind = ?2)",
                [card_id, event.as_str()],
                |r| r.get(0),
            )
            .map_err(db)?;
        if fired {
            return Ok(None);
        }
    }
    let rules = auto_move_rules(&board_settings(&tx, &card.board_id)?);
    let columns = get_board(&tx, &card.board_id)?.columns;
    let target = columns
        .iter()
        .find(|column| column.id == card.column_id)
        .and_then(|current| auto_move_target(&columns, current, event, &rules))
        .map(|column| column.id.clone());
    if let Some(target) = target {
        move_card_in(&tx, card_id, &target, usize::MAX, "system")?;
    }
    log_activity(
        &tx,
        NewActivity {
            board_id: &card.board_id,
            card_id: Some(card_id),
            kind: event.as_str(),
            actor: "system",
            from_column: None,
            to_column: None,
            detail: None,
        },
    )?;
    tx.commit().map_err(db)?;
    Ok(Some(card.board_id))
}

/// Applies the auto-move rules to every live card linked to `kind`/`reference`:
/// each card moves (as `system`, to the end of its target column) and records
/// the event in one transaction, so a card never moves without the record.
/// `pr_opened` and `pr_merged` skip a card that already has a row of that
/// kind. One failing card does not stop the others; the first error is
/// returned after every card was tried. Returns the ids of the boards written,
/// in first-seen order.
pub fn on_linked_work(
    conn: &Connection,
    kind: &str,
    reference: &str,
    event: LinkedEvent,
) -> Result<Vec<String>, String> {
    let cards = crate::ops::cards_for_link(conn, kind, reference)?;
    let mut touched: Vec<String> = Vec::new();
    let mut first_error = None;
    for card in cards {
        match apply(conn, &card.id, event) {
            Ok(Some(board_id)) => {
                if !touched.contains(&board_id) {
                    touched.push(board_id);
                }
            }
            Ok(None) => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    first_error.map_or(Ok(touched), Err)
}
