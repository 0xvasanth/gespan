//! The JSON output contract an agent state must meet, and transition selection.
use super::definition::{
    FlowState, Guard, GuardOp, OutputField, OutputType, OutputValue, Transition, VisitResult,
};
use std::collections::HashMap;
use std::sync::LazyLock;

fn type_name(t: OutputType) -> &'static str {
    match t {
        OutputType::String => "string",
        OutputType::Number => "number",
        OutputType::Boolean => "boolean",
        OutputType::StringList => "string[]",
    }
}

pub fn contract_footer(outputs: &[OutputField]) -> String {
    if outputs.is_empty() {
        return String::new();
    }
    let mut footer = String::from(
        "---\nWhen you are done, end your reply with one fenced ```json block containing exactly these fields:",
    );
    for o in outputs {
        footer.push_str(&format!(
            "\n- {} ({}): {}",
            o.name,
            type_name(o.r#type),
            o.description
        ));
    }
    footer.push_str("\nDo not add other fields.");
    footer
}

pub fn retry_prompt(errors: &[String]) -> String {
    format!(
        "Your JSON block was invalid: {}. Reply with only the corrected ```json block.",
        errors.join("; ")
    )
}

fn last_json_block(reply: &str) -> Option<&str> {
    static FENCE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?s)```json\s*\n(.*?)```").unwrap());
    FENCE
        .captures_iter(reply)
        .last()
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
}

