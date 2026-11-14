//! Flow definition types (JSON-compatible with older Tauri builds) and validation.
use super::template::list_bindings;
use crate::provider::Kind;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::LazyLock;

pub const SUCCEEDED: &str = "$succeeded";
pub const FAILED: &str = "$failed";
pub const MIN_STATES: usize = 1;
pub const MAX_STATES: usize = 30;
pub const MIN_VISITS: u8 = 1;
pub const MAX_VISITS: u8 = 10;
pub const DEFAULT_MAX_VISITS: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputType {
    #[serde(rename = "string")]
    String,
    #[serde(rename = "number")]
    Number,
    #[serde(rename = "boolean")]
    Boolean,
    #[serde(rename = "string[]")]
    StringList,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OutputValue {
    String(String),
    Number(f64),
    Bool(bool),
    List(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowInput {
    pub name: String,
    pub label: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputField {
    pub name: String,
    pub r#type: OutputType,
    pub description: String,
}

/// How an agent visit ended; guards on `status` match against this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VisitResult {
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardOp {
    Eq,
    Neq,
    Truthy,
    Falsy,
    Contains,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Guard {
    Status {
        is: VisitResult,
    },
    Output {
        field: String,
        op: GuardOp,
        /// A string, number or boolean; checked by `validate_flow`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<serde_json::Value>,
    },
}

/// `to` is a state id, `$succeeded` or `$failed`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Guard>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentState {
    pub id: String,
    pub title: String,
    /// A `provider::Kind` id.
    pub harness: String,
    pub model: String,
    pub brief: String,
    pub outputs: Vec<OutputField>,
    pub transitions: Vec<Transition>,
    pub max_visits: u8,
    /// Only on a parallel step's branch: the branch runs only when this
    /// output guard matches the outputs of the step that led into the
    /// parallel step; otherwise it is skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Guard>,
}

/// How a parallel step's branches join. Only `all` exists: the step ends
/// once every branch that runs has ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Join {
    #[default]
    All,
}

/// Runs its `branches` (agent states) side by side, then follows its
/// transitions once they have all completed (`completed`) or one has failed
/// past its `maxVisits` (`failed`). A failed branch is launched again, told
/// what failed, while it has visits left.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParallelState {
    pub id: String,
    pub title: String,
    pub branches: Vec<String>,
    #[serde(default)]
    pub join: Join,
    pub transitions: Vec<Transition>,
    pub max_visits: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalState {
    pub id: String,
    pub title: String,
    pub message: String,
    pub transitions: Vec<Transition>,
    pub max_visits: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum FlowState {
    Agent(AgentState),
    Approval(ApprovalState),
    Parallel(ParallelState),
}

impl FlowState {
    pub fn id(&self) -> &str {
        match self {
            Self::Agent(s) => &s.id,
            Self::Approval(s) => &s.id,
            Self::Parallel(s) => &s.id,
        }
    }
    pub fn title(&self) -> &str {
        match self {
            Self::Agent(s) => &s.title,
            Self::Approval(s) => &s.title,
            Self::Parallel(s) => &s.title,
        }
    }
    pub fn transitions(&self) -> &[Transition] {
        match self {
            Self::Agent(s) => &s.transitions,
            Self::Approval(s) => &s.transitions,
            Self::Parallel(s) => &s.transitions,
        }
    }
    pub fn transitions_mut(&mut self) -> &mut Vec<Transition> {
        match self {
            Self::Agent(s) => &mut s.transitions,
            Self::Approval(s) => &mut s.transitions,
            Self::Parallel(s) => &mut s.transitions,
        }
    }
    pub fn max_visits(&self) -> u8 {
        match self {
            Self::Agent(s) => s.max_visits,
            Self::Approval(s) => s.max_visits,
            Self::Parallel(s) => s.max_visits,
        }
    }
    fn text(&self) -> &str {
        match self {
            Self::Agent(s) => &s.brief,
            Self::Approval(s) => &s.message,
            Self::Parallel(_) => "",
        }
    }
}

impl Flow {
    pub fn state(&self, id: &str) -> Option<&FlowState> {
        self.states.iter().find(|s| s.id() == id)
    }

    /// The parallel step that runs `id` as one of its branches.
    pub fn parent_of(&self, id: &str) -> Option<&ParallelState> {
        self.states.iter().find_map(|s| match s {
            FlowState::Parallel(p) if p.branches.iter().any(|b| b == id) => Some(p),
            _ => None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Flow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub cwd: String,
    pub inputs: Vec<FlowInput>,
    pub initial: String,
    pub states: Vec<FlowState>,
    /// The `AccessMode` id every agent visit of a run launches with. Flows
    /// saved without it, like the TS app's, read as `"auto"`.
    #[serde(default = "default_access")]
    pub access: String,
    /// The id of the Pilot that owns this Flow. Owned Flows are hidden from
    /// the Flows view and may not contain approval states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Agents run unattended in a Flow, so they default to Auto, like automations.
pub const DEFAULT_ACCESS: &str = "auto";

/// The exact `AccessMode` ids: `AccessMode::from_id` falls back silently.
pub const ACCESS_MODES: [&str; 5] = [
    "read-only",
    "supervised",
    "auto-accept-edits",
    "auto",
    "full-access",
];

impl Flow {
    /// This Flow cut down to its step `state_id` alone, as a schedule that
    /// runs one step starts it: the run begins at that step and ends with
    /// it, succeeded when the step completes and failed otherwise. `None`
    /// when the Flow has no such step.
    pub fn only_step(&self, state_id: &str) -> Option<Flow> {
        let mut state = self.states.iter().find(|s| s.id() == state_id)?.clone();
        let ends = vec![
            Transition {
                to: SUCCEEDED.into(),
                when: Some(Guard::Status {
                    is: VisitResult::Completed,
                }),
            },
            Transition {
                to: FAILED.into(),
                when: None,
            },
        ];
        match &mut state {
            FlowState::Agent(s) => s.transitions = ends,
            FlowState::Approval(s) => s.transitions = ends,
        }
        Some(Flow {
            initial: state_id.into(),
            states: vec![state],
            ..self.clone()
        })
    }
}

fn default_access() -> String {
    DEFAULT_ACCESS.into()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Validation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn approval_outputs() -> Vec<OutputField> {
    vec![
        OutputField {
            name: "approved".into(),
            r#type: OutputType::Boolean,
            description: "Whether the reviewer approved".into(),
        },
        OutputField {
            name: "note".into(),
            r#type: OutputType::String,
            description: "Optional note from the reviewer".into(),
        },
    ]
}

pub fn outputs_of(state: &FlowState) -> Vec<OutputField> {
    match state {
        FlowState::Agent(s) => s.outputs.clone(),
        FlowState::Approval(_) => approval_outputs(),
        FlowState::Parallel(_) => Vec::new(),
    }
}

/// Whether `name` is a valid state, input or output name.
pub(crate) fn is_state_name(name: &str) -> bool {
    valid_name(name)
}

/// `^[a-z][a-z0-9_]{0,39}$`
fn valid_name(name: &str) -> bool {
    static NAME: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^[a-z][a-z0-9_]{0,39}$").unwrap());
    NAME.is_match(name)
}

/// Iterative dominator sets over the states reachable from `initial`.
fn dominators<'a>(
    initial: &'a str,
    reachable: &[&'a str],
    succ: &HashMap<&str, Vec<&'a str>>,
) -> HashMap<&'a str, HashSet<&'a str>> {
    let all: HashSet<&str> = reachable.iter().copied().collect();
    let mut preds: HashMap<&str, Vec<&str>> =
        reachable.iter().map(|id| (*id, Vec::new())).collect();
    for id in reachable {
        for to in succ.get(id).into_iter().flatten() {
            if let Some(list) = preds.get_mut(to) {
                list.push(id);
            }
        }
    }
    let mut dom: HashMap<&str, HashSet<&str>> = reachable
        .iter()
        .map(|id| {
            (
                *id,
                if *id == initial {
                    HashSet::from([*id])
                } else {
                    all.clone()
                },
            )
        })
        .collect();
    let mut changed = true;
    while changed {
        changed = false;
        for id in reachable {
            if *id == initial {
                continue;
            }
            let mut next: HashSet<&str> = match preds[id].split_first() {
                None => HashSet::new(),
                Some((first, rest)) => {
                    let mut set = dom[first].clone();
                    for p in rest {
                        set.retain(|x| dom[p].contains(x));
                    }
                    set
                }
            };
            next.insert(id);
            if next.len() != dom[id].len() {
                dom.insert(id, next);
                changed = true;
            }
        }
    }
    dom
}

pub fn validate_flow(flow: &Flow) -> Validation {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if flow.states.len() < MIN_STATES {
        errors.push("Flow needs at least 1 state".into());
    }
    if !ACCESS_MODES.contains(&flow.access.as_str()) {
        errors.push(format!("Unknown access mode {}", flow.access));
    }
    if flow.states.len() > MAX_STATES {
        errors.push(format!("Flow can have at most {MAX_STATES} states"));
    }

    let mut input_names: HashSet<&str> = HashSet::new();
    for input in &flow.inputs {
        if !valid_name(&input.name) {
            errors.push(format!("Input \"{}\": invalid name", input.name));
        }
        if !input_names.insert(&input.name) {
            errors.push(format!("Input \"{}\": duplicate input", input.name));
        }
    }

    let mut by_id: HashMap<&str, &FlowState> = HashMap::new();
    for state in &flow.states {
        let id = state.id();
        if by_id.contains_key(id) {
            errors.push(format!("duplicate state \"{id}\""));
            continue;
        }
        by_id.insert(id, state);
        if id.starts_with('$') {
            errors.push(format!("State \"{id}\": id is reserved"));
        } else if !valid_name(id) {
            errors.push(format!("State \"{id}\": invalid name"));
        }
        let visits = state.max_visits();
        if !(MIN_VISITS..=MAX_VISITS).contains(&visits) {
            errors.push(format!(
                "State \"{id}\": maxVisits must be an integer from {MIN_VISITS} to {MAX_VISITS}"
            ));
        }
        if flow.owner.is_some() && matches!(state, FlowState::Approval(_)) {
            errors.push(format!(
                "State \"{id}\": approval states are not allowed in a Pilot Flow"
            ));
        }
        if let FlowState::Agent(agent) = state {
            if Kind::parse(&agent.harness).is_none() {
                errors.push(format!(
                    "State \"{id}\": unknown harness \"{}\"",
                    agent.harness
                ));
            }
        }
    }

    if !flow.states.is_empty() && !by_id.contains_key(flow.initial.as_str()) {
        errors.push(format!("initial state \"{}\" is not a state", flow.initial));
    }
    let branch_of = validate_parallel(flow, &by_id, &mut errors);

    let mut succ: HashMap<&str, Vec<&str>> = HashMap::new();
    for state in &flow.states {
        let id = state.id();
        if let FlowState::Agent(agent) = state {
            let mut declared: HashSet<&str> = HashSet::new();
            for field in &agent.outputs {
                if !valid_name(&field.name) {
                    errors.push(format!(
                        "State \"{id}\": output \"{}\" has invalid name",
                        field.name
                    ));
                }
                if !declared.insert(&field.name) {
                    errors.push(format!(
                        "State \"{id}\": duplicate output \"{}\"",
                        field.name
                    ));
                }
            }
        }
        let fields: HashSet<String> = outputs_of(state).into_iter().map(|f| f.name).collect();
        let mut targets: Vec<&str> = Vec::new();
        for t in state.transitions() {
            if t.to != SUCCEEDED && t.to != FAILED && !by_id.contains_key(t.to.as_str()) {
                errors.push(format!("State \"{id}\": unknown target \"{}\"", t.to));
            } else {
                targets.push(&t.to);
            }
            if let Some(Guard::Output { field, value, .. }) = &t.when {
                if !fields.contains(field) {
                    errors.push(format!(
                        "State \"{id}\": unknown output \"{field}\" in transition guard"
                    ));
                }
                let scalar =
                    |v: &serde_json::Value| v.is_string() || v.is_number() || v.is_boolean();
                if value.as_ref().is_some_and(|v| !scalar(v)) {
                    errors.push(format!(
                        "State \"{id}\": guard value for \"{field}\" must be a string, number or boolean"
                    ));
                }
            }
        }
        succ.entry(id).or_default().extend(targets);
        if let Some(parent) = branch_of.get(id) {
            if !state.transitions().is_empty() {
                errors.push(format!(
                    "State \"{id}\": a branch of \"{parent}\" has no transitions; \"{parent}\"'s transitions follow it"
                ));
            }
        }

        for (r, _) in list_bindings(state.text()) {
            let parts: Vec<&str> = r.split('.').collect();
            if parts[0] == "inputs" {
                if !input_names.contains(parts[1]) {
                    errors.push(format!("State \"{id}\": unknown input \"{}\"", parts[1]));
                }
                continue;
            }
            match by_id.get(parts[1]) {
                None => errors.push(format!(
                    "State \"{id}\": unknown state \"{}\" in binding {r}",
                    parts[1]
                )),
                Some(source) => {
                    if !outputs_of(source).iter().any(|f| f.name == parts[3]) {
                        errors.push(format!(
                            "State \"{id}\": unknown output \"{}\" on state \"{}\" in binding {r}",
                            parts[3], parts[1]
                        ));
                    }
                }
            }
        }
    }

    // A parallel step reads as its branches one after another, then its
    // transitions: every branch is reachable through it, and a step after
    // it may bind any branch's outputs.
    for state in &flow.states {
        let FlowState::Parallel(parallel) = state else {
            continue;
        };
        let chain: Vec<&str> = parallel
            .branches
            .iter()
            .map(String::as_str)
            .filter(|b| by_id.contains_key(b))
            .collect();
        let (Some(first), Some(last)) = (chain.first(), chain.last()) else {
            continue;
        };
        let targets = succ.insert(&parallel.id, vec![*first]).unwrap_or_default();
        for pair in chain.windows(2) {
            succ.entry(pair[0]).or_default().push(pair[1]);
        }
        succ.entry(*last).or_default().extend(targets);
    }

    if by_id.contains_key(flow.initial.as_str()) {
        let initial = flow.initial.as_str();
        let mut reachable: Vec<&str> = vec![initial];
        let mut seen: HashSet<&str> = HashSet::from([initial]);
        let mut queue: VecDeque<&str> = VecDeque::from([initial]);
        let mut can_succeed = false;
        while let Some(id) = queue.pop_front() {
            for to in succ.get(id).into_iter().flatten() {
                if *to == SUCCEEDED {
                    can_succeed = true;
                }
                if by_id.contains_key(to) && seen.insert(to) {
                    reachable.push(to);
                    queue.push_back(to);
                }
            }
        }
        for state in &flow.states {
            if !seen.contains(state.id()) {
                errors.push(format!(
                    "State \"{}\" is unreachable from \"{initial}\"",
                    state.id()
                ));
            }
        }
        if !can_succeed {
            errors.push(format!(
                "Flow cannot succeed: no path from \"{initial}\" to {SUCCEEDED}"
            ));
        }

        let dom = dominators(initial, &reachable, &succ);
        for state in &flow.states {
            let id = state.id();
            if !seen.contains(id) {
                continue;
            }
            let mut counted: HashSet<String> = HashSet::new();
            for (r, optional) in list_bindings(state.text()) {
                if optional || !r.starts_with("states.") || counted.contains(&r) {
                    continue;
                }
                counted.insert(r.clone());
                let source = r.split('.').nth(1).unwrap_or_default();
                if !by_id.contains_key(source) {
                    continue;
                }
                let guaranteed = source != id && dom[id].contains(source);
                if !guaranteed {
                    warnings.push(format!(
                        "State \"{id}\": binding {r} may have no value when the state first runs; use {{{{{r}?}}}} to make it optional"
                    ));
                }
            }
        }
    }

    Validation { errors, warnings }
}

/// Checks every parallel step and its branches, and returns each branch's
/// parallel step.
fn validate_parallel<'a>(
    flow: &'a Flow,
    by_id: &HashMap<&'a str, &'a FlowState>,
    errors: &mut Vec<String>,
) -> HashMap<&'a str, &'a str> {
    let mut branch_of: HashMap<&str, &str> = HashMap::new();
    for state in &flow.states {
        let FlowState::Parallel(parallel) = state else {
            continue;
        };
        let id = parallel.id.as_str();
        if parallel.branches.len() < 2 {
            errors.push(format!(
                "State \"{id}\": a parallel step needs at least 2 branches"
            ));
        }
        let mut listed: HashSet<&str> = HashSet::new();
        for branch in &parallel.branches {
            if !listed.insert(branch) {
                errors.push(format!(
                    "State \"{id}\": branch \"{branch}\" is listed twice"
                ));
                continue;
            }
            match by_id.get(branch.as_str()) {
                None => errors.push(format!("State \"{id}\": unknown branch \"{branch}\"")),
                Some(FlowState::Agent(_)) => {
                    if let Some(other) = branch_of.insert(branch, id) {
                        errors.push(format!(
                            "State \"{branch}\": a branch of both \"{other}\" and \"{id}\""
                        ));
                    }
                }
                Some(_) => errors.push(format!(
                    "State \"{id}\": branch \"{branch}\" must be an agent step"
                )),
            }
        }
    }
    // The steps whose transitions lead to each state.
    let mut leading: HashMap<&str, Vec<&FlowState>> = HashMap::new();
    for state in &flow.states {
        for t in state.transitions() {
            leading.entry(t.to.as_str()).or_default().push(state);
        }
    }
    for state in &flow.states {
        let FlowState::Agent(agent) = state else {
            continue;
        };
        let id = agent.id.as_str();
        let Some(parent) = branch_of.get(id) else {
            if agent.when.is_some() {
                errors.push(format!(
                    "State \"{id}\": only a parallel step's branch can have \"when\""
                ));
            }
            continue;
        };
        if id == flow.initial {
            errors.push(format!(
                "State \"{id}\": a branch of \"{parent}\" cannot be the initial state"
            ));
        }
        if leading.contains_key(id) {
            errors.push(format!(
                "State \"{id}\": a branch of \"{parent}\" runs only from it, so no transition may lead to it"
            ));
        }
        match &agent.when {
            None => {}
            Some(Guard::Status { .. }) => errors.push(format!(
                "State \"{id}\": a branch's \"when\" must be an output guard"
            )),
            Some(Guard::Output { field, .. }) => {
                let before = leading.get(parent).map(Vec::as_slice).unwrap_or_default();
                let named = |s: &&FlowState| outputs_of(s).iter().any(|f| &f.name == field);
                if before.is_empty() || !before.iter().all(named) {
                    errors.push(format!(
                        "State \"{id}\": \"when\" needs output \"{field}\" on every step that leads to \"{parent}\""
                    ));
                }
            }
        }
    }
    branch_of
}

/// The JSON keys each Flow type writes. Serde ignores keys it doesn't know,
/// so the editor compares typed text against these to catch typos. A test
/// pins every list to what serde writes.
pub const FLOW_KEYS: &[&str] = &[
    "id",
    "name",
    "description",
    "cwd",
    "inputs",
    "initial",
    "states",
    "access",
    "owner",
    "createdAt",
    "updatedAt",
];
pub const INPUT_KEYS: &[&str] = &["name", "label", "required", "default"];
pub const AGENT_STATE_KEYS: &[&str] = &[
    "kind",
    "id",
    "title",
    "harness",
    "model",
    "brief",
    "outputs",
    "transitions",
    "maxVisits",
    "when",
];
pub const APPROVAL_STATE_KEYS: &[&str] =
    &["kind", "id", "title", "message", "transitions", "maxVisits"];
pub const PARALLEL_STATE_KEYS: &[&str] = &[
    "kind",
    "id",
    "title",
    "branches",
    "join",
    "transitions",
    "maxVisits",
];
pub const OUTPUT_FIELD_KEYS: &[&str] = &["name", "type", "description"];
pub const TRANSITION_KEYS: &[&str] = &["to", "when"];
pub const STATUS_GUARD_KEYS: &[&str] = &["kind", "is"];
pub const OUTPUT_GUARD_KEYS: &[&str] = &["kind", "field", "op", "value"];

/// One error per key in `flow_json` that the Flow format doesn't define, such
/// as a misspelled `maxVisit`. Any shape is accepted: parts
/// of an unexpected shape are skipped, and items without an id or name are
/// named by their 1-based position.
pub fn unknown_fields(flow_json: &serde_json::Value) -> Vec<String> {
    use serde_json::Value;
    /// The item's `key` text, or its 1-based position when it has none.
    fn name_of(value: &Value, key: &str, index: usize) -> String {
        match value.get(key).and_then(Value::as_str) {
            Some(name) if !name.is_empty() => name.into(),
            _ => (index + 1).to_string(),
        }
    }
    fn scan(value: &Value, known: &[&str], place: &str, errors: &mut Vec<String>) {
        if let Some(object) = value.as_object() {
            for key in object.keys().filter(|k| !known.contains(&k.as_str())) {
                errors.push(format!("Unknown field \"{key}\" in {place}"));
            }
        }
    }
    let mut errors = Vec::new();
    scan(flow_json, FLOW_KEYS, "the Flow", &mut errors);
    let list = |value: &Value, key: &str| -> Vec<Value> {
        value
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    for (index, input) in list(flow_json, "inputs").iter().enumerate() {
        let place = format!("input {}", name_of(input, "name", index));
        scan(input, INPUT_KEYS, &place, &mut errors);
    }
    for (position, state) in list(flow_json, "states").iter().enumerate() {
        let id = name_of(state, "id", position);
        let place = format!("state {id}");
        let known = match state.get("kind").and_then(Value::as_str) {
            Some("approval") => APPROVAL_STATE_KEYS,
            Some("parallel") => PARALLEL_STATE_KEYS,
            _ => AGENT_STATE_KEYS,
        };
        scan(state, known, &place, &mut errors);
        for (index, output) in list(state, "outputs").iter().enumerate() {
            let place = format!("state {id} output {}", name_of(output, "name", index));
            scan(output, OUTPUT_FIELD_KEYS, &place, &mut errors);
        }
        for (index, transition) in list(state, "transitions").iter().enumerate() {
            let place = format!("state {id} transition {}", index + 1);
            scan(transition, TRANSITION_KEYS, &place, &mut errors);
            if let Some(guard) = transition.get("when") {
                let known = if guard.get("kind").and_then(Value::as_str) == Some("output") {
                    OUTPUT_GUARD_KEYS
                } else {
                    STATUS_GUARD_KEYS
                };
                scan(guard, known, &format!("{place} guard"), &mut errors);
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::starter::starter_flows;

    fn field(name: &str, ty: OutputType) -> OutputField {
        OutputField {
            name: name.into(),
            r#type: ty,
            description: String::new(),
        }
    }

    fn to(target: &str) -> Transition {
        Transition {
            to: target.into(),
            when: None,
        }
    }

    /// A Flow that writes every key, optional ones included.
    fn fully_populated() -> Flow {
        let mut flow = starter_flows().remove(0);
        flow.owner = Some("p1".into());
        flow.inputs[0].default = Some("x".into());
        let approval = FlowState::Approval(ApprovalState {
            id: "gate".into(),
            title: "Gate".into(),
            message: "ok?".into(),
            transitions: vec![
                Transition {
                    to: SUCCEEDED.into(),
                    when: Some(Guard::Status {
                        is: VisitResult::Completed,
                    }),
                },
                Transition {
                    to: FAILED.into(),
                    when: Some(Guard::Output {
                        field: "approved".into(),
                        op: GuardOp::Eq,
                        value: Some(serde_json::json!(true)),
                    }),
                },
            ],
            max_visits: 1,
        });
        let mut branch = agent("mobile");
        branch.transitions.clear();
        branch.outputs = vec![field("built", OutputType::Boolean)];
        branch.when = Some(Guard::Output {
            field: "changed".into(),
            op: GuardOp::Truthy,
            value: None,
        });
        flow.states.push(FlowState::Agent(branch));
        flow.states.push(FlowState::Parallel(ParallelState {
            id: "both".into(),
            title: "Both".into(),
            branches: vec!["gate".into(), "mobile".into()],
            join: Join::All,
            transitions: vec![to(SUCCEEDED)],
            max_visits: 1,
        }));
        // The approval stays last, where the unknown-field test edits it.
        flow.states.push(approval);
        flow
    }

    fn keys(value: &serde_json::Value) -> Vec<String> {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    fn sorted(list: &[&str]) -> Vec<String> {
        let mut list: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        list.sort();
        list
    }

    #[test]
    fn pilot_flows_reject_approval_states() {
        let mut flow = starter_flows().remove(0);
        let gate = FlowState::Approval(ApprovalState {
            id: "gate".into(),
            title: "Gate".into(),
            message: "ok?".into(),
            transitions: vec![to(SUCCEEDED)],
            max_visits: 1,
        });
        flow.states.push(gate);
        let msg = "State \"gate\": approval states are not allowed in a Pilot Flow";
        assert!(!validate_flow(&flow).errors.iter().any(|e| e == msg));
        flow.owner = Some("p1".into());
        assert!(validate_flow(&flow).errors.iter().any(|e| e == msg));
    }

    #[test]
    fn owner_round_trips_and_old_json_without_it_still_parses() {
        let mut flow = starter_flows().remove(0);
        let json = serde_json::to_value(&flow).unwrap();
        assert!(json.get("owner").is_none());
        let old: Flow = serde_json::from_value(json).unwrap();
        assert_eq!(old.owner, None);
        flow.owner = Some("p1".into());
        let json = serde_json::to_value(&flow).unwrap();
        assert_eq!(json["owner"], "p1");
        let back: Flow = serde_json::from_value(json).unwrap();
        assert_eq!(back.owner.as_deref(), Some("p1"));
    }

    #[test]
    fn the_known_key_lists_match_what_serde_writes() {
        let json = serde_json::to_value(fully_populated()).unwrap();
        assert_eq!(keys(&json), sorted(FLOW_KEYS));
        assert_eq!(keys(&json["inputs"][0]), sorted(INPUT_KEYS));
        let states = json["states"].as_array().unwrap();
        let agent = states.iter().find(|s| s["id"] == "mobile").unwrap();
        let approval = states.iter().find(|s| s["id"] == "gate").unwrap();
        let parallel = states.iter().find(|s| s["kind"] == "parallel").unwrap();
        assert_eq!(keys(agent), sorted(AGENT_STATE_KEYS));
        assert_eq!(keys(approval), sorted(APPROVAL_STATE_KEYS));
        assert_eq!(keys(parallel), sorted(PARALLEL_STATE_KEYS));
        assert_eq!(keys(&agent["outputs"][0]), sorted(OUTPUT_FIELD_KEYS));
        assert_eq!(keys(&approval["transitions"][0]), sorted(TRANSITION_KEYS));
        assert_eq!(
            keys(&approval["transitions"][0]["when"]),
            sorted(STATUS_GUARD_KEYS)
        );
        assert_eq!(
            keys(&approval["transitions"][1]["when"]),
            sorted(OUTPUT_GUARD_KEYS)
        );
        // Everything serde writes is known, so a saved Flow has no unknown keys.
        assert_eq!(unknown_fields(&json), Vec::<String>::new());
    }

    #[test]
    fn unknown_fields_name_the_key_and_where_it_is() {
        let mut json = serde_json::to_value(fully_populated()).unwrap();
        json["extra"] = 1.into();
        json["inputs"][0]["lable"] = 1.into();
        json["states"][0]["maxVisit"] = 5.into();
        json["states"][0]["outputs"][0]["desc"] = 1.into();
        json["states"][0]["transitions"][0]["whn"] = 1.into();
        let last = json["states"].as_array().unwrap().len() - 1;
        json["states"][last]["transitions"][1]["when"]["valeu"] = 1.into();
        json["states"][last]["transitions"][0]["when"]["op"] = "eq".into();
        let id0 = json["states"][0]["id"].as_str().unwrap().to_owned();
        let input = json["inputs"][0]["name"].as_str().unwrap().to_owned();
        let output = json["states"][0]["outputs"][0]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        let errors = unknown_fields(&json);
        assert_eq!(
            errors,
            vec![
                "Unknown field \"extra\" in the Flow".to_string(),
                format!("Unknown field \"lable\" in input {input}"),
                format!("Unknown field \"maxVisit\" in state {id0}"),
                format!("Unknown field \"desc\" in state {id0} output {output}"),
                format!("Unknown field \"whn\" in state {id0} transition 1"),
                "Unknown field \"op\" in state gate transition 1 guard".to_string(),
                "Unknown field \"valeu\" in state gate transition 2 guard".to_string(),
            ]
        );
    }

    fn when_output(target: &str, field: &str, op: GuardOp) -> Transition {
        Transition {
            to: target.into(),
            when: Some(Guard::Output {
                field: field.into(),
                op,
                value: None,
            }),
        }
    }

    fn agent(id: &str) -> AgentState {
        AgentState {
            id: id.into(),
            title: id.into(),
            harness: "claude".into(),
            model: "claude:sonnet-5".into(),
            brief: "do it".into(),
            outputs: vec![],
            transitions: vec![to(SUCCEEDED)],
            max_visits: 3,
            when: None,
        }
    }

    fn make_flow(states: Vec<FlowState>) -> Flow {
        Flow {
            id: "f1".into(),
            name: "Flow".into(),
            description: String::new(),
            cwd: "/repo".into(),
            inputs: vec![],
            initial: "a".into(),
            states,
            access: "auto".into(),
            owner: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn one(state: AgentState) -> Flow {
        make_flow(vec![FlowState::Agent(state)])
    }

    fn errors_of(flow: &Flow) -> String {
        validate_flow(flow).errors.join("\n")
    }

    #[test]
    fn only_step_starts_and_ends_at_that_step() {
        let mut plan = agent("a");
        plan.transitions = vec![to("b")];
        let mut build = agent("b");
        build.transitions = vec![to("a"), to(SUCCEEDED)];
        let flow = make_flow(vec![FlowState::Agent(plan), FlowState::Agent(build)]);
        let only = flow.only_step("b").unwrap();
        assert_eq!(only.initial, "b");
        assert_eq!(only.states.len(), 1);
        assert_eq!(only.states[0].id(), "b");
        let ends: Vec<_> = only.states[0]
            .transitions()
            .iter()
            .map(|t| t.to.as_str())
            .collect();
        assert_eq!(ends, [SUCCEEDED, FAILED], "it never goes back to plan");
        assert_eq!((only.id.as_str(), only.cwd.as_str()), ("f1", "/repo"));
        assert_eq!(validate_flow(&only).errors, Vec::<String>::new());
        assert!(flow.only_step("missing").is_none());
    }

    #[test]
    fn accepts_a_minimal_flow() {
        assert_eq!(validate_flow(&one(agent("a"))), Validation::default());
    }

    #[test]
    fn requires_at_least_1_state() {
        assert!(errors_of(&make_flow(vec![])).contains("at least 1 state"));
    }

    #[test]
    fn allows_30_states_but_rejects_31() {
        let chain = |n: usize| {
            let states = (0..n)
                .map(|i| {
                    let next = if i + 1 < n {
                        format!("s{}", i + 1)
                    } else {
                        SUCCEEDED.into()
                    };
                    FlowState::Agent(AgentState {
                        transitions: vec![to(&next)],
                        ..agent(&format!("s{i}"))
                    })
                })
                .collect();
            Flow {
                initial: "s0".into(),
                ..make_flow(states)
            }
        };
        assert!(!errors_of(&chain(30)).contains("at most 30 states"));
        assert!(errors_of(&chain(31)).contains("at most 30 states"));
    }

    #[test]
    fn rejects_duplicate_state_ids() {
        let f = make_flow(vec![
            FlowState::Agent(agent("a")),
            FlowState::Agent(agent("a")),
        ]);
        assert!(errors_of(&f).contains("duplicate state \"a\""));
    }

    #[test]
    fn rejects_an_initial_that_is_not_a_state() {
        let f = Flow {
            initial: "zzz".into(),
            ..one(agent("a"))
        };
        assert!(errors_of(&f).contains("initial"));
    }

    #[test]
    fn rejects_unknown_transition_targets_and_names_the_state() {
        let f = one(AgentState {
            transitions: vec![to("nope"), to(SUCCEEDED)],
            ..agent("a")
        });
        assert!(errors_of(&f).contains("State \"a\": unknown target \"nope\""));
    }

    #[test]
    fn rejects_state_ids_starting_with_dollar_as_reserved() {
        let f = Flow {
            initial: "$x".into(),
            ..one(agent("$x"))
        };
        assert!(errors_of(&f).contains("reserved"));
    }

    #[test]
    fn rejects_invalid_state_input_and_output_names() {
        let f = Flow {
            initial: "Bad-Name".into(),
            ..one(agent("Bad-Name"))
        };
        assert!(errors_of(&f).contains("invalid name"));

        let f = Flow {
            inputs: vec![FlowInput {
                name: "Bad-Name".into(),
                label: "x".into(),
                required: true,
                default: None,
            }],
            ..one(agent("a"))
        };
        assert!(errors_of(&f).contains("invalid name"));

        let f = one(AgentState {
            outputs: vec![field("Bad-Name", OutputType::String)],
            ..agent("a")
        });
        assert!(errors_of(&f).contains("invalid name"));
    }

    #[test]
    fn rejects_names_longer_than_40_chars() {
        let long = format!("a{}", "b".repeat(40));
        let f = Flow {
            initial: long.clone(),
            ..one(agent(&long))
        };
        assert!(errors_of(&f).contains("invalid name"));
        let ok = format!("a{}", "b".repeat(39));
        let f = Flow {
            initial: ok.clone(),
            ..one(agent(&ok))
        };
        assert!(!errors_of(&f).contains("invalid name"));
    }

    #[test]
    fn rejects_max_visits_outside_1_to_10() {
        // The TS twin also rejects 2.5; `u8` makes a fractional value
        // unrepresentable (it fails to deserialize instead).
        for max_visits in [0u8, 11] {
            let f = one(AgentState {
                max_visits,
                ..agent("a")
            });
            assert!(errors_of(&f).contains("maxVisits"), "{max_visits}");
        }
        for max_visits in [1u8, 10] {
            let f = one(AgentState {
                max_visits,
                ..agent("a")
            });
            assert!(!errors_of(&f).contains("maxVisits"), "{max_visits}");
        }
    }

    #[test]
    fn rejects_output_guards_on_undeclared_fields() {
        let f = one(AgentState {
            outputs: vec![field("ok", OutputType::Boolean)],
            transitions: vec![
                when_output(SUCCEEDED, "missing", GuardOp::Truthy),
                to(FAILED),
            ],
            ..agent("a")
        });
        assert!(errors_of(&f).contains("State \"a\": unknown output \"missing\""));
    }

    #[test]
    fn accepts_approval_guards_on_approved_and_note() {
        let f = Flow {
            initial: "r".into(),
            ..make_flow(vec![FlowState::Approval(ApprovalState {
                id: "r".into(),
                title: "R".into(),
                message: "ok?".into(),
                max_visits: 3,
                transitions: vec![
                    when_output(SUCCEEDED, "approved", GuardOp::Truthy),
                    to(FAILED),
                ],
            })])
        };
        assert!(validate_flow(&f).errors.is_empty());
        assert_eq!(outputs_of(&f.states[0]), approval_outputs());
    }

    #[test]
    fn rejects_bindings_to_unknown_inputs() {
        let f = one(AgentState {
            brief: "see {{inputs.x}}".into(),
            ..agent("a")
        });
        assert!(errors_of(&f).contains("unknown input \"x\""));
    }

    #[test]
    fn rejects_bindings_to_undeclared_outputs_and_unknown_states() {
        let f = Flow {
            initial: "s".into(),
            ..make_flow(vec![
                FlowState::Agent(AgentState {
                    outputs: vec![field("g", OutputType::String)],
                    transitions: vec![to("t")],
                    ..agent("s")
                }),
                FlowState::Agent(AgentState {
                    brief: "{{states.s.outputs.f}} {{states.ghost.outputs.f}}".into(),
                    ..agent("t")
                }),
            ])
        };
        let e = errors_of(&f);
        assert!(e.contains("unknown output \"f\""));
        assert!(e.contains("unknown state \"ghost\""));
    }

    #[test]
    fn rejects_unreachable_states() {
        let f = make_flow(vec![
            FlowState::Agent(agent("a")),
            FlowState::Agent(agent("island")),
        ]);
        assert!(errors_of(&f).contains("State \"island\" is unreachable"));
    }

    #[test]
    fn rejects_flows_that_cannot_reach_succeeded() {
        let f = one(AgentState {
            transitions: vec![to(FAILED)],
            ..agent("a")
        });
        assert!(errors_of(&f).contains("cannot succeed"));
    }

    #[test]
    fn harness_must_be_a_native_kind() {
        let f = one(AgentState {
            harness: "nonexistent".into(),
            ..agent("a")
        });
        assert!(errors_of(&f).contains("unknown harness"));
        for kind in Kind::ALL {
            let f = one(AgentState {
                harness: kind.id().into(),
                ..agent("a")
            });
            assert!(validate_flow(&f).errors.is_empty(), "{}", kind.id());
        }
    }

    #[test]
    fn rejects_guard_values_that_are_not_scalars() {
        let guard = |value: serde_json::Value| {
            one(AgentState {
                outputs: vec![field("v", OutputType::String)],
                transitions: vec![
                    Transition {
                        to: SUCCEEDED.into(),
                        when: Some(Guard::Output {
                            field: "v".into(),
                            op: GuardOp::Eq,
                            value: Some(value),
                        }),
                    },
                    to(FAILED),
                ],
                ..agent("a")
            })
        };
        for ok in [
            serde_json::json!("x"),
            serde_json::json!(2),
            serde_json::json!(true),
        ] {
            assert!(validate_flow(&guard(ok)).errors.is_empty());
        }
        for bad in [
            serde_json::json!(null),
            serde_json::json!([1]),
            serde_json::json!({"a": 1}),
        ] {
            assert!(errors_of(&guard(bad)).contains("must be a string, number or boolean"));
        }
    }

    fn loop_flow(binding_suffix: &str) -> Flow {
        Flow {
            initial: "implement".into(),
            ..make_flow(vec![
                FlowState::Agent(AgentState {
                    brief: format!(
                        "fix. failures: {{{{states.test.outputs.failures{binding_suffix}}}}}"
                    ),
                    transitions: vec![to("test")],
                    ..agent("implement")
                }),
                FlowState::Agent(AgentState {
                    outputs: vec![
                        field("passed", OutputType::Boolean),
                        field("failures", OutputType::String),
                    ],
                    transitions: vec![
                        when_output(SUCCEEDED, "passed", GuardOp::Truthy),
                        to("implement"),
                    ],
                    ..agent("test")
                }),
            ])
        }
    }

    #[test]
    fn warns_on_a_non_optional_binding_to_a_later_state() {
        let r = validate_flow(&loop_flow(""));
        assert!(r.errors.is_empty());
        assert!(r.warnings.join("\n").contains("may have no value"));
    }

    #[test]
    fn does_not_warn_when_the_binding_is_optional() {
        assert!(validate_flow(&loop_flow("?")).warnings.is_empty());
    }

    #[test]
    fn does_not_warn_when_the_source_dominates_the_binding_state() {
        let f = Flow {
            initial: "p".into(),
            ..make_flow(vec![
                FlowState::Agent(AgentState {
                    outputs: vec![field("plan", OutputType::String)],
                    transitions: vec![to("q")],
                    ..agent("p")
                }),
                FlowState::Agent(AgentState {
                    brief: "{{states.p.outputs.plan}}".into(),
                    ..agent("q")
                }),
            ])
        };
        assert!(validate_flow(&f).warnings.is_empty());
    }

    #[test]
    fn warns_when_one_path_skips_the_source_state() {
        let f = Flow {
            initial: "r".into(),
            ..make_flow(vec![
                FlowState::Agent(AgentState {
                    outputs: vec![field("x", OutputType::Boolean)],
                    transitions: vec![when_output("p", "x", GuardOp::Truthy), to("q")],
                    ..agent("r")
                }),
                FlowState::Agent(AgentState {
                    outputs: vec![field("plan", OutputType::String)],
                    transitions: vec![to("q")],
                    ..agent("p")
                }),
                FlowState::Agent(AgentState {
                    brief: "{{states.p.outputs.plan}}".into(),
                    ..agent("q")
                }),
            ])
        };
        assert!(validate_flow(&f)
            .warnings
            .join("\n")
            .contains("may have no value"));
    }

    #[test]
    fn warns_on_a_non_optional_self_reference() {
        let f = one(AgentState {
            brief: "{{states.a.outputs.n}}".into(),
            outputs: vec![field("n", OutputType::String)],
            transitions: vec![
                Transition {
                    to: "a".into(),
                    when: Some(Guard::Status {
                        is: VisitResult::Failed,
                    }),
                },
                to(SUCCEEDED),
            ],
            ..agent("a")
        });
        assert!(validate_flow(&f)
            .warnings
            .join("\n")
            .contains("may have no value"));
    }

    #[test]
    fn every_starter_validates_with_no_errors_or_warnings() {
        for f in starter_flows() {
            let r = validate_flow(&Flow {
                id: "x".into(),
                cwd: "/r".into(),
                ..f
            });
            assert_eq!(r, Validation::default());
        }
    }

    #[test]
    fn flow_json_round_trips_with_ts_shape() {
        let fixture = serde_json::json!({
            "id": "f1",
            "name": "Fix",
            "description": "d",
            "cwd": "/repo",
            "inputs": [
                {"name": "issue", "label": "Issue", "required": true},
                {"name": "notes", "label": "Notes", "required": false, "default": "none"}
            ],
            "initial": "a",
            "states": [
                {
                    "kind": "agent", "id": "a", "title": "A", "harness": "codex", "model": "",
                    "brief": "go", "maxVisits": 3,
                    "outputs": [
                        {"name": "passed", "type": "boolean", "description": "ok"},
                        {"name": "files", "type": "string[]", "description": "f"},
                        {"name": "n", "type": "number", "description": "n"}
                    ],
                    "transitions": [
                        {"to": "r", "when": {"kind": "output", "field": "passed", "op": "truthy"}},
                        {"to": "r", "when": {"kind": "output", "field": "n", "op": "eq", "value": 2}},
                        {"to": "$failed", "when": {"kind": "status", "is": "failed"}},
                        {"to": "r"}
                    ]
                },
                {
                    "kind": "approval", "id": "r", "title": "R", "message": "ok?", "maxVisits": 1,
                    "transitions": [{"to": "$succeeded"}]
                }
            ],
            "access": "full-access",
            "createdAt": 10,
            "updatedAt": 20
        });
        let flow: Flow = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!(flow.created_at, 10);
        assert_eq!(flow.access, "full-access");
        assert!(matches!(&flow.states[1], FlowState::Approval(s) if s.max_visits == 1));
        assert_eq!(serde_json::to_value(&flow).unwrap(), fixture);
    }

    #[test]
    fn a_flow_saved_without_access_reads_as_auto_and_writes_it_back() {
        // A Flow saved before access existed, or written by the TS app.
        let mut older =
            serde_json::to_value(make_flow(vec![FlowState::Agent(agent("a"))])).unwrap();
        older.as_object_mut().unwrap().remove("access");
        let flow: Flow = serde_json::from_value(older).unwrap();
        assert_eq!(flow.access, "auto");
        assert_eq!(serde_json::to_value(&flow).unwrap()["access"], "auto");
        assert_eq!(validate_flow(&flow), Validation::default());
    }

    #[test]
    fn access_mode_ids_match_the_agents_access_modes() {
        for id in ACCESS_MODES {
            assert_eq!(crate::agent::AccessMode::from_id(id).id(), id);
        }
    }

    #[test]
    fn access_must_be_an_exact_access_mode_id() {
        for id in [
            "read-only",
            "supervised",
            "auto-accept-edits",
            "auto",
            "full-access",
        ] {
            let mut flow = one(agent("a"));
            flow.access = id.into();
            assert_eq!(validate_flow(&flow), Validation::default(), "{id}");
        }
        // `AccessMode::from_id` would quietly read these as supervised.
        for id in ["root", "", "Auto", "full_access"] {
            let mut flow = one(agent("a"));
            flow.access = id.into();
            assert_eq!(errors_of(&flow), format!("Unknown access mode {id}"));
        }
    }

    #[test]
    fn output_value_json_forms() {
        let v: Vec<OutputValue> = serde_json::from_str(r#"["a", 2, true, ["x", "y"]]"#).unwrap();
        assert_eq!(
            v,
            vec![
                OutputValue::String("a".into()),
                OutputValue::Number(2.0),
                OutputValue::Bool(true),
                OutputValue::List(vec!["x".into(), "y".into()]),
            ]
        );
    }

    // ---- parallel steps ------------------------------------------------

    /// a (reports `changed`) → both (x, y side by side) → z, whose brief
    /// binds an output of branch x.
    fn with_parallel() -> Flow {
        let leaf = |id: &str| {
            FlowState::Agent(AgentState {
                transitions: vec![],
                ..agent(id)
            })
        };
        let mut x = AgentState {
            transitions: vec![],
            outputs: vec![field("artifact", OutputType::String)],
            ..agent("x")
        };
        x.brief = "Build it.".into();
        make_flow(vec![
            FlowState::Agent(AgentState {
                outputs: vec![field("changed", OutputType::Boolean)],
                transitions: vec![to("both")],
                ..agent("a")
            }),
            FlowState::Parallel(ParallelState {
                id: "both".into(),
                title: "Both".into(),
                branches: vec!["x".into(), "y".into()],
                join: Join::All,
                transitions: vec![to("z")],
                max_visits: 1,
            }),
            FlowState::Agent(x),
            leaf("y"),
            FlowState::Agent(AgentState {
                brief: "Ship {{states.x.outputs.artifact}}.".into(),
                ..agent("z")
            }),
        ])
    }

    fn parallel_mut(flow: &mut Flow) -> &mut ParallelState {
        flow.states
            .iter_mut()
            .find_map(|s| match s {
                FlowState::Parallel(p) => Some(p),
                _ => None,
            })
            .unwrap()
    }

    fn agent_mut<'a>(flow: &'a mut Flow, id: &str) -> &'a mut AgentState {
        flow.states
            .iter_mut()
            .find_map(|s| match s {
                FlowState::Agent(a) if a.id == id => Some(a),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn a_parallel_step_is_valid_and_a_later_step_may_bind_any_branch() {
        // Branches are reachable through their step, and a branch's output
        // is guaranteed after the join, so binding it gives no warning.
        assert_eq!(validate_flow(&with_parallel()), Validation::default());
    }

    #[test]
    fn a_parallel_step_needs_two_known_agent_branches_listed_once() {
        let mut flow = with_parallel();
        parallel_mut(&mut flow).branches = vec!["x".into()];
        assert!(
            errors_of(&flow).contains("State \"both\": a parallel step needs at least 2 branches")
        );

        let mut flow = with_parallel();
        parallel_mut(&mut flow).branches.push("nope".into());
        assert!(errors_of(&flow).contains("State \"both\": unknown branch \"nope\""));

        let mut flow = with_parallel();
        parallel_mut(&mut flow).branches.push("x".into());
        assert!(errors_of(&flow).contains("State \"both\": branch \"x\" is listed twice"));

        let mut flow = with_parallel();
        parallel_mut(&mut flow).branches.push("both".into());
        assert!(errors_of(&flow).contains("State \"both\": branch \"both\" must be an agent step"));
    }

    #[test]
    fn a_branch_belongs_to_its_step_alone() {
        let mut flow = with_parallel();
        agent_mut(&mut flow, "x").transitions = vec![to("z")];
        assert!(errors_of(&flow).contains(
            "State \"x\": a branch of \"both\" has no transitions; \"both\"'s transitions follow it"
        ));

        let mut flow = with_parallel();
        agent_mut(&mut flow, "a").transitions = vec![to("x")];
        assert!(errors_of(&flow).contains(
            "State \"x\": a branch of \"both\" runs only from it, so no transition may lead to it"
        ));

        let mut flow = with_parallel();
        flow.initial = "x".into();
        assert!(errors_of(&flow)
            .contains("State \"x\": a branch of \"both\" cannot be the initial state"));

        let mut flow = with_parallel();
        flow.states.push(FlowState::Parallel(ParallelState {
            id: "again".into(),
            branches: vec!["x".into(), "y".into()],
            ..parallel_mut(&mut with_parallel()).clone()
        }));
        agent_mut(&mut flow, "z").transitions = vec![to("again")];
        let errors = errors_of(&flow);
        assert!(
            errors.contains("State \"x\": a branch of both \"both\" and \"again\""),
            "{errors}"
        );
    }

    #[test]
    fn a_branch_condition_names_an_output_of_every_step_before_it() {
        let changed = |field: &str| {
            Some(Guard::Output {
                field: field.into(),
                op: GuardOp::Truthy,
                value: None,
            })
        };
        let mut flow = with_parallel();
        agent_mut(&mut flow, "y").when = changed("changed");
        assert_eq!(validate_flow(&flow), Validation::default());

        agent_mut(&mut flow, "y").when = changed("missing");
        assert!(errors_of(&flow).contains(
            "State \"y\": \"when\" needs output \"missing\" on every step that leads to \"both\""
        ));

        agent_mut(&mut flow, "y").when = Some(Guard::Status {
            is: VisitResult::Completed,
        });
        assert!(
            errors_of(&flow).contains("State \"y\": a branch's \"when\" must be an output guard")
        );

        let mut flow = with_parallel();
        agent_mut(&mut flow, "z").when = changed("changed");
        assert!(errors_of(&flow)
            .contains("State \"z\": only a parallel step's branch can have \"when\""));
    }

    #[test]
    fn a_pilot_flow_may_hold_a_parallel_step() {
        let mut flow = with_parallel();
        flow.owner = Some("p1".into());
        assert_eq!(validate_flow(&flow).errors, Vec::<String>::new());
    }
}
