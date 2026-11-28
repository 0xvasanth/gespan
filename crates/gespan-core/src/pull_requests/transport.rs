//! How a request reaches a code host: the one thing that changes with the
//! way you connect. A host adapter builds an [`ApiRequest`] against the
//! host's REST API and gets an [`ApiResponse`] back; the [`Transport`]
//! decides whether that goes through a CLI the user is signed in to (`gh api`,
//! `glab api`) or straight over HTTPS with a personal access token.
//!
//! Both kinds reach the outside through a seam of their own ([`CommandRunner`]
//! for processes, [`HttpClient`] for HTTPS) so tests can stand in for them
//! and see exactly what would have been sent.
use super::error::{HostError, Result};
use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

/// One REST call: `path` is relative to the host's API root
/// (`repos/acme/web/pulls`, `projects/acme%2Fweb/merge_requests`).
#[derive(Clone, Debug, PartialEq)]
pub struct ApiRequest {
    pub method: Method,
    pub path: String,
    pub body: Option<Value>,
}

impl ApiRequest {
    pub fn get(path: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            path: path.into(),
            body: None,
        }
    }
    pub fn with_body(method: Method, path: impl Into<String>, body: Value) -> Self {
        Self {
            method,
            path: path.into(),
            body: Some(body),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApiResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl ApiResponse {
    pub fn ok(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
        }
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        let body = if self.body.trim().is_empty() {
            "null"
        } else {
            &self.body
        };
        serde_json::from_str(body).map_err(|error| HostError::Parse(error.to_string()))
    }
    /// Whether a `Link` header names a next page.
    pub fn has_next_link(&self) -> bool {
        self.header("link")
            .is_some_and(|link| link.split(',').any(|part| part.contains("rel=\"next\"")))
    }
}

/// Sends a request to the host and returns its answer, or the error a
/// failed answer means. Only 2xx answers come back as `Ok`.
pub trait Transport: Send + Sync {
    fn send(&self, request: &ApiRequest) -> Result<ApiResponse>;
}

/// The error a non-2xx answer means, the same however it arrived.
pub(crate) fn check(response: ApiResponse) -> Result<ApiResponse> {
    if (200..300).contains(&response.status) {
        return Ok(response);
    }
    let message = host_message(&response.body);
    let retry_after = response
        .header("retry-after")
        .and_then(|value| value.trim().parse().ok());
    let exhausted = response
        .header("x-ratelimit-remaining")
        .is_some_and(|value| value.trim() == "0");
    Err(match response.status {
        429 => HostError::RateLimited {
            retry_after_secs: retry_after,
        },
        403 if exhausted
            || retry_after.is_some()
            || message.to_lowercase().contains("rate limit") =>
        {
            HostError::RateLimited {
                retry_after_secs: retry_after,
            }
        }
        401 | 403 => HostError::Unauthorized(message),
        404 => HostError::NotFound(message),
        status => HostError::Http { status, message },
    })
}

/// The message in an error body: GitHub's and GitLab's `message`, GitLab's
/// `error`, Azure DevOps's `message`, or the text itself.
fn host_message(body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let field = parsed.as_ref().and_then(|value| {
        ["message", "error", "error_description"]
            .iter()
            .find_map(|key| match &value[*key] {
                Value::String(text) => Some(text.clone()),
                Value::Null => None,
                other => Some(other.to_string()),
            })
    });
    field.unwrap_or_else(|| body.trim().chars().take(300).collect())
}

/// What a finished process printed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a CLI. The real one starts the process; tests stand in for it.
pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        program: &str,
        args: &[String],
        stdin: Option<&str>,
    ) -> std::result::Result<ProcessOutput, String>;
}

/// Starts the CLI as the app does, with the GUI's environment and a
/// timeout.
pub struct SystemRunner {
    pub timeout: Duration,
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
        }
    }
}

