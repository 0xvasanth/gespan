//! A Flow shown as YAML, for reading on demand. Flows are stored as JSON;
//! this view never parses YAML back, so it only has to write it.
use super::definition::Flow;
use serde_json::{Map, Value};

/// One line of a YAML diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffLine {
    Same(String),
    Removed(String),
    Added(String),
}

/// `old` against `new`, line by line, as the Pipeline tab shows a draft
/// against the live version.
pub fn yaml_diff(old: &str, new: &str) -> Vec<DiffLine> {
    similar::TextDiff::from_lines(old, new)
        .iter_all_changes()
        .map(|change| {
            let line = change.value().trim_end_matches('\n').to_owned();
            match change.tag() {
                similar::ChangeTag::Equal => DiffLine::Same(line),
                similar::ChangeTag::Delete => DiffLine::Removed(line),
                similar::ChangeTag::Insert => DiffLine::Added(line),
            }
        })
        .collect()
}

/// Keys in the order a person reads a Flow; any others follow, sorted.
const KEY_ORDER: &[&str] = &[
    "kind",
    "id",
    "name",
    "title",
    "label",
    "version",
    "description",
    "required",
    "default",
    "initial",
    "access",
    "inputs",
    "harness",
    "model",
    "brief",
    "message",
    "outputs",
    "branches",
    "join",
    "maxVisits",
    "states",
    "to",
    "when",
    "transitions",
];

/// Bookkeeping that says nothing about what the Flow does.
const HIDDEN: &[&str] = &["id", "cwd", "owner", "createdAt", "updatedAt"];

/// `flow` as YAML, headed by its name and `version`. The Flow's id, folder,
/// owner and timestamps are left out.
pub fn flow_yaml(flow: &Flow, version: i64) -> String {
    let mut value = serde_json::to_value(flow).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        for key in HIDDEN {
            map.remove(*key);
        }
        map.insert("version".into(), Value::from(version));
    }
    let mut out = String::new();
    write_value(&mut out, &value, 0, false);
    out
}

fn ordered(map: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by_key(|(key, _)| {
        (
            KEY_ORDER
                .iter()
                .position(|k| k == key)
                .unwrap_or(KEY_ORDER.len()),
            (*key).clone(),
        )
    });
    entries
}

/// Writes `value` at `indent`. `inline` is true right after "- " or
/// "key: ", where the first line continues the current one.
fn write_value(out: &mut String, value: &Value, indent: usize, inline: bool) {
    let pad = " ".repeat(indent);
    match value {
        Value::Object(map) if map.is_empty() => out.push_str("{}\n"),
        Value::Object(map) => {
            let mut first = inline;
            for (key, item) in ordered(map) {
                if first {
                    first = false;
                } else {
                    out.push_str(&pad);
                }
                out.push_str(&scalar_key(key));
                out.push(':');
                write_entry(out, item, indent);
            }
        }
        Value::Array(items) if items.is_empty() => out.push_str("[]\n"),
        Value::Array(items) => {
            for (ix, item) in items.iter().enumerate() {
                if ix > 0 || !inline {
                    out.push_str(&pad);
                }
                out.push_str("- ");
                match item {
                    Value::Object(map) if !map.is_empty() => {
                        write_value(out, item, indent + 2, true);
                    }
                    _ => {
                        out.push_str(scalar(item).trim_start());
                        out.push('\n');
                    }
                }
            }
        }
        _ => {
            out.push_str(&scalar(value));
            out.push('\n');
        }
    }
}

/// The value after "key:": inline for scalars and empty collections, on
/// the next lines, indented, for the rest.
fn write_entry(out: &mut String, value: &Value, indent: usize) {
    match value {
        // A short list of names, like a parallel step's branches, reads best
        // on one line, as the board writes it.
        Value::Array(items) if !items.is_empty() && inline_list(value).is_some() => {
            out.push(' ');
            out.push_str(&inline_list(value).unwrap_or_default());
            out.push('\n');
        }
        Value::Object(map) if !map.is_empty() => {
            out.push('\n');
            write_value(out, value, indent + 2, false);
        }
        Value::Array(items) if !items.is_empty() => {
            out.push('\n');
            write_value(out, value, indent + 2, false);
        }
        Value::String(text) if text.contains('\n') => {
            out.push_str(" |\n");
            for line in text.trim_end_matches('\n').lines() {
                if line.is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(&" ".repeat(indent + 2));
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        _ => {
            out.push(' ');
            write_value(out, value, indent, true);
        }
    }
}

/// `[a, b, c]` for a short list of plain names, like a parallel step's
/// branches; `None` for anything else.
fn inline_list(value: &Value) -> Option<String> {
    let items = value.as_array()?;
    let names: Vec<&str> = items
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|s| !s.is_empty() && super::definition::is_state_name(s))
        })
        .collect::<Option<_>>()?;
    let line = format!("[{}]", names.join(", "));
    (line.len() <= 60).then_some(line)
}

