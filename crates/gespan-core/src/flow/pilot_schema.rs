//! The Flow format as a Pilot writes it through `flows.save` and
//! `flows.editRun`: its JSON Schema, a Flow that saves as written, the
//! defaults a Pilot may leave out, and one report of every problem in a draft.
//! A Pilot's SOPs, shown on its SOP Roadmap tab, are these Flows.
use super::definition::{
    unknown_fields, validate_flow, Flow, ACCESS_MODES, DEFAULT_MAX_VISITS, FAILED, MAX_STATES,
    MAX_VISITS, MIN_VISITS, SUCCEEDED,
};
use crate::provider::Kind;
use serde_json::{json, Map, Value};

const NAME_PATTERN: &str = "^[a-z][a-z0-9_]{0,39}$";

/// The provider ids a state's `harness` may name.
pub fn harness_ids() -> Vec<&'static str> {
    Kind::ALL.iter().map(|kind| kind.id()).collect()
}

/// A JSON Schema of the `flow` field. It lists what `flows.read` returns, so a
/// Flow read back can be edited and saved again.
pub fn schema() -> Value {
    let name = |what: &str| json!({"type":"string","pattern":NAME_PATTERN,"description":what});
    let guard = json!({"oneOf":[
        {"type":"object","properties":{
            "kind":{"const":"status"},
            "is":{"enum":["completed","failed"]},
        },"required":["kind","is"],"additionalProperties":false,
         "description":"Matches how the step ended. Only a `failed` status guard is followed when a step fails."},
        {"type":"object","properties":{
            "kind":{"const":"output"},
            "field":{"type":"string","description":"One of this state's outputs"},
            "op":{"enum":["eq","neq","truthy","falsy","contains"]},
            "value":{"type":["string","number","boolean"],"description":"Needed by eq, neq and contains"},
        },"required":["kind","field","op"],"additionalProperties":false,
         "description":"Matches one of this step's outputs after it completed."},
    ]});
    let transitions = json!({"type":"array","items":{
        "type":"object",
        "properties":{
            "to":{"type":"string","description":format!("A state id, {SUCCEEDED} or {FAILED}")},
            "when":guard,
        },
        "required":["to"],"additionalProperties":false,
    },"description":"Checked in order; the first that matches is taken. One without `when` matches any completed step. A parallel step's branch has none: its parallel step's transitions follow it."});
    let state = json!({
        "type":"object",
        "description":"One agent step. A Pilot's Flow has agent and parallel steps only.",
        "properties":{
            "kind":{"const":"agent","default":"agent"},
            "id":name("Unique state id, like `build`"),
            "title":{"type":"string","description":"Shown on the SOP Roadmap"},
            "harness":{"enum":harness_ids(),"description":"The agent that runs the step"},
            "model":{"type":"string","description":"A model id that models.list gives for this harness"},
            "brief":{"type":"string","description":"The step's instructions. May use {{inputs.NAME}} and {{states.ID.outputs.FIELD}}; add ? inside the braces, as in {{states.ID.outputs.FIELD?}}, when the value may not exist yet."},
            "outputs":{"type":"array","default":[],"items":{
                "type":"object",
                "properties":{
                    "name":name("Output name"),
                    "type":{"enum":["string","number","boolean","string[]"]},
                    "description":{"type":"string"},
                },
                "required":["name","type","description"],"additionalProperties":false,
            },"description":"Values the step must report, for later briefs and guards"},
            "transitions":transitions.clone(),
            "maxVisits":{"type":"integer","minimum":MIN_VISITS,"maximum":MAX_VISITS,"default":DEFAULT_MAX_VISITS,"description":"On a parallel step's branch, a failed branch runs again, told what failed, until it has run this many times"},
            "when":{"type":"object","properties":{
                "kind":{"const":"output"},
                "field":{"type":"string","description":"An output of every step that leads to the parallel step"},
                "op":{"enum":["eq","neq","truthy","falsy","contains"]},
                "value":{"type":["string","number","boolean"]},
            },"required":["kind","field","op"],"additionalProperties":false,
             "description":"Only on a parallel step's branch: the branch runs only when this matches the outputs of the step that led to the parallel step, and is skipped otherwise."},
        },
        "required":["id","title","harness","model","brief","transitions"],
        "additionalProperties":false,
    });
    let parallel = json!({
        "type":"object",
        "description":"Runs its branches side by side, then follows its transitions once every branch that ran has completed (a `completed` status), or once one has failed past its maxVisits (`failed`; the others are stopped).",
        "properties":{
            "kind":{"const":"parallel"},
            "id":name("Unique state id, like `build`"),
            "title":{"type":"string","description":"Names the step in Flow edits"},
            "branches":{"type":"array","minItems":2,"items":{"type":"string"},"description":"Ids of agent steps run side by side. Each belongs to this step alone, has no transitions, and nothing else leads to it."},
            "join":{"const":"all","default":"all","description":"Waits for every branch"},
            "transitions":transitions,
            "maxVisits":{"type":"integer","minimum":MIN_VISITS,"maximum":MAX_VISITS,"default":DEFAULT_MAX_VISITS},
        },
        "required":["kind","id","title","branches","transitions"],
        "additionalProperties":false,
    });
    json!({
        "type":"object",
        "description":format!("A Flow: one of your SOPs on the SOP Roadmap. A run starts at `initial` and follows transitions until {SUCCEEDED} or {FAILED}."),
        "properties":{
            "id":{"type":"string","description":"Leave out to create a Flow. To change one, pass its id from flows.list and the whole Flow, as flows.read returns it."},
            "name":{"type":"string"},
            "description":{"type":"string","default":""},
            "cwd":{"type":"string","description":"An assigned project's absolute path. Every step runs there."},
            "access":{"enum":ACCESS_MODES,"default":"auto"},
            "inputs":{"type":"array","default":[],"items":{
                "type":"object",
                "properties":{
                    "name":name("Input name, used as {{inputs.NAME}}"),
                    "label":{"type":"string"},
                    "required":{"type":"boolean"},
                    "default":{"type":"string"},
                },
                "required":["name","label","required"],"additionalProperties":false,
            },"description":"Values flows.start passes in `inputs`"},
            "initial":{"type":"string","description":"The first state's id"},
            "states":{"type":"array","minItems":1,"maxItems":MAX_STATES,"items":{"oneOf":[state, parallel]}},
            "owner":{"type":"string","description":"Set by Gespan"},
            "createdAt":{"type":"integer","description":"Set by Gespan"},
            "updatedAt":{"type":"integer","description":"Set by Gespan"},
        },
        "required":["name","cwd","initial","states"],
        "additionalProperties":false,
    })
}