impl CommandRunner for SystemRunner {
    fn run(
        &self,
        program: &str,
        args: &[String],
        stdin: Option<&str>,
    ) -> std::result::Result<ProcessOutput, String> {
        let binary = crate::scm::resolve_gui_binary(program)
            .ok_or_else(|| format!("{program} is not installed"))?;
        let mut command = Command::new(binary);
        crate::scm::shell::apply_gui_env(&mut command);
        command
            .args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_PROMPT", "1")
            .env("GH_PAGER", "cat")
            .env("GLAB_PAGER", "cat")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::process::isolate(&mut command);
        let mut child = crate::process::spawn(&mut command).map_err(|e| e.to_string())?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            let text = text.to_owned();
            std::thread::spawn(move || {
                let _ = pipe.write_all(text.as_bytes());
            });
        }
        let read = |pipe: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut text = Vec::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_end(&mut text);
                }
                String::from_utf8_lossy(&text).into_owned()
            })
        };
        let stdout = read(
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let stderr = read(
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Err(error) => break Err(error.to_string()),
                Ok(None) if Instant::now() >= deadline => {
                    break Err(format!("{program} timed out"));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        crate::process::terminate(&mut child);
        let output = ProcessOutput {
            success: status?.success(),
            stdout: stdout.join().unwrap_or_default(),
            stderr: stderr.join().unwrap_or_default(),
        };
        Ok(output)
    }
}

/// A host's REST API through its CLI: `gh api` for GitHub, `glab api` for
/// GitLab. Both take the same flags, sign the request with the account the
/// user logged in with, and print the status line and headers with
/// `--include`.
pub struct CliTransport {
    program: &'static str,
    hostname: Option<String>,
    runner: Arc<dyn CommandRunner>,
}

impl CliTransport {
    pub fn new(
        program: &'static str,
        hostname: Option<String>,
        runner: Arc<dyn CommandRunner>,
    ) -> Self {
        Self {
            program,
            hostname: hostname.filter(|h| !h.trim().is_empty()),
            runner,
        }
    }
    pub fn program(&self) -> &str {
        self.program
    }
    /// `api --method M --include [--hostname H] PATH [--input -]`.
    pub fn args(&self, request: &ApiRequest) -> Vec<String> {
        let mut args = vec![
            "api".to_owned(),
            "--method".to_owned(),
            request.method.as_str().to_owned(),
            "--include".to_owned(),
        ];
        if let Some(hostname) = &self.hostname {
            args.extend(["--hostname".to_owned(), hostname.clone()]);
        }
        args.push(request.path.clone());
        if request.body.is_some() {
            args.extend(["--input".to_owned(), "-".to_owned()]);
        }
        args
    }
}

impl Transport for CliTransport {
    fn send(&self, request: &ApiRequest) -> Result<ApiResponse> {
        let body = request.body.as_ref().map(Value::to_string);
        let output = self
            .runner
            .run(self.program, &self.args(request), body.as_deref())
            .map_err(HostError::Transport)?;
        match parse_included(&output.stdout) {
            Some(response) => check(response),
            // No status line: the CLI failed before it asked (not signed in,
            // unknown host).
            None => {
                let message = output.stderr.trim();
                let lower = message.to_lowercase();
                Err(
                    if lower.contains("auth login") || lower.contains("not logged") {
                        HostError::Unauthorized(message.to_owned())
                    } else {
                        HostError::Transport(if message.is_empty() {
                            format!("{} printed nothing", self.program)
                        } else {
                            message.to_owned()
                        })
                    },
                )
            }
        }
    }
}

/// An answer printed with `--include`: the status line, headers, a blank
/// line, then the body. None if the output doesn't start with a status line.
pub(crate) fn parse_included(output: &str) -> Option<ApiResponse> {
    let text = output.trim_start();
    let (head, body) = match text.find("\r\n\r\n") {
        Some(at) => (&text[..at], &text[at + 4..]),
        None => match text.find("\n\n") {
            Some(at) => (&text[..at], &text[at + 2..]),
            None => (text, ""),
        },
    };
    let mut lines = head.lines();
    let status_line = lines.next()?;
    if !status_line.starts_with("HTTP/") {
        return None;
    }
    let status = status_line.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect();
    Some(ApiResponse {
        status,
        headers,
        body: body.to_owned(),
    })
}

/// Sends a request over HTTPS. The real one is `ureq`; tests stand in for it.
pub trait HttpClient: Send + Sync {
    fn send(
        &self,
        method: Method,
        url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) -> std::result::Result<ApiResponse, String>;
}

/// HTTPS through `ureq`, with the app's resolver and a timeout.
pub struct UreqClient {
    agent: ureq::Agent,
}

impl Default for UreqClient {
    fn default() -> Self {
        Self {
            agent: crate::net::agent().timeout(Duration::from_secs(30)).build(),
        }
    }
}

impl HttpClient for UreqClient {
    fn send(
        &self,
        method: Method,
        url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) -> std::result::Result<ApiResponse, String> {
        let mut request = self.agent.request(method.as_str(), url);
        for (key, value) in headers {
            request = request.set(key, value);
        }
        let result = match body {
            Some(body) => request.send_string(body),
            None => request.call(),
        };
        let response = match result {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(error) => return Err(error.to_string()),
        };
        let status = response.status();
        let headers = response
            .headers_names()
            .into_iter()
            .filter_map(|name| {
                let value = response.header(&name)?.to_owned();
                Some((name, value))
            })
            .collect();
        let body = response.into_string().map_err(|e| e.to_string())?;
        Ok(ApiResponse {
            status,
            headers,
            body,
        })
    }
}

/// How a personal access token signs a request, which differs by host.
#[derive(Clone, PartialEq, Eq)]
pub enum Auth {
    /// GitHub: `Authorization: Bearer <token>`.
    Bearer(String),
    /// GitLab: `PRIVATE-TOKEN: <token>`.
    PrivateToken(String),
    /// Azure DevOps: basic auth with an empty user and the token as the
    /// password.
    BasicToken(String),
}

impl std::fmt::Debug for Auth {
    // Never print the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Bearer(_) => "Bearer(…)",
            Self::PrivateToken(_) => "PrivateToken(…)",
            Self::BasicToken(_) => "BasicToken(…)",
        })
    }
}

