//! `{{inputs.x}}` / `{{states.s.outputs.f}}` binding substitution.
use super::definition::{Flow, OutputValue};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

pub struct BindingContext<'a> {
    pub inputs: &'a HashMap<String, String>,
    /// Latest outputs per state id.
    pub outputs: &'a HashMap<String, HashMap<String, OutputValue>>,
}

pub const MAX_BINDING_CHARS: usize = 8000;
const TRUNCATION_SUFFIX: &str = "\n…[truncated]";

fn binding_pattern() -> &'static Regex {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"\{\{\s*(inputs\.[a-z][a-z0-9_]*|states\.[a-z][a-z0-9_]*\.outputs\.[a-z][a-z0-9_]*)(\?)?\s*\}\}",
        )
        .unwrap()
    });
    &PATTERN
}

fn lookup(reference: &str, ctx: &BindingContext) -> Option<String> {
    let parts: Vec<&str> = reference.split('.').collect();
    let value = if parts[0] == "inputs" {
        return ctx.inputs.get(parts[1]).map(|v| truncate(v.clone()));
    } else {
        ctx.outputs.get(parts[1])?.get(parts[3])?
    };
    Some(truncate(stringify(value)))
}

fn stringify(value: &OutputValue) -> String {
    match value {
        OutputValue::String(s) => s.clone(),
        OutputValue::Number(n) => n.to_string(),
        OutputValue::Bool(b) => b.to_string(),
        OutputValue::List(items) => items
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn truncate(text: String) -> String {
    match text.char_indices().nth(MAX_BINDING_CHARS) {
        Some((cut, _)) => format!("{}{TRUNCATION_SUFFIX}", &text[..cut]),
        None => text,
    }
}

/// Single-pass substitution: substituted values are never re-expanded.
pub fn render_template(text: &str, ctx: &BindingContext) -> Result<String, Vec<String>> {
    let mut errors = Vec::new();
    let rendered = binding_pattern().replace_all(text, |caps: &regex::Captures| {
        let reference = &caps[1];
        match lookup(reference, ctx) {
            Some(value) => value,
            None => {
                if caps.get(2).is_none() {
                    errors.push(format!("Binding {reference} has no value yet"));
                }
                String::new()
            }
        }
    });
    if errors.is_empty() {
        Ok(rendered.into_owned())
    } else {
        Err(errors)
    }
}

/// `(ref, optional)` for every binding, in order of appearance.
pub fn list_bindings(text: &str) -> Vec<(String, bool)> {
    binding_pattern()
        .captures_iter(text)
        .map(|caps| (caps[1].to_string(), caps.get(2).is_some()))
        .collect()
}

/// Applies defaults; `Err` lists the required inputs that are still empty.
pub fn resolve_inputs(
    flow: &Flow,
    given: &HashMap<String, String>,
) -> Result<HashMap<String, String>, Vec<String>> {
    let mut inputs = HashMap::new();
    let mut missing = Vec::new();
    for input in &flow.inputs {
        let value = match given.get(&input.name) {
            Some(v) if !v.is_empty() => v.clone(),
            _ => input.default.clone().unwrap_or_default(),
        };
        if input.required && value.is_empty() {
            missing.push(input.name.clone());
        }
        inputs.insert(input.name.clone(), value);
    }
    if missing.is_empty() {
        Ok(inputs)
    } else {
        Err(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::definition::FlowInput;

    type Outputs = HashMap<String, HashMap<String, OutputValue>>;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn ctx_outputs() -> Outputs {
        HashMap::from([(
            "test".to_string(),
            HashMap::from([
                ("passed".to_string(), OutputValue::Bool(true)),
                ("failures".to_string(), OutputValue::String("boom".into())),
                (
                    "files".to_string(),
                    OutputValue::List(vec!["a".into(), "b".into()]),
                ),
                ("count".to_string(), OutputValue::Number(3.0)),
            ]),
        )])
    }

    fn render(
        text: &str,
        inputs: &[(&str, &str)],
        outputs: &Outputs,
    ) -> Result<String, Vec<String>> {
        let inputs = map(inputs);
        render_template(
            text,
            &BindingContext {
                inputs: &inputs,
                outputs,
            },
        )
    }

    #[test]
    fn substitutes_inputs_and_latest_outputs() {
        let r = render(
            "{{inputs.issue}} / {{states.test.outputs.failures}} / {{ states.test.outputs.count }}",
            &[("issue", "Bug #1")],
            &ctx_outputs(),
        );
        assert_eq!(r, Ok("Bug #1 / boom / 3".to_string()));
    }

    #[test]
    fn renders_booleans_and_string_arrays() {
        let o = ctx_outputs();
        assert_eq!(
            render("{{states.test.outputs.passed}}", &[], &o),
            Ok("true".into())
        );
        assert_eq!(
            render("{{states.test.outputs.files}}", &[], &o),
            Ok("- a\n- b".into())
        );
    }

    #[test]
    fn renders_fractional_numbers() {
        let o: Outputs = HashMap::from([(
            "s".to_string(),
            HashMap::from([("n".to_string(), OutputValue::Number(2.5))]),
        )]);
        assert_eq!(render("{{states.s.outputs.n}}", &[], &o), Ok("2.5".into()));
    }

    #[test]
    fn renders_optional_missing_values_as_empty() {
        let o = ctx_outputs();
        assert_eq!(
            render("x[{{states.nope.outputs.f?}}]", &[], &o),
            Ok("x[]".into())
        );
        assert_eq!(render("x[{{inputs.missing?}}]", &[], &o), Ok("x[]".into()));
    }

    #[test]
    fn optional_bindings_still_render_a_present_value() {
        assert_eq!(
            render("{{states.test.outputs.failures?}}", &[], &ctx_outputs()),
            Ok("boom".into())
        );
    }

    #[test]
    fn errors_on_required_missing_values_collecting_all() {
        let r = render(
            "{{states.test.outputs.nope}} {{states.zz.outputs.failures}}",
            &[],
            &ctx_outputs(),
        );
        assert_eq!(
            r,
            Err(vec![
                "Binding states.test.outputs.nope has no value yet".to_string(),
                "Binding states.zz.outputs.failures has no value yet".to_string(),
            ])
        );
        assert_eq!(
            render("{{states.test.outputs.failures}}", &[], &HashMap::new()),
            Err(vec![
                "Binding states.test.outputs.failures has no value yet".to_string()
            ])
        );
    }

    #[test]
    fn truncates_long_values_at_8000_chars() {
        let big = "x".repeat(9000);
        let r = render("{{inputs.big}}", &[("big", &big)], &HashMap::new());
        assert_eq!(r, Ok(format!("{}\n…[truncated]", "x".repeat(8000))));
    }

    #[test]
    fn does_not_truncate_exactly_8000_chars() {
        let big = "x".repeat(8000);
        let r = render("{{inputs.big}}", &[("big", &big)], &HashMap::new());
        assert_eq!(r, Ok(big));
    }

    #[test]
    fn does_not_re_expand_bindings_inside_values() {
        let r = render(
            "{{inputs.a}}",
            &[("a", "{{inputs.x}}"), ("x", "NO")],
            &HashMap::new(),
        );
        assert_eq!(r, Ok("{{inputs.x}}".into()));
    }

    #[test]
    fn treats_an_empty_string_input_as_present() {
        assert_eq!(
            render("[{{inputs.a}}]", &[("a", "")], &HashMap::new()),
            Ok("[]".into())
        );
    }

    #[test]
    fn leaves_non_matching_braces_alone() {
        assert_eq!(
            render("{{ foo }} {x}", &[], &ctx_outputs()),
            Ok("{{ foo }} {x}".into())
        );
    }

    #[test]
    fn list_bindings_lists_refs_with_optionality_in_order() {
        assert_eq!(
            list_bindings("{{inputs.a}} {{states.t.outputs.f?}} {{inputs.a}}"),
            vec![
                ("inputs.a".to_string(), false),
                ("states.t.outputs.f".to_string(), true),
                ("inputs.a".to_string(), false),
            ]
        );
    }

    #[test]
    fn list_bindings_returns_empty_for_plain_text() {
        assert!(list_bindings("nothing").is_empty());
    }

    fn input(name: &str, required: bool, default: Option<&str>) -> FlowInput {
        FlowInput {
            name: name.into(),
            label: name.to_uppercase(),
            required,
            default: default.map(String::from),
        }
    }

    fn flow_with(inputs: Vec<FlowInput>) -> Flow {
        Flow {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            cwd: String::new(),
            inputs,
            initial: String::new(),
            states: vec![],
            access: "auto".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn four_inputs() -> Flow {
        flow_with(vec![
            input("a", true, None),
            input("b", false, Some("dflt")),
            input("c", true, Some("cd")),
            input("d", true, None),
        ])
    }

    #[test]
    fn resolve_inputs_applies_defaults_and_ignores_unknown_givens() {
        let r = resolve_inputs(
            &four_inputs(),
            &map(&[("a", "1"), ("d", "4"), ("zzz", "9")]),
        );
        assert_eq!(
            r,
            Ok(map(&[("a", "1"), ("b", "dflt"), ("c", "cd"), ("d", "4")]))
        );
    }

    #[test]
    fn resolve_inputs_treats_empty_as_missing_in_declaration_order() {
        let r = resolve_inputs(&four_inputs(), &map(&[("a", ""), ("d", "")]));
        assert_eq!(r, Err(vec!["a".to_string(), "d".to_string()]));
    }

    #[test]
    fn resolve_inputs_optional_without_default_is_empty() {
        let f = flow_with(vec![input("n", false, None)]);
        assert_eq!(resolve_inputs(&f, &HashMap::new()), Ok(map(&[("n", "")])));
    }
}