/// A two-step Flow in `cwd` that saves as written.
pub fn example(cwd: &str) -> Value {
    json!({
        "name":"Release checklist",
        "description":"Build and test a version, then write its release notes.",
        "cwd":cwd,
        "inputs":[{"name":"version","label":"Version to release","required":true}],
        "initial":"build",
        "states":[
            {"kind":"agent","id":"build","title":"Build and test","harness":"claude","model":"opus",
             "brief":"Build version {{inputs.version}} and run the tests.",
             "outputs":[{"name":"passed","type":"boolean","description":"Whether every test passed"}],
             "transitions":[
                {"to":"notes","when":{"kind":"output","field":"passed","op":"truthy"}},
                {"to":FAILED},
             ],
             "maxVisits":2},
            {"kind":"agent","id":"notes","title":"Write release notes","harness":"claude","model":"opus",
             "brief":"Write the release notes for {{inputs.version}}.",
             "outputs":[],"transitions":[{"to":SUCCEEDED}],"maxVisits":1},
        ],
    })
}

/// Fills in what a Pilot may leave out. Gespan sets `owner` and the
/// timestamps; an empty `id` gets a new one when saved.
fn fill_defaults(flow: &mut Map<String, Value>, owner: &str) {
    flow.insert("owner".into(), json!(owner));
    flow.entry("id").or_insert(json!(""));
    flow.entry("description").or_insert(json!(""));
    flow.entry("inputs").or_insert(json!([]));
    for key in ["createdAt", "updatedAt"] {
        flow.entry(key).or_insert(json!(0));
    }
    for state in flow
        .get_mut("states")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object_mut)
    {
        state.entry("kind").or_insert(json!("agent"));
        if state["kind"] == "agent" || state["kind"] == "parallel" {
            state
                .entry("maxVisits")
                .or_insert(json!(DEFAULT_MAX_VISITS));
        }
        if state["kind"] == "agent" {
            state.entry("outputs").or_insert(json!([]));
        }
    }
}

