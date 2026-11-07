//! Task runners: `package.json`, Deno, mise, just, Cargo aliases, a
//! `Procfile` and make. Each lists the names the tool itself would run, as
//! `<tool> <name>`. Only `just` and `make` files are scanned as text.
use super::{in_order, json_keys, json_object, string, toml_keys, toml_object, Source};
use crate::project_actions::{Ctx, Found};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

const NEEDS_ARGUMENTS: &str = "Needs arguments, which Gespan can't pass yet";

/// `scripts` in `package.json`, run with the project's package manager.
pub struct PackageJson;
/// `tasks` in `deno.json` or `deno.jsonc`.
pub struct Deno;
/// `[tasks.<name>]` in mise's config.
pub struct Mise;
/// Recipes in a `justfile`.
pub struct Just;
/// `[alias]` in `.cargo/config.toml`.
pub struct CargoAlias;
/// `name: command` lines, which are long-running processes.
pub struct Procfile;
/// Targets in a `Makefile`.
pub struct Makefile;

/// The package manager the lockfile points to, else `npm`.
pub(crate) fn package_manager(checkout: &Path) -> &'static str {
    let has = |file: &str| checkout.join(file).exists();
    if has("bun.lock") || has("bun.lockb") {
        "bun"
    } else if has("pnpm-lock.yaml") {
        "pnpm"
    } else if has("yarn.lock") {
        "yarn"
    } else {
        "npm"
    }
}

impl Source for PackageJson {
    fn id(&self) -> &'static str {
        "package.json"
    }
    fn files(&self) -> &'static [&'static str] {
        &["package.json"]
    }
    fn parse(&self, ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        let root = json_object(text)?;
        let Some(Value::Object(scripts)) = root.get("scripts") else {
            return Ok(Vec::new());
        };
        Ok(in_order(scripts, &json_keys(text, &["scripts"]))
            .into_iter()
            .filter_map(|(name, body)| {
                let body = body.as_str()?;
                // npm runs `pre<name>` and `post<name>` around `<name>` itself.
                let hook_of = ["pre", "post"]
                    .iter()
                    .filter_map(|prefix| name.strip_prefix(prefix))
                    .any(|base| !base.is_empty() && scripts.contains_key(base));
                if hook_of {
                    return None;
                }
                let mut found = tool(name, ctx.package_manager, &["run"]);
                found.detail = Some(body.to_string()).filter(|body| !body.trim().is_empty());
                Some(found)
            })
            .collect())
    }
}

impl Source for Deno {
    fn id(&self) -> &'static str {
        "deno"
    }
    fn files(&self) -> &'static [&'static str] {
        &["deno.json", "deno.jsonc"]
    }
    fn parse(&self, _ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        let root = json_object(text)?;
        let Some(Value::Object(tasks)) = root.get("tasks") else {
            return Ok(Vec::new());
        };
        Ok(in_order(tasks, &json_keys(text, &["tasks"]))
            .into_iter()
            .filter_map(|(name, task)| {
                let mut found = tool(name, "deno", &["task"]);
                found.detail = match task {
                    Value::String(command) => Some(command.clone()),
                    // A task of only `dependencies` still runs.
                    Value::Object(_) => {
                        string(task, "description").or_else(|| string(task, "command"))
                    }
                    _ => return None,
                };
                Some(found)
            })
            .collect())
    }
}

impl Source for Mise {
    fn id(&self) -> &'static str {
        "mise"
    }
    fn files(&self) -> &'static [&'static str] {
        &["mise.toml", ".mise.toml"]
    }
    fn parse(&self, _ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        let root = toml_object(text)?;
        let Some(Value::Object(tasks)) = root.get("tasks") else {
            return Ok(Vec::new());
        };
        Ok(in_order(tasks, &toml_keys(text, &["tasks"]))
            .into_iter()
            .filter_map(|(name, task)| {
                if !matches!(task, Value::Object(_) | Value::String(_) | Value::Array(_)) {
                    return None;
                }
                let mut found = tool(name, "mise", &["run"]);
                found.detail = string(task, "description");
                found.hide = task.get("hide").and_then(Value::as_bool) == Some(true);
                Some(found)
            })
            .collect())
    }
}

impl Source for Just {
    fn id(&self) -> &'static str {
        "just"
    }
    fn files(&self) -> &'static [&'static str] {
        &["justfile", "Justfile", ".justfile"]
    }
    fn parse(&self, _ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        let mut found: Vec<Found> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        let mut doc: Option<String> = None;
        let mut private = false;
        for line in text.lines() {
            // Recipe bodies are indented.
            if line.starts_with(char::is_whitespace) {
                continue;
            }
            let trimmed = line.trim();
            if let Some(comment) = trimmed.strip_prefix('#') {
                doc = Some(comment.trim().to_string()).filter(|doc| !doc.is_empty());
                continue;
            }
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                private |= trimmed[1..trimmed.len() - 1]
                    .split(',')
                    .any(|attribute| attribute.trim() == "private");
                continue;
            }
            let recipe = just_recipe(line);
            let (doc, private) = (doc.take(), std::mem::take(&mut private));
            let Some((name, needs_arguments)) = recipe else {
                continue;
            };
            if private || name.starts_with('_') || !seen.insert(name) {
                continue;
            }
            let mut recipe = tool(name, "just", &[]);
            recipe.detail = doc;
            recipe.blocked = needs_arguments.then(|| NEEDS_ARGUMENTS.into());
            found.push(recipe);
        }
        Ok(found)
    }
}

