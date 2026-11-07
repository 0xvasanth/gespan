//! What sharing one upstream server between many agents requires of the
//! messages between them. The server sees the hub as its only client, so every
//! request id, progress token and subscription is the hub's own, and anything
//! the server sends is mapped back to the session it belongs to.
//!
//! This is the logic alone, with no I/O: a frame goes in, and what to do with
//! it comes out.
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};

/// The agent session a message belongs to.
pub(crate) type Caller = i64;

/// Requests a server may send its client on older protocol versions.
const SERVER_REQUESTS: [&str; 3] = ["sampling/createMessage", "elicitation/create", "roots/list"];

/// How far a server's requests may reach, by how it's shared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// Several agents use this process, so nothing is traced to one of them.
    Shared,
    /// One workspace, possibly several sessions.
    Workspace,
    /// One session.
    Session,
}

/// What to do after a frame from the server.
#[derive(Debug, PartialEq)]
pub(crate) enum Action {
    /// Write this frame back to the server.
    Reply(Value),
    /// A request of ours was answered.
    Answered {
        id: u64,
        outcome: Result<Value, Value>,
    },
    /// Send this to a session's agent.
    Notify { caller: Caller, message: Value },
    /// Send this server request to the session it belongs to.
    Ask {
        caller: Caller,
        upstream_id: Value,
        method: String,
        params: Value,
    },
    /// A list changed: refresh it, then tell every user.
    ListChanged { method: String },
    /// A resource changed: tell these sessions.
    ResourceUpdated {
        callers: BTreeSet<Caller>,
        message: Value,
    },
    /// A log line, for agents that asked for this level or above.
    Log { level: String, message: Value },
    /// The server needs what a shared server can't have, so it should move to
    /// a narrower class.
    Demote { method: String },
}

/// What to do with something a server needs for one agent's call, on the
/// sessionless protocol. The call it belongs to is known there, so even a
/// shared process can put it to that agent.
#[derive(Debug, PartialEq)]
pub(crate) enum Input {
    /// The hub has the answer.
    Answer(Value),
    /// Ask the agent that made the call.
    Ask,
    Refuse(String),
}

fn refusal(id: &Value, code: i64, message: String) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn untraceable(id: &Value, method: &str) -> Value {
    refusal(id, -32603, format!("{method} can't be traced to one agent"))
}

fn frame(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}

/// Tells a server nobody is waiting on this request any more.
pub(crate) fn cancelled(id: u64) -> Value {
    json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}})
}

struct Pending {
    caller: Caller,
    /// The hub's progress token, for this request's life.
    progress: Option<String>,
}

pub(crate) struct Router {
    next: u64,
    pending: HashMap<u64, Pending>,
    /// The hub's token to the caller and the token the caller used.
    progress: HashMap<String, (Caller, Value)>,
    subscribers: HashMap<String, BTreeSet<Caller>>,
    /// The `file:` URI of the folder this process serves.
    root: Option<String>,
    reach: Reach,
}

impl Router {
    pub(crate) fn new(reach: Reach, root: Option<String>) -> Self {
        Self {
            next: 0,
            pending: HashMap::new(),
            progress: HashMap::new(),
            subscribers: HashMap::new(),
            root,
            reach,
        }
    }

    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// A request the hub makes for itself, such as refreshing a list.
    pub(crate) fn own_request(&mut self, method: &str, params: Value) -> (u64, Value) {
        let id = self.id();
        (id, frame(id, method, params))
    }

    /// An agent's request, rewritten so nothing of the agent's reaches the
    /// server. Returns the id the answer will carry.
    pub(crate) fn request(
        &mut self,
        caller: Caller,
        method: &str,
        mut params: Value,
    ) -> (u64, Value) {
        let id = self.id();
        let token = params["_meta"]
            .get("progressToken")
            .filter(|token| !token.is_null())
            .cloned();
        let progress = token.map(|token| {
            let ours = format!("hub-{id}");
            params["_meta"]["progressToken"] = json!(ours);
            self.progress.insert(ours.clone(), (caller, token));
            ours
        });
        self.pending.insert(id, Pending { caller, progress });
        (id, frame(id, method, params))
    }

