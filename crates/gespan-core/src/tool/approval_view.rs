//! What an approval card shows: the request read as a sentence, the command
//! or change it makes, and its other arguments as plain key and value rows,
//! instead of the provider's escaped JSON. Display only: what runs is the
//! provider's own request, which this never changes.
use super::{edit_preview::EditPreview, ToolCall};
use serde_json::Value;

/// Paths longer than this keep their first and last parts around an ellipsis.
const MAX_PATH_CHARS: usize = 56;
/// Fields a command's own row or the caption already shows, or that only
/// describe a finished run.
const COMMAND_FIELDS: &[&str] = &[
    "command",
    "cmd",
    "script",
    "description",
    "cwd",
    "workingDirectory",
    "working_directory",
    "commandActions",
    "parsed_cmd",
    "parsedCmd",
    "exitCode",
    "durationMs",
];
/// An edit's path, which its own row shows.
const EDIT_PATH_FIELDS: &[&str] = &["file_path", "path", "filePath", "target_file", "targetFile"];
/// An edit's texts, which its short diff stands in for until the card opens.
const EDIT_FIELDS: &[&str] = &[
    "old_string",
    "new_string",
    "oldText",
    "newText",
    "old_text",
    "new_text",
    "oldString",
    "newString",
    "content",
    "contents",
    "edits",
    "diff",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalView {
    /// What the agent says it is doing, or a caption such as "Read notes.md".
    pub summary: Option<String>,
    /// An MCP tool's server and tool, which its name runs together.
    pub mcp: Option<McpTool>,
    pub body: Body,
    /// Where a command runs, cut in the middle when long.
    pub cwd: Option<String>,
    /// The whole folder, when `cwd` had to be cut. Shown once the card opens.
    pub cwd_full: Option<String>,
    /// The arguments the body does not show.
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpTool {
    pub server: String,
    pub tool: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub key: String,
    /// Strings as they are, with real line breaks; anything nested indented.
    pub value: String,
    /// Shown only once the card opens: an edit's texts, which its short
    /// diff stands in for until then.
    pub folded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    None,
    /// The shell command, unwrapped from a launcher such as `bash -lc`.
    Command(String),
    /// A read, list or search: what it does, and its file, folder or pattern.
    Target {
        verb: String,
        target: String,
    },
    Url(String),
    /// An Edit or Write: the file and, when the call carries it, the change.
    Change {
        path: String,
        preview: Option<EditPreview>,
    },
    /// Free text from a provider that sends no structured call.
    Text(String),
}

/// The card for an approval. `tool` is the call when the provider names one;
/// otherwise `detail` is all there is. `cwd` and `home` shorten paths.
pub fn approval_view(
    tool: Option<&ToolCall>,
    detail: &str,
    cwd: Option<&str>,
    home: Option<&str>,
) -> ApprovalView {
    match tool {
        Some(call) => tool_view(call, cwd, home),
        None => detail_view(detail),
    }
}

fn tool_view(call: &ToolCall, cwd: Option<&str>, home: Option<&str>) -> ApprovalView {
    let input = arguments(call);
    let shown = call.presentation_in(cwd);
    // Only a name the provider gives a tool is read as MCP. ACP providers
    // (Cursor, Kiro, ...) name a call by a free-text title with its own kind,
    // which must not pass for one.
    let mcp = mcp_tool(&call.name).filter(|_| call.display.is_none());
    let mut view = ApprovalView {
        summary: None,
        mcp: mcp.clone(),
        body: Body::None,
        cwd: None,
        cwd_full: None,
        fields: Vec::new(),
    };
    if let Some(command) = shown.command.as_deref().filter(|_| mcp.is_none()) {
        let declared = string_field(&input, &["cwd", "workingDirectory", "working_directory"]);
        let (dir, command) = match split_leading_cd(command) {
            Some((dir, rest)) => (Some(dir), rest.to_owned()),
            None => (declared, command.to_owned()),
        };
        view.summary = shown.description.filter(|d| !d.trim().is_empty());
        view.body = Body::Command(command);
        // The session's own folder goes without saying.
        let same = |dir: &str| {
            cwd.is_some_and(|cwd| cwd.trim_end_matches('/') == dir.trim_end_matches('/'))
        };
        if let Some(dir) = dir.filter(|dir| !same(dir)) {
            let full = short_path(&dir, cwd, home);
            let cut = middle_ellipsis(&full);
            view.cwd_full = (cut != full).then_some(full);
            view.cwd = Some(cut);
        }
        view.fields = fields(&input, COMMAND_FIELDS, cwd, home);
        return view;
    }
    if let Some(url) = string_field(&input, &["url", "uri"]).filter(|url| url.contains("://")) {
        if mcp.is_none() && !url.starts_with("file://") {
            view.body = Body::Url(url);
            view.fields = fields(&input, &["url", "uri"], cwd, home);
            return view;
        }
    }
    if mcp.is_none() {
        match shown.action.as_str() {
            "Edit" | "Write" => {
                let path = shown.file_path.as_deref().unwrap_or(&shown.target);
                view.body = Body::Change {
                    path: short_path(path, cwd, home),
                    preview: call.edit_preview().filter(EditPreview::shows_change),
                };
                view.fields = fields(&input, EDIT_PATH_FIELDS, cwd, home);
                for field in &mut view.fields {
                    field.folded = EDIT_FIELDS.contains(&field.key.as_str());
                }
                return view;
            }
            "Read" | "List" | "Find" | "Search" if !shown.target.is_empty() => {
                let search = matches!(shown.action.as_str(), "Find" | "Search");
                let pattern = string_field(&input, &["pattern", "glob", "glob_pattern"])
                    .filter(|_| shown.action == "List");
                let target = match pattern {
                    Some(pattern) => pattern,
                    None if search => shown.target.clone(),
                    None => short_path(
                        shown.file_path.as_deref().unwrap_or(&shown.target),
                        cwd,
                        home,
                    ),
                };
                // The target's own field is in the row already; a search's
                // folder is not, so it stays a field.
                let shown_values = [
                    Some(shown.target.as_str()),
                    shown.file_path.as_deref().filter(|_| !search),
                ];
                let skip: Vec<&str> = input
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(_, value)| {
                        let text = value.as_str();
                        text.is_some()
                            && (shown_values.contains(&text) || text == Some(target.as_str()))
                    })
                    .map(|(key, _)| key.as_str())
                    .collect();
                view.fields = fields(&input, &skip, cwd, home);
                view.body = Body::Target {
                    verb: shown.action.clone(),
                    target,
                };
                return view;
            }
            _ => {}
        }
    }
    // An unrecognized or MCP tool: what Gespan's captions make of it, then
    // its arguments, whatever they are.
    let caption = format!("{} {}", shown.action, shown.target);
    if shown.action != call.name && !shown.action.is_empty() {
        view.summary = Some(caption.trim().to_owned());
    }
    match &input {
        Value::Object(_) => view.fields = fields(&input, &[], cwd, home),
        Value::Null => {}
        Value::String(text) => view.body = Body::Text(text.clone()),
        other => view.body = Body::Text(pretty(other, 0)),
    }
    view
}

/// What runs: a desktop envelope's `rawInput`, or the input itself. Input
/// that is not JSON stays the string it is.
fn arguments(call: &ToolCall) -> Value {
    let Ok(value) = serde_json::from_str::<Value>(&call.input) else {
        return if call.input.trim().is_empty() {
            Value::Null
        } else {
            Value::String(call.input.clone())
        };
    };
    if value["__desktop"].is_object() {
        return value
            .get("rawInput")
            .filter(|raw| !raw.is_null())
            .cloned()
            .unwrap_or(Value::Null);
    }
    value
}

/// A provider that sends only text: JSON becomes key and value rows, and
/// anything else is shown as written.
fn detail_view(detail: &str) -> ApprovalView {
    let mut view = ApprovalView {
        summary: None,
        mcp: None,
        body: Body::None,
        cwd: None,
        cwd_full: None,
        fields: Vec::new(),
    };
    let detail = detail.trim();
    match serde_json::from_str::<Value>(detail) {
        Ok(value @ Value::Object(_)) => view.fields = fields(&value, &[], None, None),
        Ok(Value::String(text)) => view.body = Body::Text(text),
        Ok(value @ Value::Array(_)) => view.body = Body::Text(pretty(&value, 0)),
        _ if detail.is_empty() => {}
        _ => view.body = Body::Text(detail.replace("\r\n", "\n")),
    }
    view
}

/// Claude's `mcp__server__tool`, Codex's `server:tool` and Kiro's
/// `@server/tool`. A server name may itself hold single underscores.
pub fn mcp_tool(name: &str) -> Option<McpTool> {
    let (server, tool) = if let Some(rest) = name.strip_prefix("mcp__") {
        rest.split_once("__")?
    } else if let Some(rest) = name.strip_prefix('@') {
        rest.split_once('/')?
    } else {
        name.split_once(':')?
    };
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
    };
    (valid(server) && valid(tool)).then(|| McpTool {
        server: server.into(),
        tool: tool.into(),
    })
}

