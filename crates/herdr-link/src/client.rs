use std::fmt;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::transport::{Conn, Transport};
use crate::{parse_event, Agent, Event, Pong, Workspace};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The connection ended before a reply (or, on a stream, ended).
    Closed,
    /// herdr answered with an error.
    Api {
        code: String,
        message: String,
    },
    /// A reply that is not shaped the way the API says.
    BadReply(String),
    /// ssh could not reach the machine's herdr.
    Ssh(crate::ssh::SshFailure),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Closed => write!(f, "connection closed"),
            Error::Api { code, message } => write!(f, "herdr error {code}: {message}"),
            Error::BadReply(why) => write!(f, "unexpected reply: {why}"),
            Error::Ssh(failure) => write!(f, "{failure}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Ssh(f) => Some(f),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        // A transport reports why it died by wrapping an `SshFailure`.
        if let Some(failure) = e
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<crate::ssh::SshFailure>())
        {
            return Error::Ssh(failure.clone());
        }
        Error::Io(e)
    }
}

/// Talks to herdr over a [`Transport`], opening one connection per request.
pub struct Client<T: Transport> {
    transport: T,
    next_id: AtomicU64,
}

impl<T: Transport> Client<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            next_id: AtomicU64::new(0),
        }
    }

    /// Send one request and return its `result`.
    pub fn call(&self, method: &str, params: Value) -> Result<Value, Error> {
        let mut conn = self.send(method, params)?;
        read_result(&mut conn.reader)
    }

    pub fn ping(&self) -> Result<Pong, Error> {
        self.typed("ping", json!({}), None)
    }

    pub fn agents(&self) -> Result<Vec<Agent>, Error> {
        self.typed("agent.list", json!({}), Some("agents"))
    }

    pub fn workspaces(&self) -> Result<Vec<Workspace>, Error> {
        self.typed("workspace.list", json!({}), Some("workspaces"))
    }

    /// Start an event stream. The connection stays open until `Events` is
    /// dropped.
    pub fn subscribe(&self, subscriptions: &[Value]) -> Result<Events, Error> {
        let mut conn = self.send(
            "events.subscribe",
            json!({ "subscriptions": subscriptions }),
        )?;
        read_result(&mut conn.reader)?;
        Ok(Events { conn })
    }

    fn send(&self, method: &str, params: Value) -> Result<Conn, Error> {
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = json!({ "id": format!("{method}-{n}"), "method": method, "params": params });
        let mut conn = self.transport.open()?;
        writeln!(conn.writer, "{request}")?;
        conn.writer.flush()?;
        Ok(conn)
    }

    fn typed<R: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
        field: Option<&str>,
    ) -> Result<R, Error> {
        let mut result = self.call(method, params)?;
        let value = match field {
            Some(field) => result
                .get_mut(field)
                .map(Value::take)
                .ok_or_else(|| Error::BadReply(format!("{method} reply has no {field}")))?,
            None => result,
        };
        serde_json::from_value(value).map_err(|e| Error::BadReply(format!("{method}: {e}")))
    }
}

/// An open event subscription.
pub struct Events {
    conn: Conn,
}

impl Events {
    /// Cuts the stream from another thread; the reader then sees it end.
    pub fn closer(&self) -> Option<crate::transport::Closer> {
        self.conn.closer.clone()
    }
}

impl Iterator for Events {
    type Item = Result<Event, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut line = String::new();
            match self.conn.reader.read_line(&mut line) {
                Ok(0) => return Some(Err(Error::Closed)),
                Ok(_) => {}
                Err(e) => return Some(Err(e.into())),
            }
            if let Some(event) = parse_event(&line) {
                return Some(Ok(event));
            }
        }
    }
}