/// One error per required field missing anywhere in the Flow, and per list or
/// object given as something else, named by where it is.
fn missing_fields(flow: &Map<String, Value>) -> Vec<String> {
    let mut errors = Vec::new();
    let mut need = |item: &Map<String, Value>, keys: &[&str], place: &str| {
        for key in keys.iter().filter(|key| !item.contains_key(**key)) {
            errors.push(format!("{place} is missing \"{key}\""));
        }
    };
    need(flow, &["name", "cwd", "initial", "states"], "the Flow");
    let mut shape = Vec::new();
    /// The objects in a list, noting in `shape` what isn't one.
    fn items(
        value: Option<&Value>,
        place: &str,
        shape: &mut Vec<String>,
    ) -> Vec<Map<String, Value>> {
        match value {
            None => Vec::new(),
            Some(Value::Array(list)) => list
                .iter()
                .enumerate()
                .filter_map(|(index, item)| match item {
                    Value::Object(item) => Some(item.clone()),
                    _ => {
                        shape.push(format!("{place} item {} must be an object", index + 1));
                        None
                    }
                })
                .collect(),
            Some(_) => {
                shape.push(format!("{place} must be a list"));
                Vec::new()
            }
        }
    }
    let inputs = items(flow.get("inputs"), "inputs", &mut shape);
    let states = items(flow.get("states"), "states", &mut shape);
    let label = |item: &Map<String, Value>, key: &str, index: usize| match item
        .get(key)
        .and_then(Value::as_str)
    {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => (index + 1).to_string(),
    };
    let mut nested = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        let place = format!("input {}", label(input, "name", index));
        nested.push((input.clone(), vec!["name", "label", "required"], place));
    }
    for (index, state) in states.iter().enumerate() {
        let id = label(state, "id", index);
        let place = format!("state {id}");
        match state.get("kind").and_then(Value::as_str) {
            Some("approval") => {
                nested.push((
                    state.clone(),
                    vec!["id", "title", "message", "transitions"],
                    place,
                ));
                continue;
            }
            Some("parallel") => {
                nested.push((
                    state.clone(),
                    vec!["id", "title", "branches", "transitions"],
                    place,
                ));
                continue;
            }
            _ => {}
        }
        nested.push((
            state.clone(),
            vec!["id", "title", "harness", "model", "brief", "transitions"],
            place.clone(),
        ));
        for (index, output) in items(
            state.get("outputs"),
            &format!("{place} outputs"),
            &mut shape,
        )
        .iter()
        .enumerate()
        {
            let place = format!("{place} output {}", label(output, "name", index));
            nested.push((output.clone(), vec!["name", "type", "description"], place));
        }
        for (index, transition) in items(
            state.get("transitions"),
            &format!("{place} transitions"),
            &mut shape,
        )
        .iter()
        .enumerate()
        {
            let place = format!("{place} transition {}", index + 1);
            nested.push((transition.clone(), vec!["to"], place.clone()));
            match transition.get("when") {
                None | Some(Value::Null) => {}
                Some(Value::Object(guard)) => {
                    let keys = match guard.get("kind").and_then(Value::as_str) {
                        Some("output") => vec!["field", "op"],
                        Some("status") => vec!["is"],
                        _ => {
                            shape.push(format!(
                                "{place} guard needs \"kind\": \"status\" or \"output\""
                            ));
                            continue;
                        }
                    };
                    nested.push((guard.clone(), keys, format!("{place} guard")));
                }
                Some(_) => shape.push(format!("{place} \"when\" must be an object")),
            }
        }
    }
    for (item, keys, place) in &nested {
        need(item, keys, place);
    }
    errors.extend(shape);
    errors
}

