//! Where a session an agent starts appears: in a tab of its own, or split
//! beside an open session. Pilots' `sessions.start` uses it.
use super::*;
use serde_json::Value;

pub(crate) type Placement = Option<(i64, Edge)>;

impl Shell {
    pub(crate) fn app_placement(
        &self,
        input: &Value,
        caller: i64,
        project: &str,
        cx: &App,
    ) -> Result<Placement, String> {
        let edge = match input["placement"].as_str().unwrap_or("tab") {
            "tab" => return Ok(None),
            "right" => Edge::Right,
            "down" => Edge::Bottom,
            _ => return Err("placement must be tab, right or down".into()),
        };
        let id = if input.get("besideSessionId").is_some() {
            session_id(input, "besideSessionId", &self.store)?
        } else {
            caller
        };
        if self.workspace.tab_of(id).is_none()
            || self.session_metadata(id, cx).summary.cwd != project
        {
            return Err("besideSessionId must be an open session pane in this project".into());
        }
        Ok(Some((id, edge)))
    }
    pub(crate) fn app_place_launch(
        &mut self,
        id: i64,
        placement: Placement,
        reveal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((target, edge)) =
            placement.filter(|(target, _)| self.workspace.tab_of(*target).is_some())
        {
            self.open(target, window, cx);
            let before = self.active_leaves();
            self.workspace.place(id, target, edge);
            self.begin_pane_motion(before, cx);
            self.fit_sidebar(cx);
            self.activate_session(id, window, cx);
        } else if reveal {
            self.open(id, window, cx);
        }
        self.persist(cx);
    }
}

fn session_id(input: &Value, key: &str, store: &Database) -> Result<i64, String> {
    match input.get(key) {
        Some(Value::String(id)) => locked(store)?.resolve_session_id(id),
        Some(value) => value.as_i64().ok_or_else(|| format!("Provide {key}")),
        None => Err(format!("Provide {key}")),
    }
}