fn read_result(reader: &mut dyn BufRead) -> Result<Value, Error> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(Error::Closed);
    }
    let mut reply: Value =
        serde_json::from_str(&line).map_err(|e| Error::BadReply(e.to_string()))?;
    if let Some(error) = reply.get("error") {
        let text = |key| {
            error
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        return Err(Error::Api {
            code: text("code"),
            message: text("message"),
        });
    }
    reply
        .get_mut("result")
        .map(Value::take)
        .ok_or_else(|| Error::BadReply("reply has neither result nor error".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::FakeTransport;
    use crate::{AgentStatus, Event};
    use serde_json::json;

    fn client(scripts: &[&str]) -> (Client<FakeTransport>, FakeTransport) {
        let fake = FakeTransport::new(scripts);
        (Client::new(fake.clone()), fake)
    }

    const PONG: &str = "{\"id\":\"ping-0\",\"result\":{\"version\":\"0.9.5\",\"protocol\":1}}\n";
    const AGENTS: &str = "{\"id\":\"agent.list-1\",\"result\":{\"agents\":[{\"terminal_id\":\"t1\",\"pane_id\":\"w1:p1\",\"workspace_id\":\"w1\",\"tab_id\":\"w1:t1\",\"agent_status\":\"working\"}]}}\n";

    #[test]
    fn each_call_opens_its_own_connection() {
        let (c, fake) = client(&[PONG, AGENTS]);
        assert_eq!(c.ping().unwrap().version, "0.9.5");
        let agents = c.agents().unwrap();
        assert_eq!(agents[0].agent_status, AgentStatus::Working);
        assert_eq!(fake.opens(), 2);
        assert_eq!(fake.requests()[1]["method"], "agent.list");
    }

    #[test]
    fn request_has_id_method_params() {
        let (c, fake) = client(&[PONG]);
        c.ping().unwrap();
        let req = &fake.requests()[0];
        assert!(req["id"].as_str().unwrap().starts_with("ping-"));
        assert_eq!(req["method"], "ping");
        assert_eq!(req["params"], json!({}));
    }

    #[test]
    fn request_ids_differ_between_calls() {
        let (c, fake) = client(&[PONG, PONG]);
        c.ping().unwrap();
        c.ping().unwrap();
        let r = fake.requests();
        assert_ne!(r[0]["id"], r[1]["id"]);
    }

    #[test]
    fn workspaces_reads_result_workspaces() {
        let (c, fake) = client(&[
            "{\"id\":\"x\",\"result\":{\"workspaces\":[{\"workspace_id\":\"w1\",\"label\":\"main\"}]}}\n",
        ]);
        let ws = c.workspaces().unwrap();
        assert_eq!(ws[0].label, "main");
        assert_eq!(fake.requests()[0]["method"], "workspace.list");
    }

    #[test]
    fn api_error_is_typed() {
        let (c, _) = client(&[
            "{\"error\":{\"code\":\"agent_not_ready\",\"message\":\"still starting\"},\"id\":\"x\"}\n",
        ]);
        let err = c.ping().unwrap_err();
        assert!(
            matches!(&err, Error::Api { code, message } if code == "agent_not_ready" && message == "still starting"),
            "{err:?}"
        );
    }

    #[test]
    fn empty_reply_is_closed() {
        let (c, _) = client(&[""]);
        assert!(matches!(c.ping(), Err(Error::Closed)));
    }

    #[test]
    fn reply_without_result_or_error_is_bad() {
        let (c, _) = client(&["{\"id\":\"x\"}\n"]);
        assert!(matches!(c.ping(), Err(Error::BadReply(_))));
    }

    #[test]
    fn non_json_reply_is_bad() {
        let (c, _) = client(&["not json\n"]);
        assert!(matches!(c.ping(), Err(Error::BadReply(_))));
    }

    #[test]
    fn result_of_the_wrong_shape_is_bad() {
        let (c, _) = client(&["{\"id\":\"x\",\"result\":{\"agents\":\"nope\"}}\n"]);
        assert!(matches!(c.agents(), Err(Error::BadReply(_))));
    }

    #[test]
    fn open_failure_is_io() {
        let (c, _) = client(&[]);
        assert!(matches!(c.ping(), Err(Error::Io(_))));
    }

    const ACK: &str =
        "{\"id\":\"events.subscribe-0\",\"result\":{\"type\":\"subscription_started\"}}\n";
    const STATUS: &str = "{\"event\":\"pane.agent_status_changed\",\"data\":{\"pane_id\":\"w1:p1\",\"workspace_id\":\"w1\",\"agent_status\":\"blocked\"}}\n";

    #[test]
    fn subscribe_skips_ack_and_yields_events_then_closed() {
        let script = format!("{ACK}{STATUS}");
        let (c, fake) = client(&[&script]);
        let mut events = c
            .subscribe(&[json!({"type": "pane.agent_status_changed"})])
            .unwrap();
        assert_eq!(
            events.next().unwrap().unwrap(),
            Event::AgentStatusChanged {
                pane_id: "w1:p1".into(),
                workspace_id: "w1".into(),
                agent_status: AgentStatus::Blocked,
            }
        );
        assert!(matches!(events.next(), Some(Err(Error::Closed))));
        let req = &fake.requests()[0];
        assert_eq!(req["method"], "events.subscribe");
        assert_eq!(
            req["params"]["subscriptions"][0]["type"],
            "pane.agent_status_changed"
        );
    }

    #[test]
    fn subscribe_rejected_is_error() {
        let (c, _) = client(&[
            "{\"id\":\"x\",\"error\":{\"code\":\"invalid_params\",\"message\":\"bad\"}}\n",
        ]);
        assert!(matches!(c.subscribe(&[]), Err(Error::Api { .. })));
    }

    #[test]
    fn subscribe_closed_before_ack_is_closed() {
        let (c, _) = client(&[""]);
        assert!(matches!(c.subscribe(&[]), Err(Error::Closed)));
    }

    #[test]
    fn stream_lines_that_are_not_events_are_skipped() {
        let script = format!("{ACK}\n{{\"id\":\"stray\",\"result\":{{}}}}\n{STATUS}");
        let (c, _) = client(&[&script]);
        let mut events = c.subscribe(&[]).unwrap();
        assert!(matches!(
            events.next(),
            Some(Ok(Event::AgentStatusChanged { .. }))
        ));
    }
}
