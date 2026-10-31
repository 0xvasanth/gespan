//! Adds MCP servers to the agents' own config files.
//!
//! A server's `env` and `headers` values are usually credentials, and anything
//! on a command line can be read by every local process. So this crate writes
//! the files itself and never spawns anything: no agent CLI sees a server's
//! configuration as an argument.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

/// How long to wait for a running Claude Code to finish saving its config.
const CLAUDE_LOCK_WAIT: Duration = Duration::from_secs(5);
/// Claude's lock library refreshes a held lock well inside this age.
const CLAUDE_LOCK_STALE: Duration = Duration::from_secs(10);

/// Add a Claude or Codex server. `home` and `project` must be absolute.
pub fn add_agent_server(
    home: &Path,
    project: &Path,
    provider: &str,
    scope: &str,
    name: &str,
    server: &Value,
) -> Result<(), String> {
    let override_dir = |key| {
        std::env::var_os(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    match provider {
        "claude" => {
            let global = claude_global_config(home, override_dir("CLAUDE_CONFIG_DIR").as_deref());
            add_claude_server(&global, project, scope, name, server)
        }
        "codex" => {
            let directory = override_dir("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
            add_codex_server(&directory.join("config.toml"), scope, name, server)
        }
        _ => Err("Unsupported MCP provider".into()),
    }
}

/// The file Claude keeps user and local servers in.
fn claude_global_config(home: &Path, config_dir: Option<&Path>) -> PathBuf {
    let directory = config_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".claude"));
    let legacy = directory.join(".config.json");
    if std::fs::metadata(&legacy).is_ok_and(|file| file.is_file() && file.len() > 0) {
        return legacy;
    }
    config_dir.unwrap_or(home).join(".claude.json")
}

fn add_claude_server(
    global: &Path,
    project: &Path,
    scope: &str,
    name: &str,
    server: &Value,
) -> Result<(), String> {
    if !matches!(scope, "local" | "project" | "user") {
        return Err("Invalid Claude MCP scope".into());
    }
    let server = normalize_claude_server(server)?;
    if scope == "project" {
        return write_json_server(&project.join(".mcp.json"), name, server);
    }
    with_claude_lock(global, CLAUDE_LOCK_WAIT, || {
        write_json_config(global, |root| {
            let root = root
                .as_object_mut()
                .ok_or("Existing config must be a JSON object")?;
            if scope == "user" {
                return insert_server(root, name, server);
            }
            let projects = root.entry("projects").or_insert_with(|| json!({}));
            let projects = projects
                .as_object_mut()
                .ok_or("Existing projects must be an object")?;
            // The same fields Claude gives a project it has not seen before.
            let entry = projects
                .entry(claude_project_key(project))
                .or_insert_with(|| {
                    json!({
                        "allowedTools": [],
                        "mcpContextUris": [],
                        "mcpServers": {},
                        "enabledMcpjsonServers": [],
                        "disabledMcpjsonServers": [],
                        "hasTrustDialogAccepted": false,
                        "hasClaudeMdExternalIncludesApproved": false,
                        "hasClaudeMdExternalIncludesWarningShown": false,
                    })
                });
            let entry = entry
                .as_object_mut()
                .ok_or("Existing project entry must be an object")?;
            insert_server(entry, name, server)
        })
    })
}

/// Apply what `claude mcp add-json` checks, since nothing else will: Claude
/// skips an entry it cannot read rather than reporting it.
fn normalize_claude_server(server: &Value) -> Result<Value, String> {
    let mut object = server
        .as_object()
        .ok_or("Server configuration must be an object")?
        .clone();
    let kind = object
        .get("type")
        .map(|kind| kind.as_str().ok_or("type must be a string"))
        .transpose()?
        .map(str::to_owned);
    if let Some(url) = object.get("url").and_then(Value::as_str) {
        let schemes: &[&str] = match kind.as_deref() {
            None | Some("http") | Some("sse") => &["http", "https"],
            Some("ws") => &["ws", "wss"],
            Some(_) => return Err("Claude supports http, sse and ws servers".into()),
        };
        let parsed = url::Url::parse(url).map_err(|_| "Invalid server URL")?;
        if !schemes.contains(&parsed.scheme()) {
            return Err(format!("MCP URL must use {}", schemes.join(" or ")));
        }
        string_map(object.get("headers"), "headers")?;
        if kind.is_none() {
            object.insert("type".into(), "http".into());
        }
    } else {
        if !matches!(kind.as_deref(), None | Some("stdio")) {
            return Err("Local server type must be stdio".into());
        }
        local_command(&object)?;
        string_map(object.get("env"), "env")?;
    }
    Ok(Value::Object(object))
}

fn local_command(object: &Map<String, Value>) -> Result<(String, Vec<String>), String> {
    let command = object
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.is_empty())
        .ok_or("Local server needs a command")?;
    let parameters = object
        .get("args")
        .map(|args| args.as_array().ok_or("args must be an array"))
        .transpose()?
        .into_iter()
        .flatten()
        .map(|parameter| {
            parameter
                .as_str()
                .map(str::to_owned)
                .ok_or("args must contain strings".to_owned())
        })
        .collect::<Result<_, _>>()?;
    Ok((command.to_owned(), parameters))
}