    /// Tells the server an agent gave up on a request.
    pub(crate) fn cancel(&mut self, id: u64) -> Option<Value> {
        self.settle(id)?;
        Some(cancelled(id))
    }

    fn settle(&mut self, id: u64) -> Option<Pending> {
        let pending = self.pending.remove(&id)?;
        if let Some(token) = &pending.progress {
            self.progress.remove(token);
        }
        Some(pending)
    }

    /// Adds a subscriber. Only the first for a resource reaches the server.
    pub(crate) fn subscribe(&mut self, caller: Caller, uri: &str) -> Option<Value> {
        let subscribers = self.subscribers.entry(uri.to_owned()).or_default();
        let first = subscribers.is_empty();
        subscribers.insert(caller);
        first.then(|| {
            let id = self.id();
            frame(id, "resources/subscribe", json!({"uri":uri}))
        })
    }

    /// Removes a subscriber. Only the last to leave reaches the server.
    pub(crate) fn unsubscribe(&mut self, caller: Caller, uri: &str) -> Option<Value> {
        let subscribers = self.subscribers.get_mut(uri)?;
        if !subscribers.remove(&caller) || !subscribers.is_empty() {
            return None;
        }
        self.subscribers.remove(uri);
        let id = self.id();
        Some(frame(id, "resources/unsubscribe", json!({"uri":uri})))
    }

    /// Every resource someone is subscribed to.
    pub(crate) fn subscribed(&self) -> Vec<String> {
        let mut uris: Vec<_> = self.subscribers.keys().cloned().collect();
        uris.sort();
        uris
    }

    /// What to do with an input a server asked for in an `input_required`.
    pub(crate) fn input(&self, method: &str) -> Input {
        if !SERVER_REQUESTS.contains(&method) {
            return Input::Refuse(format!("{method} isn't available here"));
        }
        match (&self.root, method) {
            (Some(uri), "roots/list") => Input::Answer(json!({"roots":[{"uri":uri}]})),
            _ => Input::Ask,
        }
    }

    /// The server refused this subscription, so nobody holds it.
    pub(crate) fn refused(&mut self, uri: &str) {
        self.subscribers.remove(uri);
    }