fn scalar_key(key: &str) -> String {
    match needs_quotes(key) {
        true => quoted(key),
        false => key.to_owned(),
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) if needs_quotes(s) => quoted(s),
        Value::String(s) => s.clone(),
        // Collections are written by `write_value`.
        _ => String::new(),
    }
}

/// Whether `text` would read as something other than this string.
fn needs_quotes(text: &str) -> bool {
    text.is_empty()
        || text != text.trim()
        || matches!(
            text.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~"
        )
        || text.parse::<f64>().is_ok()
        || text.starts_with([
            '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%',
            '@', '`',
        ])
        || text.contains(": ")
        || text.contains(" #")
        || text.ends_with(':')
        || text.contains('\n')
}

fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::starter_flows;

    #[test]
    fn a_flow_reads_name_first_and_hides_its_bookkeeping() {
        let mut flow = starter_flows().remove(0);
        flow.id = "f-123".into();
        flow.cwd = "/Users/me/project".into();
        flow.owner = Some("pilot-1".into());
        let yaml = flow_yaml(&flow, 3);
        let lines: Vec<&str> = yaml.lines().collect();
        assert!(lines[0].starts_with("name: "), "{yaml}");
        assert_eq!(lines[1], "version: 3");
        for hidden in [
            "f-123",
            "/Users/me/project",
            "pilot-1",
            "createdAt",
            "updatedAt",
        ] {
            assert!(!yaml.contains(hidden), "{hidden} in\n{yaml}");
        }
        assert!(yaml.contains("\nstates:\n  - kind: "), "{yaml}");
        assert!(
            yaml.contains(&format!("\ninitial: {}\n", flow.initial)),
            "{yaml}"
        );
        // Each state lists its transitions under it.
        assert!(yaml.contains("    transitions:\n      - to: "), "{yaml}");
    }

    #[test]
    fn a_draft_diffs_against_the_live_version_line_by_line() {
        let live = starter_flows().remove(0);
        let mut draft = live.clone();
        draft.description = "Only build Linux when main has new commits".into();
        let diff = yaml_diff(&flow_yaml(&live, 1), &flow_yaml(&draft, 2));
        let changed: Vec<_> = diff
            .iter()
            .filter(|l| !matches!(l, DiffLine::Same(_)))
            .collect();
        assert_eq!(
            changed,
            [
                &DiffLine::Removed("version: 1".into()),
                &DiffLine::Removed(format!("description: {}", live.description)),
                &DiffLine::Added("version: 2".into()),
                &DiffLine::Added("description: Only build Linux when main has new commits".into()),
            ],
            "only what changed is marked"
        );
        // Everything else reads as it was, in order.
        let same = diff
            .iter()
            .filter(|l| matches!(l, DiffLine::Same(_)))
            .count();
        assert_eq!(same, flow_yaml(&live, 1).lines().count() - 2);
    }

    #[test]
    fn strings_that_would_read_as_something_else_are_quoted() {
        let value = serde_json::json!({
            "a": "true", "b": "12", "c": "", "d": "key: value", "e": "plain words",
            "f": "- dash", "g": " padded", "h": "say \"hi\"", "i": "\"quoted\" start",
        });
        let mut out = String::new();
        write_value(&mut out, &value, 0, false);
        assert_eq!(
            out,
            "a: \"true\"\nb: \"12\"\nc: \"\"\nd: \"key: value\"\ne: plain words\n\
             f: \"- dash\"\ng: \" padded\"\nh: say \"hi\"\ni: \"\\\"quoted\\\" start\"\n"
        );
    }

    #[test]
    fn long_text_is_a_block_and_short_name_lists_and_empty_collections_stay_inline() {
        let value = serde_json::json!({
            "brief": "Read the changes.\nWrite the plan.\n",
            "outputs": [],
            "when": {},
            "tags": ["one", "two"],
            "notes": ["Read it.", "Ship it."],
        });
        let mut out = String::new();
        write_value(&mut out, &value, 0, false);
        assert_eq!(
            out,
            "brief: |\n  Read the changes.\n  Write the plan.\noutputs: []\nwhen: {}\nnotes:\n  - Read it.\n  - Ship it.\ntags: [one, two]\n"
        );
    }

    #[test]
    fn a_parallel_step_lists_its_branches_on_one_line() {
        let state = crate::flow::FlowState::Parallel(crate::flow::definition::ParallelState {
            id: "build".into(),
            title: "Build & test".into(),
            branches: vec![
                "build_macos".into(),
                "build_windows".into(),
                "build_linux".into(),
            ],
            join: crate::flow::definition::Join::All,
            transitions: vec![],
            max_visits: 1,
        });
        let mut out = String::new();
        write_value(&mut out, &serde_json::to_value(&state).unwrap(), 0, false);
        assert_eq!(
            out,
            "kind: parallel\nid: build\ntitle: Build & test\nbranches: [build_macos, build_windows, build_linux]\njoin: all\nmaxVisits: 1\ntransitions: []\n"
        );
    }
}
