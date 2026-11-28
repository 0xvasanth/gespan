//! Tests for the pull request layer. Nothing here reaches a real host: a
//! [`Fake`] transport answers from routes and keeps every request, and the
//! process and HTTPS seams have fakes of their own.
use super::{
    transport::{
        ApiRequest, ApiResponse, CommandRunner, HttpClient, Method, ProcessOutput, Transport,
    },
    HostError, Result,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

mod contract;
mod factory;
mod hosts;
mod service;
mod transports;

/// A transport that answers each `(method, path)` from its routes, and
/// says "not found" for anything else, keeping every request it was sent.
#[derive(Clone, Default)]
pub(super) struct Fake {
    routes: Arc<Mutex<HashMap<(Method, String), ApiResponse>>>,
    sent: Arc<Mutex<Vec<ApiRequest>>>,
}

impl Fake {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn on(&self, method: Method, path: &str, response: ApiResponse) -> &Self {
        self.routes
            .lock()
            .unwrap()
            .insert((method, path.to_owned()), response);
        self
    }
    pub fn get(&self, path: &str, body: Value) -> &Self {
        self.on(Method::Get, path, json(body))
    }
    pub fn transport(&self) -> Box<dyn Transport> {
        Box::new(self.clone())
    }
    pub fn sent(&self) -> Vec<ApiRequest> {
        self.sent.lock().unwrap().clone()
    }
    /// `METHOD path` of every request, in order.
    pub fn calls(&self) -> Vec<String> {
        self.sent()
            .iter()
            .map(|r| format!("{} {}", r.method.as_str(), r.path))
            .collect()
    }
    /// The body of the one request sent as `METHOD path`.
    pub fn body(&self, method: Method, path: &str) -> Value {
        let sent = self.sent();
        let matching: Vec<_> = sent
            .iter()
            .filter(|r| r.method == method && r.path == path)
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "one {method:?} {path}: {:?}",
            self.calls()
        );
        matching[0].body.clone().unwrap_or(Value::Null)
    }
}

impl Transport for Fake {
    fn send(&self, request: &ApiRequest) -> Result<ApiResponse> {
        self.sent.lock().unwrap().push(request.clone());
        match self
            .routes
            .lock()
            .unwrap()
            .get(&(request.method, request.path.clone()))
        {
            Some(response) => super::transport::check(response.clone()),
            None => Err(HostError::NotFound(format!(
                "no route for {} {}",
                request.method.as_str(),
                request.path
            ))),
        }
    }
}

pub(super) fn json(body: Value) -> ApiResponse {
    ApiResponse::ok(body.to_string())
}

pub(super) fn with_headers(body: Value, headers: &[(&str, &str)]) -> ApiResponse {
    ApiResponse {
        status: 200,
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: body.to_string(),
    }
}

/// One run of a CLI: the program, its arguments and its stdin.
pub(super) type Run = (String, Vec<String>, Option<String>);
/// One HTTPS request: method, URL, headers and body.
pub(super) type Sent = (Method, String, Vec<(String, String)>, Option<String>);

/// A CLI that prints `output` and keeps how it was run.
#[derive(Clone, Default)]
pub(super) struct FakeRunner {
    pub output: Arc<Mutex<ProcessOutput>>,
    pub runs: Arc<Mutex<Vec<Run>>>,
}

impl FakeRunner {
    pub fn printing(success: bool, stdout: &str, stderr: &str) -> Self {
        let runner = Self::default();
        *runner.output.lock().unwrap() = ProcessOutput {
            success,
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        };
        runner
    }
    pub fn runs(&self) -> Vec<Run> {
        self.runs.lock().unwrap().clone()
    }
}

impl CommandRunner for FakeRunner {
    fn run(
        &self,
        program: &str,
        args: &[String],
        stdin: Option<&str>,
    ) -> std::result::Result<ProcessOutput, String> {
        self.runs.lock().unwrap().push((
            program.to_owned(),
            args.to_vec(),
            stdin.map(str::to_owned),
        ));
        Ok(self.output.lock().unwrap().clone())
    }
}

/// HTTPS that answers `response` and keeps what it was sent.
#[derive(Clone, Default)]
pub(super) struct FakeHttp {
    pub response: Arc<Mutex<ApiResponse>>,
    pub sent: Arc<Mutex<Vec<Sent>>>,
}

impl FakeHttp {
    pub fn answering(response: ApiResponse) -> Self {
        let http = Self::default();
        *http.response.lock().unwrap() = response;
        http
    }
    pub fn header(&self, index: usize, name: &str) -> Option<String> {
        self.sent.lock().unwrap()[index]
            .2
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }
    pub fn url(&self, index: usize) -> String {
        self.sent.lock().unwrap()[index].1.clone()
    }
}

impl HttpClient for FakeHttp {
    fn send(
        &self,
        method: Method,
        url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) -> std::result::Result<ApiResponse, String> {
        self.sent.lock().unwrap().push((
            method,
            url.to_owned(),
            headers.to_vec(),
            body.map(str::to_owned),
        ));
        Ok(self.response.lock().unwrap().clone())
    }
}