    /// Everything a session held here goes when it does.
    pub(crate) fn forget(&mut self, caller: Caller) -> Vec<Value> {
        let uris: Vec<_> = self
            .subscribers
            .iter()
            .filter(|(_, subscribers)| subscribers.contains(&caller))
            .map(|(uri, _)| uri.clone())
            .collect();
        let mut frames: Vec<_> = uris
            .iter()
            .filter_map(|uri| self.unsubscribe(caller, uri))
            .collect();
        let abandoned: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.caller == caller)
            .map(|(id, _)| *id)
            .collect();
        frames.extend(abandoned.into_iter().filter_map(|id| self.cancel(id)));
        frames
    }

    /// Nothing in flight and nobody listening, so this server may stop.
    pub(crate) fn is_idle(&self) -> bool {
        self.is_quiet() && self.subscribers.is_empty()
    }

    /// No agent is waiting on this server.
    pub(crate) fn is_quiet(&self) -> bool {
        self.pending.is_empty()
    }

    /// A frame from the server.
    pub(crate) fn inbound(&mut self, frame: Value) -> Vec<Action> {
        let method = frame["method"].as_str().map(str::to_owned);
        match (method, frame.get("id").filter(|id| !id.is_null())) {
            (Some(method), Some(id)) => self.server_request(&method, id.clone(), &frame["params"]),
            (Some(method), None) => self.notification(&method, frame),
            (None, Some(id)) => self.answer(id, &frame),
            (None, None) => Vec::new(),
        }
    }

    fn answer(&mut self, id: &Value, frame: &Value) -> Vec<Action> {
        let Some(id) = id.as_u64() else {
            return Vec::new();
        };
        self.settle(id);
        let outcome = match frame.get("error").filter(|error| !error.is_null()) {
            Some(error) => Err(error.clone()),
            None => Ok(frame["result"].clone()),
        };
        vec![Action::Answered { id, outcome }]
    }

    fn server_request(&mut self, method: &str, id: Value, params: &Value) -> Vec<Action> {
        if method == "ping" {
            return vec![Action::Reply(json!({"jsonrpc":"2.0","id":id,"result":{}}))];
        }
        if !SERVER_REQUESTS.contains(&method) {
            let refused = refusal(&id, -32601, format!("{method} isn't available here"));
            return vec![Action::Reply(refused)];
        }
        if self.reach == Reach::Shared {
            return vec![
                Action::Reply(untraceable(&id, method)),
                Action::Demote {
                    method: method.to_owned(),
                },
            ];
        }
        // The folder is ours to answer: it's why this process exists.
        if method == "roots/list" {
            if let Some(uri) = &self.root {
                let roots = json!({"jsonrpc":"2.0","id":id,"result":{"roots":[{"uri":uri}]}});
                return vec![Action::Reply(roots)];
            }
        }
        // Old protocol versions don't say which call a request belongs to, so
        // it's known only when exactly one session has anything in flight.
        let callers: BTreeSet<_> = self.pending.values().map(|call| call.caller).collect();
        let mut callers = callers.into_iter();
        match (callers.next(), callers.next()) {
            (Some(caller), None) => vec![Action::Ask {
                caller,
                upstream_id: id,
                method: method.to_owned(),
                params: params.clone(),
            }],
            _ => vec![Action::Reply(untraceable(&id, method))],
        }
    }

    fn notification(&mut self, method: &str, mut frame: Value) -> Vec<Action> {
        match method {
            "notifications/progress" => {
                let token = frame["params"]["progressToken"].as_str();
                let Some((caller, original)) = token.and_then(|t| self.progress.get(t).cloned())
                else {
                    return Vec::new();
                };
                frame["params"]["progressToken"] = original;
                vec![Action::Notify {
                    caller,
                    message: frame,
                }]
            }
            "notifications/tools/list_changed"
            | "notifications/resources/list_changed"
            | "notifications/prompts/list_changed" => vec![Action::ListChanged {
                method: method.to_owned(),
            }],
            "notifications/resources/updated" => {
                let uri = frame["params"]["uri"].as_str().unwrap_or("");
                match self.subscribers.get(uri) {
                    Some(callers) => vec![Action::ResourceUpdated {
                        callers: callers.clone(),
                        message: frame,
                    }],
                    None => Vec::new(),
                }
            }
            "notifications/message" => vec![Action::Log {
                level: frame["params"]["level"]
                    .as_str()
                    .unwrap_or("info")
                    .to_owned(),
                message: frame,
            }],
            _ => Vec::new(),
        }
    }
}