/// The recipe a line starts, and whether it needs arguments: a parameter
/// with no default that isn't `*` variadic. Settings, aliases and
/// assignments end in `:=`, so they don't match.
fn just_recipe(line: &str) -> Option<(&str, bool)> {
    let line = line.strip_prefix('@').unwrap_or(line);
    let (name, mut rest) = line.split_at(name_end(line));
    if name.is_empty() {
        return None;
    }
    let mut needs_arguments = false;
    loop {
        let spaced = rest.starts_with(char::is_whitespace);
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix(':') {
            return (!after.starts_with('=')).then_some((name, needs_arguments));
        }
        // `name`, `$name`, `*name` or `+name`, then `=default`.
        if !spaced {
            return None;
        }
        let optional = rest.starts_with('*');
        rest = rest.strip_prefix(['*', '+']).unwrap_or(rest);
        rest = rest.strip_prefix('$').unwrap_or(rest);
        let (parameter, after) = rest.split_at(name_end(rest));
        if parameter.is_empty() {
            return None;
        }
        rest = after;
        match rest.strip_prefix('=') {
            Some(value) => rest = &value[just_value(value)?..],
            None => needs_arguments |= !optional,
        }
    }
}

/// Where a name of letters, digits, `_` and `-` ends.
fn name_end(text: &str) -> usize {
    text.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(text.len())
}

/// The length of a parameter's default: a quoted string, a name, a function
/// call, or an expression in parentheses, which may hold quoted strings.
fn just_value(text: &str) -> Option<usize> {
    for triple in ["'''", "\"\"\""] {
        if let Some(body) = text.strip_prefix(triple) {
            return Some(2 * triple.len() + body.find(triple)?);
        }
    }
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (at, c) in text.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' && open == '"' {
                escaped = true;
            } else if c == open {
                quote = None;
                if depth == 0 {
                    return Some(at + 1);
                }
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => quote = Some(c),
            '(' => depth += 1,
            ')' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(at + 1);
                }
            }
            _ if depth > 0 => {}
            // A variable, or a function call such as `arch()`.
            _ if at == 0 => {
                let name = name_end(text);
                if name == 0 {
                    return None;
                }
                let call = &text[name..];
                let args = if call.starts_with('(') {
                    just_value(call)?
                } else {
                    0
                };
                return Some(name + args);
            }
            _ => return None,
        }
    }
    None
}

impl Source for CargoAlias {
    fn id(&self) -> &'static str {
        "cargo"
    }
    fn files(&self) -> &'static [&'static str] {
        &[".cargo/config.toml"]
    }
    fn parse(&self, _ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        let root = toml_object(text)?;
        let Some(Value::Object(aliases)) = root.get("alias") else {
            return Ok(Vec::new());
        };
        Ok(in_order(aliases, &toml_keys(text, &["alias"]))
            .into_iter()
            .filter_map(|(name, alias)| {
                let expands_to = match alias {
                    Value::String(text) => text.clone(),
                    Value::Array(words) => words
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" "),
                    _ => return None,
                };
                let mut found = tool(name, "cargo", &[]);
                found.detail = Some(expands_to).filter(|text| !text.trim().is_empty());
                Some(found)
            })
            .collect())
    }
}

impl Source for Procfile {
    fn id(&self) -> &'static str {
        "procfile"
    }
    fn files(&self) -> &'static [&'static str] {
        &["Procfile"]
    }
    fn parse(&self, _ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        Ok(text
            .lines()
            .filter_map(|line| {
                let (name, command) = line.split_once(':')?;
                let (name, command) = (name.trim(), command.trim());
                let plain = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
                (plain && !command.is_empty()).then(|| Found {
                    label: name.to_string(),
                    command: command.to_string(),
                    shell: true,
                    background: true,
                    ..Found::default()
                })
            })
            .collect())
    }
}

impl Source for Makefile {
    fn id(&self) -> &'static str {
        "make"
    }
    fn files(&self) -> &'static [&'static str] {
        &["Makefile", "makefile", "GNUmakefile"]
    }
    fn parse(&self, _ctx: &Ctx, _file: &str, text: &str) -> Result<Vec<Found>, String> {
        let mut found: Vec<Found> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for line in text.lines() {
            // Recipe lines start with a tab.
            let Some((name, rest)) = make_target(line) else {
                continue;
            };
            if !seen.insert(name) {
                continue;
            }
            let mut target = tool(name, "make", &[]);
            target.detail = rest
                .split_once("##")
                .map(|(_, doc)| doc.trim().to_string())
                .filter(|doc| !doc.is_empty());
            found.push(target);
        }
        Ok(found)
    }
}

/// The target a line starts and what follows its colon, matching
/// `^([A-Za-z0-9][A-Za-z0-9_.-]*)\s*:(?!=)`. `.PHONY`, `_x` and `%` rules
/// don't start with a letter or digit, and `X := 1` ends in `:=`.
fn make_target(line: &str) -> Option<(&str, &str)> {
    if !line.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return None;
    }
    let end = line
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
        .unwrap_or(line.len());
    let (name, rest) = line.split_at(end);
    let rest = rest.trim_start().strip_prefix(':')?;
    (!rest.starts_with('=')).then_some((name, rest))
}

/// `<program> <subcommand…> <name>`. It isn't a shell line, so the resolver
/// quotes `name` and a name like `a b` or `x;y` stays one word.
fn tool(name: &str, program: &str, subcommand: &[&str]) -> Found {
    let mut args: Vec<String> = subcommand.iter().map(|word| word.to_string()).collect();
    args.push(name.to_string());
    Found {
        label: name.to_string(),
        command: program.to_string(),
        args,
        ..Found::default()
    }
}