fn coerce(value: &serde_json::Value, t: OutputType) -> Option<OutputValue> {
    use serde_json::Value;
    match (t, value) {
        (OutputType::String, Value::String(s)) => Some(OutputValue::String(s.clone())),
        (OutputType::Number, Value::Number(n)) => n.as_f64().map(OutputValue::Number),
        (OutputType::Boolean, Value::Bool(b)) => Some(OutputValue::Bool(*b)),
        (OutputType::StringList, Value::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .map(OutputValue::List),
        _ => None,
    }
}

/// Reads the last ```json block of `reply` and checks it against `outputs`.
pub fn parse_outputs(
    reply: &str,
    outputs: &[OutputField],
) -> Result<HashMap<String, OutputValue>, Vec<String>> {
    let Some(block) = last_json_block(reply) else {
        return if outputs.is_empty() {
            Ok(HashMap::new())
        } else {
            Err(vec!["No ```json block found".into()])
        };
    };
    let value: serde_json::Value =
        serde_json::from_str(block.trim()).map_err(|e| vec![format!("Invalid JSON: {e}")])?;
    let Some(object) = value.as_object() else {
        return Err(vec!["Expected a JSON object".into()]);
    };
    let mut errors = Vec::new();
    let mut values = HashMap::new();
    for o in outputs {
        match object.get(&o.name) {
            None => errors.push(format!("missing field {}", o.name)),
            Some(v) => match coerce(v, o.r#type) {
                Some(parsed) => {
                    values.insert(o.name.clone(), parsed);
                }
                None => errors.push(format!("field {} must be {}", o.name, type_name(o.r#type))),
            },
        }
    }
    for key in object.keys() {
        if !outputs.iter().any(|o| &o.name == key) {
            errors.push(format!("unexpected field {key}"));
        }
    }
    if errors.is_empty() {
        Ok(values)
    } else {
        Err(errors)
    }
}

fn truthy(v: &OutputValue) -> bool {
    match v {
        OutputValue::Bool(b) => *b,
        OutputValue::String(s) => !s.is_empty(),
        OutputValue::Number(n) => *n != 0.0,
        OutputValue::List(l) => !l.is_empty(),
    }
}

/// JSON equality, except numbers compare numerically (2 equals 2.0).
fn equals(v: &OutputValue, want: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (v, want) {
        (OutputValue::String(a), Value::String(b)) => a == b,
        (OutputValue::Bool(a), Value::Bool(b)) => a == b,
        (OutputValue::Number(a), Value::Number(b)) => b.as_f64() == Some(*a),
        (OutputValue::List(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| y.as_str() == Some(x))
        }
        _ => false,
    }
}

fn contains(v: &OutputValue, want: &serde_json::Value) -> bool {
    match (v, want.as_str()) {
        (OutputValue::String(s), Some(w)) => s.contains(w),
        (OutputValue::List(l), Some(w)) => l.iter().any(|x| x == w),
        _ => false,
    }
}

fn matches(
    guard: &Option<Guard>,
    result: VisitResult,
    values: &HashMap<String, OutputValue>,
) -> bool {
    match guard {
        None => result == VisitResult::Completed,
        Some(Guard::Status { is }) => *is == result,
        Some(Guard::Output { field, op, value }) => {
            if result != VisitResult::Completed {
                return false;
            }
            let Some(v) = values.get(field) else {
                return false;
            };
            match (op, value) {
                (GuardOp::Truthy, _) => truthy(v),
                (GuardOp::Falsy, _) => !truthy(v),
                (GuardOp::Eq, Some(w)) => equals(v, w),
                (GuardOp::Neq, Some(w)) => !equals(v, w),
                (GuardOp::Contains, Some(w)) => contains(v, w),
                _ => false,
            }
        }
    }
}

/// The first transition whose guard matches. A failed visit only follows
/// `status: failed` guards; unguarded transitions need a completed visit.
pub fn select_transition<'a>(
    state: &'a FlowState,
    result: VisitResult,
    values: &HashMap<String, OutputValue>,
) -> Option<&'a Transition> {
    state
        .transitions()
        .iter()
        .find(|t| matches(&t.when, result, values))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::definition::{AgentState, ApprovalState};
    use serde_json::json;

    fn field(name: &str, t: OutputType) -> OutputField {
        OutputField {
            name: name.into(),
            r#type: t,
            description: format!("the {name}"),
        }
    }
    fn errs(r: Result<HashMap<String, OutputValue>, Vec<String>>) -> Vec<String> {
        r.unwrap_err()
    }
    fn tr(to: &str, when: Option<Guard>) -> Transition {
        Transition {
            to: to.into(),
            when,
        }
    }
    fn out(field: &str, op: GuardOp, value: Option<serde_json::Value>) -> Option<Guard> {
        Some(Guard::Output {
            field: field.into(),
            op,
            value,
        })
    }
    fn agent(transitions: Vec<Transition>) -> FlowState {
        FlowState::Agent(AgentState {
            id: "a".into(),
            title: "A".into(),
            harness: "claude".into(),
            model: "m".into(),
            brief: "b".into(),
            outputs: vec![],
            transitions,
            max_visits: 3,
        })
    }
    fn vals(pairs: &[(&str, OutputValue)]) -> HashMap<String, OutputValue> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }
    fn pick(state: &FlowState, r: VisitResult, v: &HashMap<String, OutputValue>) -> Option<String> {
        select_transition(state, r, v).map(|t| t.to.clone())
    }

    #[test]
    fn footer_matches_exact_copy() {
        let outputs = vec![
            field("passed", OutputType::Boolean),
            field("files", OutputType::StringList),
        ];
        assert_eq!(
            contract_footer(&outputs),
            "---\nWhen you are done, end your reply with one fenced ```json block containing exactly these fields:\n- passed (boolean): the passed\n- files (string[]): the files\nDo not add other fields."
        );
    }
    #[test]
    fn footer_is_empty_without_outputs() {
        assert_eq!(contract_footer(&[]), "");
    }
    #[test]
    fn retry_prompt_joins_errors() {
        assert_eq!(
            retry_prompt(&["a".into(), "b".into()]),
            "Your JSON block was invalid: a; b. Reply with only the corrected ```json block."
        );
    }

    #[test]
    fn last_json_block_wins_and_other_fences_are_ignored() {
        let outputs = vec![field("passed", OutputType::Boolean)];
        let reply = "```json\n{\"passed\": false}\n```\ntext\n```js\n{\"passed\": 1}\n```\n```\n{\"x\":1}\n```\n```json\n{\"passed\": true}\n```\n```js\nlet a;\n```";
        let got = parse_outputs(reply, &outputs).unwrap();
        assert_eq!(got["passed"], OutputValue::Bool(true));
    }
    #[test]
    fn missing_block_is_reported() {
        let outputs = vec![field("passed", OutputType::Boolean)];
        let e = errs(parse_outputs("```js\n{}\n```\n```\n{}\n```", &outputs));
        assert_eq!(e, vec!["No ```json block found".to_string()]);
    }
    #[test]
    fn invalid_json_is_reported() {
        let outputs = vec![field("passed", OutputType::Boolean)];
        let e = errs(parse_outputs("```json\n{nope}\n```", &outputs));
        assert_eq!(e.len(), 1);
        assert!(e[0].starts_with("Invalid JSON"), "{e:?}");
    }
    #[test]
    fn contract_violations_are_all_listed() {
        let outputs = vec![
            field("passed", OutputType::Boolean),
            field("files", OutputType::StringList),
            field("count", OutputType::Number),
            field("name", OutputType::String),
        ];
        let reply =
            "```json\n{\"files\": [\"a\", 2], \"count\": \"3\", \"name\": \"n\", \"x\": 1}\n```";
        let e = errs(parse_outputs(reply, &outputs));
        assert!(e.contains(&"missing field passed".to_string()), "{e:?}");
        assert!(e.contains(&"unexpected field x".to_string()), "{e:?}");
        assert!(
            e.contains(&"field files must be string[]".to_string()),
            "{e:?}"
        );
        assert!(
            e.contains(&"field count must be number".to_string()),
            "{e:?}"
        );
        assert_eq!(e.len(), 4);
    }
    #[test]
    fn wrong_scalar_type_is_reported() {
        let outputs = vec![field("passed", OutputType::Boolean)];
        let e = errs(parse_outputs(
            "```json\n{\"passed\": \"yes\"}\n```",
            &outputs,
        ));
        assert_eq!(e, vec!["field passed must be boolean".to_string()]);
    }
    #[test]
    fn non_object_json_is_reported() {
        let outputs = vec![field("passed", OutputType::Boolean)];
        for body in ["[1]", "true", "null", "\"s\""] {
            let e = errs(parse_outputs(&format!("```json\n{body}\n```"), &outputs));
            assert_eq!(e, vec!["Expected a JSON object".to_string()], "{body}");
        }
    }
    #[test]
    fn all_types_parse() {
        let outputs = vec![
            field("s", OutputType::String),
            field("n", OutputType::Number),
            field("b", OutputType::Boolean),
            field("l", OutputType::StringList),
        ];
        let reply = "```json\n{\"s\":\"hi\",\"n\":2,\"b\":false,\"l\":[\"a\",\"b\"]}\n```";
        let got = parse_outputs(reply, &outputs).unwrap();
        assert_eq!(got["s"], OutputValue::String("hi".into()));
        assert_eq!(got["n"], OutputValue::Number(2.0));
        assert_eq!(got["b"], OutputValue::Bool(false));
        assert_eq!(got["l"], OutputValue::List(vec!["a".into(), "b".into()]));
        assert_eq!(got.len(), 4);
    }
    #[test]
    fn zero_outputs_without_block_is_ok_but_extra_fields_are_not() {
        assert_eq!(parse_outputs("done", &[]).unwrap(), HashMap::new());
        let e = errs(parse_outputs("```json\n{\"x\": 1}\n```", &[]));
        assert_eq!(e, vec!["unexpected field x".to_string()]);
    }

    #[test]
    fn first_match_wins_and_unguarded_always_matches() {
        let s = agent(vec![
            tr("one", out("n", GuardOp::Eq, Some(json!(1)))),
            tr("two", None),
            tr("three", None),
        ]);
        assert_eq!(
            pick(
                &s,
                VisitResult::Completed,
                &vals(&[("n", OutputValue::Number(1.0))])
            )
            .as_deref(),
            Some("one")
        );
        assert_eq!(
            pick(
                &s,
                VisitResult::Completed,
                &vals(&[("n", OutputValue::Number(9.0))])
            )
            .as_deref(),
            Some("two")
        );
    }
    #[test]
    fn eq_and_neq_use_json_equality_with_numeric_comparison() {
        let s = agent(vec![
            tr("eq", out("n", GuardOp::Eq, Some(json!(2)))),
            tr("neq", out("n", GuardOp::Neq, Some(json!(2)))),
        ]);
        let two = vals(&[("n", OutputValue::Number(2.0))]);
        let three = vals(&[("n", OutputValue::Number(3.0))]);
        assert_eq!(
            pick(&s, VisitResult::Completed, &two).as_deref(),
            Some("eq")
        );
        assert_eq!(
            pick(&s, VisitResult::Completed, &three).as_deref(),
            Some("neq")
        );
        let s = agent(vec![tr("x", out("n", GuardOp::Eq, Some(json!(2.5))))]);
        assert_eq!(
            pick(
                &s,
                VisitResult::Completed,
                &vals(&[("n", OutputValue::Number(2.5))])
            )
            .as_deref(),
            Some("x")
        );
    }
    #[test]
    fn eq_compares_strings_and_bools_without_coercion() {
        let s = agent(vec![
            tr("b", out("v", GuardOp::Eq, Some(json!(true)))),
            tr("s", out("v", GuardOp::Eq, Some(json!("true")))),
        ]);
        assert_eq!(
            pick(
                &s,
                VisitResult::Completed,
                &vals(&[("v", OutputValue::Bool(true))])
            )
            .as_deref(),
            Some("b")
        );
        assert_eq!(
            pick(
                &s,
                VisitResult::Completed,
                &vals(&[("v", OutputValue::String("true".into()))])
            )
            .as_deref(),
            Some("s")
        );
        assert_eq!(
            pick(
                &s,
                VisitResult::Completed,
                &vals(&[("v", OutputValue::Number(1.0))])
            ),
            None
        );
    }
    #[test]
    fn truthy_and_falsy_treat_empty_list_as_falsy() {
        let s = agent(vec![
            tr("t", out("v", GuardOp::Truthy, None)),
            tr("f", out("v", GuardOp::Falsy, None)),
        ]);
        let go = |v: OutputValue| pick(&s, VisitResult::Completed, &vals(&[("v", v)]));
        assert_eq!(go(OutputValue::List(vec![])).as_deref(), Some("f"));
        assert_eq!(
            go(OutputValue::List(vec!["a".into()])).as_deref(),
            Some("t")
        );
        assert_eq!(go(OutputValue::Bool(false)).as_deref(), Some("f"));
        assert_eq!(go(OutputValue::Bool(true)).as_deref(), Some("t"));
        assert_eq!(go(OutputValue::String(String::new())).as_deref(), Some("f"));
        assert_eq!(go(OutputValue::String("x".into())).as_deref(), Some("t"));
        assert_eq!(go(OutputValue::Number(0.0)).as_deref(), Some("f"));
        assert_eq!(go(OutputValue::Number(2.0)).as_deref(), Some("t"));
    }
    #[test]
    fn contains_covers_strings_and_lists() {
        let s = agent(vec![tr(
            "c",
            out("v", GuardOp::Contains, Some(json!("bug"))),
        )]);
        let go = |v: OutputValue| pick(&s, VisitResult::Completed, &vals(&[("v", v)]));
        assert_eq!(
            go(OutputValue::String("a bug here".into())).as_deref(),
            Some("c")
        );
        assert_eq!(go(OutputValue::String("fine".into())).as_deref(), None);
        assert_eq!(
            go(OutputValue::List(vec!["x".into(), "bug".into()])).as_deref(),
            Some("c")
        );
        assert_eq!(go(OutputValue::List(vec!["bugs".into()])).as_deref(), None);
    }
    #[test]
    fn missing_output_never_matches_an_output_guard() {
        let s = agent(vec![
            tr("neq", out("v", GuardOp::Neq, Some(json!(1)))),
            tr("falsy", out("v", GuardOp::Falsy, None)),
        ]);
        assert_eq!(pick(&s, VisitResult::Completed, &HashMap::new()), None);
    }
    #[test]
    fn failed_result_only_considers_failed_status_guards() {
        let s = agent(vec![
            tr("unguarded", None),
            tr(
                "done",
                Some(Guard::Status {
                    is: VisitResult::Completed,
                }),
            ),
            tr("out", out("v", GuardOp::Truthy, None)),
            tr(
                "failed",
                Some(Guard::Status {
                    is: VisitResult::Failed,
                }),
            ),
        ]);
        let v = vals(&[("v", OutputValue::Bool(true))]);
        assert_eq!(pick(&s, VisitResult::Failed, &v).as_deref(), Some("failed"));
        assert_eq!(
            pick(&s, VisitResult::Completed, &v).as_deref(),
            Some("unguarded")
        );
        let s = agent(vec![tr("unguarded", None)]);
        assert_eq!(pick(&s, VisitResult::Failed, &v), None);
    }
    #[test]
    fn completed_result_skips_failed_status_guards() {
        let s = agent(vec![
            tr(
                "failed",
                Some(Guard::Status {
                    is: VisitResult::Failed,
                }),
            ),
            tr(
                "done",
                Some(Guard::Status {
                    is: VisitResult::Completed,
                }),
            ),
        ]);
        assert_eq!(
            pick(&s, VisitResult::Completed, &HashMap::new()).as_deref(),
            Some("done")
        );
    }
    #[test]
    fn no_match_returns_none() {
        let s = agent(vec![tr("x", out("v", GuardOp::Eq, Some(json!("a"))))]);
        assert_eq!(
            pick(
                &s,
                VisitResult::Completed,
                &vals(&[("v", OutputValue::String("b".into()))])
            ),
            None
        );
        assert_eq!(
            pick(&agent(vec![]), VisitResult::Completed, &HashMap::new()),
            None
        );
    }
    #[test]
    fn approval_states_select_on_approved() {
        let s = FlowState::Approval(ApprovalState {
            id: "r".into(),
            title: "R".into(),
            message: "m".into(),
            transitions: vec![
                tr("yes", out("approved", GuardOp::Truthy, None)),
                tr("no", None),
            ],
            max_visits: 1,
        });
        assert_eq!(
            pick(
                &s,
                VisitResult::Completed,
                &vals(&[("approved", OutputValue::Bool(true))])
            )
            .as_deref(),
            Some("yes")
        );
        assert_eq!(
            pick(
                &s,
                VisitResult::Completed,
                &vals(&[("approved", OutputValue::Bool(false))])
            )
            .as_deref(),
            Some("no")
        );
    }
}