fn string_map(value: Option<&Value>, field: &str) -> Result<toml::Table, String> {
    let Some(value) = value else {
        return Ok(toml::Table::new());
    };
    value
        .as_object()
        .ok_or_else(|| format!("{field} must be an object"))?
        .iter()
        .map(|(key, value)| {
            let value = value
                .as_str()
                .ok_or_else(|| format!("{field} values must be strings"))?;
            Ok((key.clone(), toml::Value::String(value.to_owned())))
        })
        .collect()
}

/// Claude guards its global config with a `<file>.lock` directory and rereads
/// the file inside it, so holding the lock keeps a running session from
/// overwriting the new server with its own older copy.
fn with_claude_lock<T>(
    config: &Path,
    wait: Duration,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let parent = config.parent().ok_or("Invalid config path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut lock = config.as_os_str().to_owned();
    lock.push(".lock");
    let lock = PathBuf::from(lock);
    let deadline = Instant::now() + wait;
    loop {
        match std::fs::create_dir(&lock) {
            Ok(()) => break,
            Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => {
                return Err(error.to_string());
            }
            Err(_) => {}
        }
        let stale = std::fs::metadata(&lock)
            .and_then(|lock| lock.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= CLAUDE_LOCK_STALE);
        if stale {
            let _ = std::fs::remove_dir(&lock);
        }
        if Instant::now() >= deadline {
            return Err("Claude Code is updating its config. Try again.".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let result = work();
    let _ = std::fs::remove_dir(&lock);
    result
}

/// Claude files a project's local servers under its main checkout, so every
/// worktree and subdirectory of a repository shares one entry.
pub fn claude_project_key(project: &Path) -> String {
    let project = std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    let root = main_checkout(&project).unwrap_or(project);
    let key = root.to_string_lossy();
    if cfg!(windows) {
        key.strip_prefix(r"\\?\").unwrap_or(&key).replace('\\', "/")
    } else {
        key.into_owned()
    }
}

fn main_checkout(start: &Path) -> Option<PathBuf> {
    let directory = start
        .ancestors()
        .find(|directory| directory.join(".git").exists())?;
    let dot_git = directory.join(".git");
    if dot_git.is_dir() {
        return Some(directory.to_path_buf());
    }
    // A linked worktree's `.git` file points at a directory holding `commondir`;
    // a submodule's has none and is its own root.
    let contents = std::fs::read_to_string(&dot_git).ok()?;
    let git_directory = directory.join(contents.strip_prefix("gitdir:")?.trim());
    let Ok(common) = std::fs::read_to_string(git_directory.join("commondir")) else {
        return Some(directory.to_path_buf());
    };
    let common = std::fs::canonicalize(git_directory.join(common.trim())).ok()?;
    common.parent().map(Path::to_path_buf)
}

fn add_codex_server(config: &Path, scope: &str, name: &str, server: &Value) -> Result<(), String> {
    if scope != "user" {
        return Err("Codex adds user-scoped servers only".into());
    }
    let section = codex_section(name, server)?;
    update_config(config, |existing| {
        let mut contents = existing.unwrap_or_default();
        let parsed: toml::Table = contents
            .parse()
            .map_err(|_| "Existing config is invalid TOML")?;
        match parsed.get("mcp_servers") {
            None => {}
            Some(toml::Value::Table(servers)) if !servers.contains_key(name) => {}
            Some(toml::Value::Table(_)) => {
                return Err(format!("{name} is already configured in this file"));
            }
            Some(_) => return Err("Existing mcp_servers must be a table".into()),
        }
        // Appending leaves the user's comments and ordering as they are.
        if !contents.is_empty() {
            if !contents.ends_with('\n') {
                contents.push('\n');
            }
            contents.push('\n');
        }
        contents.push_str(&section);
        contents
            .parse::<toml::Table>()
            .map_err(|_| "Existing mcp_servers cannot be extended; edit the file instead")?;
        Ok(contents.into_bytes())
    })
}

fn codex_section(name: &str, server: &Value) -> Result<String, String> {
    let object = server
        .as_object()
        .ok_or("Server configuration must be an object")?;
    let remote = object.get("url").and_then(Value::as_str);
    let allowed: &[&str] = if remote.is_some() {
        &["type", "url", "bearerTokenEnvVar", "headers"]
    } else {
        &["type", "command", "args", "env"]
    };
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!(
            "codex cannot preserve '{key}'; edit its config file instead"
        ));
    }
    let kind = object.get("type").and_then(Value::as_str);
    let mut entry = toml::Table::new();
    if let Some(url) = remote {
        if !matches!(kind, None | Some("http") | Some("remote")) {
            return Err("Codex supports HTTP URLs only".into());
        }
        let parsed = url::Url::parse(url).map_err(|_| "Invalid server URL")?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("MCP URL must use HTTP or HTTPS".into());
        }
        entry.insert("url".into(), url.into());
        if let Some(variable) = object.get("bearerTokenEnvVar").and_then(Value::as_str) {
            entry.insert("bearer_token_env_var".into(), variable.into());
        }
        let headers = string_map(object.get("headers"), "headers")?;
        if !headers.is_empty() {
            entry.insert("http_headers".into(), headers.into());
        }
    } else {
        if !matches!(kind, None | Some("stdio") | Some("local")) {
            return Err("Local server type must be stdio".into());
        }
        let (command, parameters) = local_command(object)?;
        entry.insert("command".into(), command.into());
        if !parameters.is_empty() {
            entry.insert("args".into(), parameters.into());
        }
        let env = string_map(object.get("env"), "env")?;
        if !env.is_empty() {
            entry.insert("env".into(), env.into());
        }
    }
    let servers = toml::Table::from_iter([(name.to_owned(), entry.into())]);
    let root = toml::Table::from_iter([("mcp_servers".to_owned(), servers.into())]);
    toml::to_string(&root).map_err(|e| e.to_string())
}

/// Add a server to a file with a top-level `mcpServers` map.
pub fn write_json_server(path: &Path, name: &str, server: Value) -> Result<(), String> {
    write_json_config(path, |root| {
        let object = root
            .as_object_mut()
            .ok_or("Existing config must be a JSON object")?;
        insert_server(object, name, server)
    })
}

fn insert_server(object: &mut Map<String, Value>, name: &str, server: Value) -> Result<(), String> {
    let servers = object.entry("mcpServers").or_insert_with(|| json!({}));
    let servers = servers
        .as_object_mut()
        .ok_or("Existing mcpServers must be an object")?;
    if servers.contains_key(name) {
        return Err(format!("{name} is already configured in this file"));
    }
    servers.insert(name.to_owned(), server);
    Ok(())
}

pub fn write_json_config(
    path: &Path,
    update: impl FnOnce(&mut Value) -> Result<(), String>,
) -> Result<(), String> {
    update_config(path, |existing| {
        let mut root: Value = match existing {
            Some(raw) => serde_json::from_str(&raw)
                .or_else(|_| serde_json::from_str(&strip_jsonc(&raw)))
                .map_err(|e| format!("Existing config is invalid JSON: {e}"))?,
            None => json!({}),
        };
        update(&mut root)?;
        serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())
    })
}

/// Replace a config file in one step, private to its owner. The update gets
/// the current contents, or `None` for a new file.
fn update_config(
    path: &Path,
    update: impl FnOnce(Option<String>) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    // Dotfile managers link these files; renaming onto a link would replace it.
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let parent = path.parent().ok_or("Invalid config path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let file_name = path
        .file_name()
        .and_then(|part| part.to_str())
        .unwrap_or("mcp");
    let lock_path = parent.join(format!(".{file_name}.lock"));
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    let existing = if path.exists() {
        Some(std::fs::read_to_string(&path).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let encoded = update(existing)?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        std::fs::rename(&temporary, &path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}

/// Remove JSONC comments and trailing commas without touching quoted text.
pub fn strip_jsonc(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut clean = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut quoted = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if quoted {
            clean.push(byte);
            if byte == b'\\' && index + 1 < bytes.len() {
                index += 1;
                clean.push(bytes[index]);
            } else if byte == b'"' {
                quoted = false;
            }
        } else if byte == b'"' {
            quoted = true;
            clean.push(byte);
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            clean.push(b'\n');
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 1).min(bytes.len() - 1);
        } else {
            clean.push(byte);
        }
        index += 1;
    }
    let mut result = Vec::with_capacity(clean.len());
    quoted = false;
    let mut escaped = false;
    for (index, byte) in clean.iter().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                quoted = false;
            }
        } else if *byte == b'"' {
            quoted = true;
        }
        if !quoted
            && *byte == b','
            && clean[index + 1..]
                .iter()
                .find(|b| !b.is_ascii_whitespace())
                .is_some_and(|b| *b == b'}' || *b == b']')
        {
            continue;
        }
        result.push(*byte);
    }
    String::from_utf8(result).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "s3cret-value";

    fn read_json(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn git_repository(root: &Path) {
        std::fs::create_dir_all(root.join(".git/worktrees/feature")).unwrap();
    }

    fn linked_worktree(repository: &Path, worktree: &Path) {
        std::fs::create_dir_all(worktree).unwrap();
        let git_directory = repository.join(".git/worktrees/feature");
        std::fs::write(git_directory.join("commondir"), "../..\n").unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", git_directory.display()),
        )
        .unwrap();
    }

    #[test]
    fn claude_user_server_joins_existing_config_with_its_secrets() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join(".claude.json");
        std::fs::write(
            &config,
            r#"{"userID":"keep","mcpServers":{"old":{"command":"node"}}}"#,
        )
        .unwrap();

        add_claude_server(
            &config,
            root.path(),
            "user",
            "docs",
            &json!({"url":"https://example.com/mcp","headers":{"Authorization":SECRET}}),
        )
        .unwrap();

        let saved = read_json(&config);
        assert_eq!(saved["userID"], "keep");
        assert_eq!(saved["mcpServers"]["old"]["command"], "node");
        assert_eq!(
            saved["mcpServers"]["docs"]["headers"]["Authorization"],
            SECRET
        );
        // Claude skips a URL entry with no type, so a bare URL has to gain one.
        assert_eq!(saved["mcpServers"]["docs"]["type"], "http");
        assert!(!config.with_extension("json.lock").exists());
        #[cfg(unix)]
        assert_eq!(mode(&config), 0o600);
    }

    #[test]
    fn claude_refuses_a_name_already_in_its_scope() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join(".claude.json");
        let server = json!({"command":"node"});
        add_claude_server(&config, root.path(), "user", "docs", &server).unwrap();
        assert_eq!(
            add_claude_server(&config, root.path(), "user", "docs", &server),
            Err("docs is already configured in this file".into())
        );
    }

    #[test]
    fn claude_project_server_goes_in_the_project_file() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join(".claude.json");
        let project = root.path().join("project");
        std::fs::create_dir_all(&project).unwrap();

        add_claude_server(
            &config,
            &project,
            "project",
            "docs",
            &json!({"command":"node","env":{"API_KEY":SECRET}}),
        )
        .unwrap();

        let path = project.join(".mcp.json");
        assert_eq!(
            read_json(&path)["mcpServers"]["docs"]["env"]["API_KEY"],
            SECRET
        );
        assert!(!config.exists());
        #[cfg(unix)]
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn claude_local_server_is_filed_under_the_main_checkout() {
        let root = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(root.path()).unwrap();
        let config = root.join(".claude.json");
        let repository = root.join("repository");
        git_repository(&repository);
        let worktree = root.join("worktree");
        linked_worktree(&repository, &worktree);
        let nested = worktree.join("src/app");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(repository.join("docs")).unwrap();
        let unrelated = root.join("unrelated");
        std::fs::create_dir_all(&unrelated).unwrap();
        let server = json!({"command":"node","env":{"API_KEY":SECRET}});

        add_claude_server(&config, &nested, "local", "one", &server).unwrap();
        add_claude_server(&config, &repository.join("docs"), "local", "two", &server).unwrap();
        add_claude_server(&config, &unrelated, "local", "three", &server).unwrap();

        let saved = read_json(&config);
        let main = &saved["projects"][claude_project_key(&repository)];
        assert_eq!(main["mcpServers"]["one"]["env"]["API_KEY"], SECRET);
        assert_eq!(main["mcpServers"]["two"]["command"], "node");
        // A new entry has every field Claude reads without a default.
        assert_eq!(main["allowedTools"], json!([]));
        let separate = &saved["projects"][claude_project_key(&unrelated)];
        assert_eq!(separate["mcpServers"]["three"]["command"], "node");
        assert!(saved["mcpServers"].is_null());
    }

    #[test]
    fn claude_local_server_keeps_an_existing_project_entry() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join(".claude.json");
        let project = root.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let key = claude_project_key(&project);
        std::fs::write(
            &config,
            json!({"projects":{&key:{"hasTrustDialogAccepted":true}}}).to_string(),
        )
        .unwrap();

        add_claude_server(
            &config,
            &project,
            "local",
            "docs",
            &json!({"command":"node"}),
        )
        .unwrap();

        let entry = &read_json(&config)["projects"][&key];
        assert_eq!(entry["hasTrustDialogAccepted"], true);
        assert_eq!(entry["mcpServers"]["docs"]["command"], "node");
    }

    #[test]
    fn submodule_is_its_own_project() {
        let root = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(root.path()).unwrap();
        let submodule = root.join("vendor/library");
        std::fs::create_dir_all(root.join(".git/modules/library")).unwrap();
        std::fs::create_dir_all(&submodule).unwrap();
        std::fs::write(
            submodule.join(".git"),
            "gitdir: ../../.git/modules/library\n",
        )
        .unwrap();
        assert_eq!(main_checkout(&submodule), Some(submodule));
    }

    #[test]
    fn claude_rejects_what_its_cli_would_reject() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join(".claude.json");
        for server in [
            json!({"type":"remote","url":"https://example.com"}),
            json!({"type":"http","url":"not a url"}),
            json!({"type":"http","url":"ftp://example.com"}),
            json!({"type":"ws","url":"https://example.com"}),
            json!({"type":"sse"}),
            json!({"command":""}),
            json!({"command":"node","args":"docs"}),
            json!({"command":"node","args":[1]}),
            json!({"command":"node","env":{"API_KEY":1}}),
            json!({"url":"https://example.com","headers":["Authorization"]}),
            json!("node"),
        ] {
            let result = add_claude_server(&config, root.path(), "user", "docs", &server);
            assert!(result.is_err(), "{server}");
        }
        assert!(!config.exists());
        assert_eq!(
            add_claude_server(
                &config,
                root.path(),
                "team",
                "docs",
                &json!({"command":"node"})
            ),
            Err("Invalid Claude MCP scope".into())
        );
    }

    #[test]
    fn claude_waits_for_a_running_session_to_release_its_lock() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join(".claude.json");
        let lock = root.path().join(".claude.json.lock");
        std::fs::create_dir(&lock).unwrap();

        let busy = with_claude_lock(&config, Duration::from_millis(50), || Ok(()));
        assert_eq!(
            busy,
            Err("Claude Code is updating its config. Try again.".into())
        );
        assert!(lock.exists(), "another process's lock must survive");

        let release = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                std::fs::remove_dir(lock).unwrap();
            })
        };
        let mut held = false;
        with_claude_lock(&config, Duration::from_secs(5), || {
            held = lock.is_dir();
            Ok(())
        })
        .unwrap();
        release.join().unwrap();
        assert!(held);
        assert!(!lock.exists());
    }

    #[cfg(unix)]
    #[test]
    fn claude_takes_over_a_lock_left_by_a_dead_session() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join(".claude.json");
        let lock = root.path().join(".claude.json.lock");
        std::fs::create_dir(&lock).unwrap();
        std::fs::File::open(&lock)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - CLAUDE_LOCK_STALE * 2)
            .unwrap();

        with_claude_lock(&config, Duration::from_secs(5), || Ok(())).unwrap();
        assert!(!lock.exists());
    }

    #[test]
    fn claude_config_location_follows_its_override_and_legacy_file() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let custom = home.join("custom");
        assert_eq!(claude_global_config(home, None), home.join(".claude.json"));
        assert_eq!(
            claude_global_config(home, Some(&custom)),
            custom.join(".claude.json")
        );
        std::fs::create_dir_all(&custom).unwrap();
        let legacy = custom.join(".config.json");
        std::fs::write(&legacy, "").unwrap();
        assert_eq!(
            claude_global_config(home, Some(&custom)),
            custom.join(".claude.json"),
            "an empty legacy file is not the config"
        );
        std::fs::write(&legacy, "{}").unwrap();
        assert_eq!(claude_global_config(home, Some(&custom)), legacy);
    }

    #[test]
    fn codex_stdio_server_keeps_the_rest_of_the_file_untouched() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config.toml");
        let original =
            "# my settings\nmodel = \"o3\" # inline note\n\n[mcp_servers.old]\ncommand = \"node\"";
        std::fs::write(&config, original).unwrap();

        add_codex_server(
            &config,
            "user",
            "docs",
            &json!({"type":"stdio","command":"npx","args":["-y","docs"],"env":{"API_KEY":SECRET}}),
        )
        .unwrap();

        let saved = std::fs::read_to_string(&config).unwrap();
        assert!(saved.starts_with(original), "{saved}");
        let parsed: toml::Table = saved.parse().unwrap();
        let docs = &parsed["mcp_servers"]["docs"];
        assert_eq!(docs["command"].as_str(), Some("npx"));
        assert_eq!(docs["args"][1].as_str(), Some("docs"));
        assert_eq!(docs["env"]["API_KEY"].as_str(), Some(SECRET));
        assert_eq!(
            parsed["mcp_servers"]["old"]["command"].as_str(),
            Some("node")
        );
        #[cfg(unix)]
        assert_eq!(mode(&config), 0o600);
    }

    #[test]
    fn codex_http_server_uses_its_own_field_names() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join(".codex/config.toml");

        add_codex_server(
            &config,
            "user",
            "docs",
            &json!({
                "type":"http",
                "url":"https://example.com/mcp",
                "bearerTokenEnvVar":"DOCS_TOKEN",
                "headers":{"Authorization":SECRET,"X-Api-Key":SECRET},
            }),
        )
        .unwrap();

        let parsed: toml::Table = std::fs::read_to_string(&config).unwrap().parse().unwrap();
        let docs = &parsed["mcp_servers"]["docs"];
        assert_eq!(docs["url"].as_str(), Some("https://example.com/mcp"));
        assert_eq!(docs["bearer_token_env_var"].as_str(), Some("DOCS_TOKEN"));
        assert_eq!(docs["http_headers"]["Authorization"].as_str(), Some(SECRET));
        assert_eq!(docs["http_headers"]["X-Api-Key"].as_str(), Some(SECRET));
    }

    #[test]
    fn codex_escapes_values_that_would_break_the_file() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config.toml");
        let tricky = "quote\" back\\slash\nnewline = [\"x\"]";

        add_codex_server(
            &config,
            "user",
            "docs",
            &json!({"command":"node","env":{"API_KEY":tricky}}),
        )
        .unwrap();

        let parsed: toml::Table = std::fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(
            parsed["mcp_servers"]["docs"]["env"]["API_KEY"].as_str(),
            Some(tricky)
        );
    }

    #[test]
    fn codex_leaves_the_file_alone_when_it_cannot_add() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config.toml");
        let server = json!({"command":"node"});
        for (original, error) in [
            (
                "[mcp_servers.docs]\ncommand = \"old\"\n",
                "docs is already configured in this file",
            ),
            (
                "mcp_servers = { other = { command = \"node\" } }\n",
                "Existing mcp_servers cannot be extended; edit the file instead",
            ),
            ("mcp_servers = 1\n", "Existing mcp_servers must be a table"),
            ("not [valid\n", "Existing config is invalid TOML"),
        ] {
            std::fs::write(&config, original).unwrap();
            assert_eq!(
                add_codex_server(&config, "user", "docs", &server),
                Err(error.into()),
                "{original}"
            );
            assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        }
    }

    #[test]
    fn codex_rejects_what_its_cli_rejected() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config.toml");
        for server in [
            json!({"type":"sse","url":"https://example.com"}),
            json!({"url":"ftp://example.com"}),
            json!({"url":"https://example.com","env":{"API_KEY":SECRET}}),
            json!({"command":"node","headers":{"Authorization":SECRET}}),
            json!({"command":"node","cwd":"/tmp"}),
            json!({"type":"http","command":"node"}),
        ] {
            let result = add_codex_server(&config, "user", "docs", &server);
            assert!(result.is_err(), "{server}");
        }
        assert_eq!(
            add_codex_server(&config, "project", "docs", &json!({"command":"node"})),
            Err("Codex adds user-scoped servers only".into())
        );
        assert!(!config.exists());
    }

    #[test]
    fn unknown_provider_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            add_agent_server(
                root.path(),
                root.path(),
                "cursor",
                "user",
                "docs",
                &json!({"command":"node"}),
            ),
            Err("Unsupported MCP provider".into())
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn linked_config_stays_a_link() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("dotfiles/claude.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "{}").unwrap();
        let config = root.path().join(".claude.json");
        std::os::unix::fs::symlink(&target, &config).unwrap();

        add_claude_server(
            &config,
            root.path(),
            "user",
            "docs",
            &json!({"command":"node"}),
        )
        .unwrap();

        assert!(std::fs::symlink_metadata(&config).unwrap().is_symlink());
        assert_eq!(read_json(&target)["mcpServers"]["docs"]["command"], "node");
    }

    #[cfg(windows)]
    #[test]
    fn project_key_uses_forward_slashes_without_a_verbatim_prefix() {
        let root = tempfile::tempdir().unwrap();
        let key = claude_project_key(root.path());
        assert!(!key.starts_with(r"\\?\"), "{key}");
        assert!(!key.contains('\\'), "{key}");
    }
}