/// Severity order from RFC 5424, for filtering logs per agent.
pub(crate) fn log_rank(level: &str) -> u8 {
    match level {
        "debug" => 0,
        "notice" => 2,
        "warning" => 3,
        "error" => 4,
        "critical" => 5,
        "alert" => 6,
        "emergency" => 7,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Router {
        Router::new(Reach::Shared, None)
    }

    #[test]
    fn an_input_a_server_asks_for_is_answered_asked_or_refused() {
        let folder = Router::new(Reach::Workspace, Some("file:///work".into()));
        assert_eq!(
            folder.input("roots/list"),
            Input::Answer(json!({"roots":[{"uri":"file:///work"}]}))
        );
        assert_eq!(folder.input("elicitation/create"), Input::Ask);
        // Only the agent knows the roots of a process with no folder of its own.
        assert_eq!(shared().input("roots/list"), Input::Ask);
        assert_eq!(shared().input("sampling/createMessage"), Input::Ask);
        assert!(matches!(shared().input("tools/call"), Input::Refuse(_)));
    }

    #[test]
    fn two_agents_with_the_same_id_never_collide_upstream() {
        let mut router = shared();
        // Each agent numbers its own requests, so both would send id 1.
        let (first, first_frame) = router.request(1, "tools/call", json!({"name":"a"}));
        let (second, second_frame) = router.request(2, "tools/call", json!({"name":"b"}));
        assert_ne!(first, second);
        assert_ne!(first_frame["id"], second_frame["id"]);
        assert_eq!(
            router.inbound(json!({"jsonrpc":"2.0","id":second,"result":{"from":"b"}})),
            [Action::Answered {
                id: second,
                outcome: Ok(json!({"from":"b"}))
            }]
        );
        assert_eq!(
            router.inbound(json!({"jsonrpc":"2.0","id":first,"error":{"code":1}})),
            [Action::Answered {
                id: first,
                outcome: Err(json!({"code":1}))
            }]
        );
        assert!(router.is_idle());
    }

    #[test]
    fn an_id_the_hub_never_sent_changes_nothing() {
        let mut router = shared();
        let (id, _) = router.request(1, "tools/call", json!({}));
        assert!(router.inbound(json!({"id":"1","result":{}})).is_empty());
        assert!(!router.is_idle());
        assert_eq!(router.inbound(json!({"id":id,"result":{}})).len(), 1);
    }

    #[test]
    fn progress_tokens_are_rewritten_and_mapped_back() {
        let mut router = shared();
        let (a, a_frame) = router.request(1, "tools/call", json!({"_meta":{"progressToken":7}}));
        let (_, b_frame) = router.request(2, "tools/call", json!({"_meta":{"progressToken":7}}));
        let ours_a = a_frame["params"]["_meta"]["progressToken"].clone();
        let ours_b = b_frame["params"]["_meta"]["progressToken"].clone();
        assert_ne!(ours_a, ours_b);
        assert_ne!(ours_a, json!(7));
        let notified = router.inbound(
            json!({"method":"notifications/progress","params":{"progressToken":ours_b,"progress":1}}),
        );
        assert_eq!(
            notified,
            [Action::Notify {
                caller: 2,
                message: json!({"method":"notifications/progress","params":{"progressToken":7,"progress":1}}),
            }]
        );
        // Once the request ends, its token means nothing.
        router.inbound(json!({"id":a,"result":{}}));
        assert!(router
            .inbound(json!({"method":"notifications/progress","params":{"progressToken":ours_a}}))
            .is_empty());
    }

    #[test]
    fn a_shared_server_cannot_ask_an_agent_anything_and_is_demoted() {
        let mut router = shared();
        router.request(1, "tools/call", json!({}));
        for method in SERVER_REQUESTS {
            let actions = router.inbound(json!({"id":"s1","method":method,"params":{}}));
            let [Action::Reply(reply), Action::Demote { method: asked }] = actions.as_slice()
            else {
                panic!("{actions:?}");
            };
            assert_eq!(reply["id"], "s1");
            assert!(reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("traced"));
            assert_eq!(asked, method);
        }
    }

    #[test]
    fn a_request_with_two_possible_callers_fails_rather_than_guesses() {
        let mut router = Router::new(Reach::Session, None);
        router.request(1, "tools/call", json!({}));
        router.request(2, "tools/call", json!({}));
        let actions = router.inbound(json!({"id":9,"method":"sampling/createMessage","params":{}}));
        let [Action::Reply(reply)] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(reply["error"]["code"], -32603);
    }

    #[test]
    fn a_request_with_no_call_in_flight_fails_too() {
        let mut router = Router::new(Reach::Session, None);
        let actions = router.inbound(json!({"id":9,"method":"elicitation/create","params":{}}));
        assert!(
            matches!(actions.as_slice(), [Action::Reply(reply)] if reply["error"]["code"] == -32603)
        );
    }

    #[test]
    fn a_request_with_one_caller_goes_to_that_caller() {
        let mut router = Router::new(Reach::Session, None);
        router.request(5, "tools/call", json!({}));
        assert_eq!(
            router.inbound(
                json!({"id":9,"method":"sampling/createMessage","params":{"maxTokens":1}})
            ),
            [Action::Ask {
                caller: 5,
                upstream_id: json!(9),
                method: "sampling/createMessage".into(),
                params: json!({"maxTokens":1}),
            }]
        );
    }

    #[test]
    fn roots_come_from_the_workspace_this_process_serves() {
        let mut router = Router::new(Reach::Workspace, Some("file:///work/a".into()));
        let actions = router.inbound(json!({"id":3,"method":"roots/list"}));
        let [Action::Reply(reply)] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(reply["result"]["roots"][0]["uri"], "file:///work/a");
    }

    #[test]
    fn subscriptions_reach_the_server_once_however_many_agents_hold_them() {
        let mut router = shared();
        let first = router.subscribe(1, "file:///a").unwrap();
        assert_eq!(first["method"], "resources/subscribe");
        assert!(router.subscribe(2, "file:///a").is_none());
        assert!(
            router.subscribe(2, "file:///a").is_none(),
            "a repeat counts once"
        );
        let update =
            json!({"method":"notifications/resources/updated","params":{"uri":"file:///a"}});
        assert_eq!(
            router.inbound(update.clone()),
            [Action::ResourceUpdated {
                callers: BTreeSet::from([1, 2]),
                message: update.clone(),
            }]
        );
        assert!(router.unsubscribe(1, "file:///a").is_none());
        assert!(router.unsubscribe(1, "file:///a").is_none(), "already gone");
        assert!(!router.is_idle(), "a subscriber keeps the server up");
        let last = router.unsubscribe(2, "file:///a").unwrap();
        assert_eq!(last["method"], "resources/unsubscribe");
        assert!(router.is_idle());
        assert!(router.inbound(update).is_empty());
    }

    #[test]
    fn a_session_that_ends_gives_up_what_it_held() {
        let mut router = shared();
        router.subscribe(1, "file:///mine");
        router.subscribe(1, "file:///both");
        router.subscribe(2, "file:///both");
        let (id, _) = router.request(1, "tools/call", json!({}));
        let frames = router.forget(1);
        let methods: Vec<_> = frames.iter().map(|frame| frame["method"].clone()).collect();
        assert_eq!(
            methods,
            ["resources/unsubscribe", "notifications/cancelled"]
        );
        assert_eq!(frames[0]["params"]["uri"], "file:///mine");
        assert_eq!(frames[1]["params"]["requestId"], id);
        assert!(!router.is_idle(), "the other session is still subscribed");
    }

    #[test]
    fn cancelling_tells_the_server_with_the_hubs_id() {
        let mut router = shared();
        let (id, _) = router.request(1, "tools/call", json!({}));
        let cancelled = router.cancel(id).unwrap();
        assert_eq!(cancelled["params"]["requestId"], id);
        assert!(router.cancel(id).is_none());
        assert!(router.is_idle());
    }

    #[test]
    fn list_changes_and_logs_are_handed_up_for_fan_out() {
        let mut router = shared();
        assert_eq!(
            router.inbound(json!({"method":"notifications/tools/list_changed"})),
            [Action::ListChanged {
                method: "notifications/tools/list_changed".into()
            }]
        );
        let logged =
            router.inbound(json!({"method":"notifications/message","params":{"level":"error"}}));
        assert!(matches!(&logged[0], Action::Log { level, .. } if level == "error"));
        assert!(log_rank("error") > log_rank("info"));
    }

    #[test]
    fn unknown_server_requests_are_refused_and_pings_answered() {
        let mut router = shared();
        let actions = router.inbound(json!({"id":1,"method":"made/up"}));
        assert!(
            matches!(actions.as_slice(), [Action::Reply(reply)] if reply["error"]["code"] == -32601)
        );
        let actions = router.inbound(json!({"id":2,"method":"ping"}));
        assert!(
            matches!(actions.as_slice(), [Action::Reply(reply)] if reply["result"] == json!({}))
        );
    }
}