fn string_field(input: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| input[*key].as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

/// Every argument but `skip`, in the provider's order. A path argument is
/// shortened like any other path on the card.
fn fields(input: &Value, skip: &[&str], cwd: Option<&str>, home: Option<&str>) -> Vec<Field> {
    input
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !skip.contains(&key.as_str()))
        .map(|(key, value)| Field {
            key: key.clone(),
            value: match value.as_str() {
                Some(path) if is_path_field(key, path) => short_path(path, cwd, home),
                _ => pretty(value, 0),
            },
            folded: false,
        })
        .collect()
}

fn is_path_field(key: &str, value: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let named = key.contains("path")
        || key.ends_with("file")
        || key.ends_with("dir")
        || matches!(key.as_str(), "cwd" | "directory" | "folder" | "root");
    named && !value.contains('\n') && (value.starts_with('/') || value.starts_with("file://"))
}

/// `value` as a person reads it: strings without quotes or escapes, lists
/// one item per line, and objects as indented `key: value` lines.
pub fn pretty(value: &Value, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    match value {
        Value::String(text) => text.replace("\r\n", "\n"),
        Value::Null => "null".into(),
        Value::Bool(_) | Value::Number(_) => value.to_string(),
        Value::Array(items) if items.is_empty() => "[]".into(),
        Value::Object(fields) if fields.is_empty() => "{}".into(),
        Value::Array(items) if items.iter().all(is_short_scalar) => items
            .iter()
            .map(|item| pretty(item, 0))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                let shown = pretty(item, depth + 1);
                let shown = shown.trim_start();
                format!("{indent}- {shown}")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| {
                let shown = pretty(value, depth + 1);
                if is_nested(value) || shown.contains('\n') {
                    let inner = "  ".repeat(depth + 1);
                    let block = if is_nested(value) {
                        shown
                    } else {
                        shown
                            .lines()
                            .map(|line| format!("{inner}{line}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    format!("{indent}{key}:\n{block}")
                } else {
                    format!("{indent}{key}: {shown}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn is_nested(value: &Value) -> bool {
    match value {
        Value::Object(fields) => !fields.is_empty(),
        Value::Array(items) => !items.is_empty() && !items.iter().all(is_short_scalar),
        _ => false,
    }
}

fn is_short_scalar(value: &Value) -> bool {
    match value {
        Value::String(text) => text.chars().count() <= 40 && !text.contains('\n'),
        Value::Object(_) | Value::Array(_) => false,
        _ => true,
    }
}

/// `cd <dir> && rest` as `(dir, rest)`, so the folder reads apart from what
/// runs in it. Only a plain `cd` with one argument joined by `&&` counts.
pub fn split_leading_cd(command: &str) -> Option<(String, &str)> {
    let command = command.trim_start();
    if !command.starts_with("cd ") {
        return None;
    }
    let bytes = command.as_bytes();
    let (mut quote, mut escaped) = (None, false);
    for (at, &ch) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
        } else if ch == b'\\' && quote != Some(b'\'') {
            escaped = true;
        } else if matches!(ch, b'\'' | b'"') {
            match quote {
                Some(open) if open == ch => quote = None,
                None => quote = Some(ch),
                _ => {}
            }
        } else if quote.is_none() {
            if matches!(ch, b'\n' | b';' | b'|' | b'`' | b'$' | b'(' | b'<' | b'>') {
                return None;
            }
            if ch == b'&' {
                if bytes.get(at + 1) != Some(&b'&') {
                    return None;
                }
                // Only a folder the card can show exactly as the shell reads
                // it: no expansion, substitution or escapes.
                if command[..at].contains(['$', '`', '\\']) {
                    return None;
                }
                let args = shlex::split(&command[..at])?;
                let rest = command[at + 2..].trim();
                return match args.as_slice() {
                    [cd, dir] if cd == "cd" && !dir.is_empty() && !rest.is_empty() => {
                        Some((dir.clone(), rest))
                    }
                    _ => None,
                };
            }
        }
    }
    None
}

/// `path` relative to the session folder, or under `~` for the home folder.
/// Never cut: a path's every part can matter.
pub fn short_path(path: &str, cwd: Option<&str>, home: Option<&str>) -> String {
    let path = path.strip_prefix("file://").unwrap_or(path);
    let trimmed = |dir: &str| dir.trim_end_matches(['/', '\\']).to_owned();
    let relative = cwd
        .map(trimmed)
        .filter(|dir| !dir.is_empty())
        .and_then(|dir| {
            if path == dir {
                return Some(".".to_owned());
            }
            path.strip_prefix(&dir)
                .and_then(|rest| rest.strip_prefix('/').or_else(|| rest.strip_prefix('\\')))
                .filter(|rest| !rest.is_empty())
                .map(str::to_owned)
        });
    relative.unwrap_or_else(|| {
        home.map(trimmed)
            .filter(|dir| !dir.is_empty())
            .and_then(|home| {
                if path == home {
                    return Some("~".to_owned());
                }
                path.strip_prefix(&home)
                    .filter(|rest| rest.starts_with('/'))
                    .map(|rest| format!("~{rest}"))
            })
            .unwrap_or_else(|| path.to_owned())
    })
}

/// A long folder cut in the middle for a one-line row. A path that steps
/// through `.` or `..` stays whole, since the cut could hide where it goes.
pub fn middle_ellipsis(path: &str) -> String {
    if path.chars().count() <= MAX_PATH_CHARS
        || path
            .split(['/', '\\'])
            .any(|part| part == ".." || part == ".")
    {
        return path.to_owned();
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 4 {
        // An absolute path's first part is the empty one before `/`.
        let lead = if parts[0].is_empty() { 3 } else { 2 };
        let head = parts[..lead].join("/");
        let tail = parts[parts.len() - 2..].join("/");
        let joined = format!("{head}/…/{tail}");
        if joined.chars().count() <= MAX_PATH_CHARS {
            return joined;
        }
        let last = parts[parts.len() - 1];
        let joined = format!("{head}/…/{last}");
        if joined.chars().count() <= MAX_PATH_CHARS {
            return joined;
        }
        let joined = format!("…/{last}");
        if joined.chars().count() <= MAX_PATH_CHARS {
            return joined;
        }
    }
    let chars: Vec<char> = path.chars().collect();
    let keep = (MAX_PATH_CHARS - 1) / 2;
    let head: String = chars[..keep].iter().collect();
    let tail: String = chars[chars.len() - keep..].iter().collect();
    format!("{head}…{tail}")
}

/// Text cut to fit a collapsed card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collapsed {
    pub text: String,
    /// Lines left out, counting a cut line's remainder as one.
    pub hidden_lines: usize,
}

/// The first `max_lines` lines of `text`, with no line past `max_chars`, or
/// `None` when all of it already fits.
pub fn collapse(text: &str, max_lines: usize, max_chars: usize) -> Option<Collapsed> {
    let lines: Vec<&str> = text.lines().collect();
    let long = |line: &&str| line.chars().count() > max_chars;
    if lines.len() <= max_lines && !lines.iter().any(long) {
        return None;
    }
    let mut hidden = lines.len().saturating_sub(max_lines);
    let shown: Vec<String> = lines
        .iter()
        .take(max_lines)
        .map(|line| {
            if long(line) {
                hidden += 1;
                let mut cut: String = line.chars().take(max_chars.saturating_sub(1)).collect();
                cut.push('…');
                cut
            } else {
                (*line).to_owned()
            }
        })
        .collect();
    Some(Collapsed {
        text: shown.join("\n"),
        hidden_lines: hidden,
    })
}

#[cfg(test)]
mod tests;