/// The Flow a Pilot sent, owned by `owner` whatever it claimed, or every
/// problem found in it: misspelled and missing fields first, then any value
/// of the wrong type, then the Flow's own rules.
pub fn pilot_flow(flow: &Value, owner: &str) -> Result<Flow, Vec<String>> {
    let Some(fields) = flow.as_object() else {
        return Err(vec!["flow must be a Flow object".into()]);
    };
    let mut fields = fields.clone();
    fill_defaults(&mut fields, owner);
    let mut errors = missing_fields(&fields);
    let filled = Value::Object(fields);
    errors.extend(unknown_fields(&filled));
    if !errors.is_empty() {
        return Err(errors);
    }
    let flow: Flow = serde_path_to_error::deserialize(filled).map_err(|error| {
        let path = error.path().to_string();
        let path = path.trim_start_matches('.');
        vec![format!("{}: {}", path, error.into_inner())]
    })?;
    let errors = validate_flow(&flow).errors;
    if errors.is_empty() {
        Ok(flow)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problems(flow: Value) -> Vec<String> {
        pilot_flow(&flow, "p1").unwrap_err()
    }

    /// One agent state that saves, for tests to break one field at a time.
    fn state() -> Value {
        json!({"id":"build","title":"Build","harness":"claude","model":"opus","brief":"Build it","transitions":[{"to":SUCCEEDED}]})
    }

    #[test]
    fn the_example_saves_as_written_for_its_project() {
        let flow = pilot_flow(&example("/work/app"), "p1").unwrap();
        assert_eq!(flow.cwd, "/work/app");
        assert_eq!(flow.owner.as_deref(), Some("p1"));
        assert_eq!(flow.states.len(), 2);
        assert!(validate_flow(&flow).warnings.is_empty());
    }

    #[test]
    fn a_pilot_may_leave_out_what_has_a_default() {
        let flow = pilot_flow(
            &json!({"name":"Ship","cwd":"/work","initial":"build","states":[state()]}),
            "p1",
        )
        .unwrap();
        assert_eq!((flow.id.as_str(), flow.description.as_str()), ("", ""));
        assert!(flow.inputs.is_empty());
        let crate::flow::FlowState::Agent(agent) = &flow.states[0] else {
            panic!("a state with no kind is an agent state");
        };
        assert_eq!(agent.max_visits, DEFAULT_MAX_VISITS);
        assert!(agent.outputs.is_empty());
    }

    #[test]
    fn the_owner_is_the_caller_whatever_the_flow_claims() {
        let mut flow = example("/work");
        flow["owner"] = json!("someone-else");
        assert_eq!(
            pilot_flow(&flow, "p1").unwrap().owner.as_deref(),
            Some("p1")
        );
    }

    /// The guesses a model makes from `flows.save {flow}` alone: each used to
    /// fail with only its first missing field, one retry at a time.
    #[test]
    fn a_guessed_sop_gets_every_problem_in_one_reply() {
        assert_eq!(
            problems(json!({"name":"Release SOP","steps":["Build","Test"]})),
            [
                "the Flow is missing \"cwd\"",
                "the Flow is missing \"initial\"",
                "the Flow is missing \"states\"",
                "Unknown field \"steps\" in the Flow",
            ]
        );
        assert_eq!(
            problems(
                json!({"name":"SOP","cwd":"/work","initial":"build","states":[
                    {"id":"build","title":"Build","brief":"Build it","next":SUCCEEDED,"max_visits":3},
                    {"title":"Test"},
                ]})
            ),
            [
                "state build is missing \"harness\"",
                "state build is missing \"model\"",
                "state build is missing \"transitions\"",
                "state 2 is missing \"id\"",
                "state 2 is missing \"harness\"",
                "state 2 is missing \"model\"",
                "state 2 is missing \"brief\"",
                "state 2 is missing \"transitions\"",
                "Unknown field \"max_visits\" in state build",
                "Unknown field \"next\" in state build",
            ]
        );
    }

    #[test]
    fn nested_parts_are_named_by_where_they_are() {
        let mut bad = state();
        bad["outputs"] = json!([{"name":"ok"}]);
        bad["transitions"] = json!([{"when":{"kind":"output"}}, {"to":"x","when":{"field":"ok"}}, {"to":"y","when":"ok"}]);
        assert_eq!(
            problems(
                json!({"name":"S","cwd":"/w","initial":"build","inputs":[{"name":"v"}],"states":[bad]})
            ),
            [
                "input v is missing \"label\"",
                "input v is missing \"required\"",
                "state build output ok is missing \"type\"",
                "state build output ok is missing \"description\"",
                "state build transition 1 is missing \"to\"",
                "state build transition 1 guard is missing \"field\"",
                "state build transition 1 guard is missing \"op\"",
                "state build transition 2 guard needs \"kind\": \"status\" or \"output\"",
                "state build transition 3 \"when\" must be an object",
                "Unknown field \"field\" in state build transition 2 guard",
            ]
        );
    }

    #[test]
    fn lists_given_as_something_else_are_named() {
        assert_eq!(
            problems(json!({"name":"S","cwd":"/w","initial":"a","states":"a then b","inputs":[1]})),
            ["inputs item 1 must be an object", "states must be a list"]
        );
        assert_eq!(problems(json!("a flow")), ["flow must be a Flow object"]);
    }

    #[test]
    fn a_wrong_type_names_its_path() {
        let mut bad = state();
        bad["maxVisits"] = json!("three");
        let errors = problems(json!({"name":"S","cwd":"/w","initial":"build","states":[bad]}));
        assert_eq!(errors.len(), 1, "{errors:?}");
        // A state's kind tag hides the field, but the path still names the state.
        assert!(
            errors[0].starts_with("states[0]: invalid type: string \"three\""),
            "{errors:?}"
        );
    }

    #[test]
    fn the_flows_own_rules_come_back_together() {
        let mut bad = state();
        bad["id"] = json!("Build");
        bad["harness"] = json!("Claude Code");
        bad["transitions"] = json!([{"to":"done"}]);
        let errors = problems(json!({"name":"S","cwd":"/w","initial":"build","states":[bad]}));
        for expected in [
            "State \"Build\": invalid name",
            "State \"Build\": unknown harness \"Claude Code\"",
            "initial state \"build\" is not a state",
            "State \"Build\": unknown target \"done\"",
        ] {
            assert!(
                errors.iter().any(|e| e == expected),
                "{expected} in {errors:?}"
            );
        }
    }

    #[test]
    fn a_pilot_flow_cannot_hold_an_approval_state() {
        let approval = json!({"kind":"approval","id":"ok","title":"OK?","message":"Approve","transitions":[{"to":SUCCEEDED}],"maxVisits":1});
        let errors = problems(json!({"name":"S","cwd":"/w","initial":"ok","states":[approval]}));
        assert_eq!(
            errors,
            ["State \"ok\": approval states are not allowed in a Pilot Flow"]
        );
    }

    #[test]
    fn what_flows_read_returns_saves_again() {
        let saved = pilot_flow(&example("/w"), "p1").unwrap();
        let read = serde_json::to_value(&saved).unwrap();
        assert_eq!(pilot_flow(&read, "p1").unwrap(), saved);
    }

    #[test]
    fn the_schema_requires_what_a_save_requires() {
        let schema = schema();
        assert_eq!(
            schema["required"],
            json!(["name", "cwd", "initial", "states"])
        );
        let state = &schema["properties"]["states"]["items"]["oneOf"][0];
        let parallel = &schema["properties"]["states"]["items"]["oneOf"][1];
        assert_eq!(
            parallel["required"],
            json!(["kind", "id", "title", "branches", "transitions"])
        );
        for key in crate::flow::definition::PARALLEL_STATE_KEYS {
            assert!(parallel["properties"].get(*key).is_some(), "{key}");
        }
        assert_eq!(
            state["required"],
            json!(["id", "title", "harness", "model", "brief", "transitions"])
        );
        assert!(state["properties"]["harness"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("claude")));
        // Every key the Flow format writes is described, so a Flow read back
        // matches the schema.
        for key in crate::flow::definition::FLOW_KEYS {
            assert!(schema["properties"].get(*key).is_some(), "{key}");
        }
        for key in crate::flow::definition::AGENT_STATE_KEYS {
            assert!(state["properties"].get(*key).is_some(), "{key}");
        }
    }

    #[test]
    fn a_pilot_saves_a_parallel_step_with_defaults_filled_in() {
        let step = |id: &str| json!({"id":id,"title":id,"harness":"claude","model":"opus","brief":"Build.","transitions":[]});
        let flow = json!({
            "name":"Release","cwd":"/w","initial":"plan",
            "states":[
                {"id":"plan","title":"Plan","harness":"claude","model":"opus","brief":"Plan.",
                 "outputs":[{"name":"mobile","type":"boolean","description":"mobile/ changed"}],
                 "transitions":[{"to":"build"}]},
                {"kind":"parallel","id":"build","title":"Build & test","branches":["mac","ios"],
                 "transitions":[{"to":SUCCEEDED}]},
                step("mac"),
                {"id":"ios","title":"iOS","harness":"claude","model":"opus","brief":"Build iOS.",
                 "transitions":[],"when":{"kind":"output","field":"mobile","op":"truthy"}},
            ],
        });
        let saved = pilot_flow(&flow, "p1").expect("saves");
        let crate::flow::FlowState::Parallel(build) = &saved.states[1] else {
            panic!("a parallel step");
        };
        assert_eq!(build.branches, ["mac", "ios"]);
        assert_eq!(build.join, crate::flow::definition::Join::All);
        assert_eq!(build.max_visits, DEFAULT_MAX_VISITS);

        let mut missing = flow.clone();
        missing["states"][1]
            .as_object_mut()
            .unwrap()
            .remove("branches");
        assert_eq!(problems(missing), ["state build is missing \"branches\""]);
        let mut typo = flow;
        typo["states"][1]["brnaches"] = json!([]);
        assert_eq!(
            problems(typo),
            ["Unknown field \"brnaches\" in state build"]
        );
    }
}