impl Auth {
    pub fn header(&self) -> (String, String) {
        match self {
            Self::Bearer(token) => ("Authorization".into(), format!("Bearer {token}")),
            Self::PrivateToken(token) => ("PRIVATE-TOKEN".into(), token.clone()),
            Self::BasicToken(token) => (
                "Authorization".into(),
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(format!(":{token}"))
                ),
            ),
        }
    }
}

/// A host's REST API over HTTPS, signed with a personal access token.
pub struct TokenTransport {
    base_url: String,
    auth: Auth,
    extra_headers: Vec<(String, String)>,
    client: Arc<dyn HttpClient>,
}

impl TokenTransport {
    pub fn new(base_url: impl Into<String>, auth: Auth, client: Arc<dyn HttpClient>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            auth,
            extra_headers: Vec::new(),
            client,
        }
    }
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.extra_headers.push((key.to_owned(), value.to_owned()));
        self
    }
    pub fn url(&self, path: &str) -> String {
        if path.starts_with("https://") || path.starts_with("http://") {
            path.to_owned()
        } else {
            format!("{}/{}", self.base_url, path.trim_start_matches('/'))
        }
    }
}

impl Transport for TokenTransport {
    fn send(&self, request: &ApiRequest) -> Result<ApiResponse> {
        let mut headers = vec![
            self.auth.header(),
            ("Accept".to_owned(), "application/json".to_owned()),
            ("User-Agent".to_owned(), "Gespan".to_owned()),
        ];
        // A host's own Accept replaces the generic one.
        for (key, value) in &self.extra_headers {
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
            headers.push((key.clone(), value.clone()));
        }
        let body = request.body.as_ref().map(Value::to_string);
        if body.is_some() {
            headers.push(("Content-Type".to_owned(), "application/json".to_owned()));
        }
        let response = self
            .client
            .send(
                request.method,
                &self.url(&request.path),
                &headers,
                body.as_deref(),
            )
            .map_err(HostError::Transport)?;
        check(response)
    }
}
