//! Session-scoped local app access, served through the MCP hub as the `app`
//! and `control` toolkits. All requests are checked again against the Pilot's
//! project list. An agent that mounts no MCP servers reaches the same tools
//! through the `app` and `control` CLIs.
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::Read,
    sync::{mpsc, Mutex, OnceLock},
    time::Duration,
};

pub const HELP: &str = r#"Gespan app access
Usage: gespan-gpui app ACTION --json '{...}' [--request-id ID]
Pass --input FILE or --input - to read a JSON object instead.
Actions:
models.list {}
sessions.list {project?}
sessions.read {sessionId,project?,before?,limit?,maxChars?}
sessions.start {prompt,project?,harness?,model?,modelSettings?,effort?,runtimeMode?,title?,draft?,reveal?,workspaceMode?,worktreeBase?,worktreeCwd?,placement?,besideSessionId?,notifyOnComplete?}
sessions.send {sessionId,prompt,project?,notifyOnComplete?}
sessions.draft {sessionId,prompt,project?}
sessions.stop / sessions.archive / sessions.delete {sessionId,project?}
worktrees.list {project?}
worktrees.create {branch,base?,existing?,project?}
folders.list {project?}
folders.move {sessionId,folderId?,newFolderName?,project?}
notes.list {limit?,offset?}; notes.read {id}; notes.write {id?,title?,body?,tags?}
artifacts.list {kind?,limit?,offset?}; artifacts.read {id}; artifacts.write {id?,kind,title,body,summary?}
soul.read {}; soul.update {text,expectedHash}
memory.read {topic?}; memory.search {query,since?}; memory.add {fact,topic?,until?}; memory.replace {find,fact,topic?,until?}; memory.remove {find,topic?}
habits.list {}; habits.add {name,instructions,schedule:{kind,time?,minute?,dayOfWeek?}}; habits.update {id,name?,instructions?,schedule?,enabled?}; habits.run {id}; habits.remove {id}
flows.list {}; flows.read {flowId}; flows.save {flow}; flows.delete {flowId}; flows.start {flowId,goal,task,inputs?}; flows.runs {goal?}; flows.run {runId}; flows.editRun {runId,flow,note}; flows.cancel {runId}; flows.resume {runId,stateId}
chat.card {type:"pr",repo?,number,note?} / {type:"session",sessionId,note?} / {type:"choices",options:[text]} / {type:"habit",name,instructions,schedule}
Ordinary Operator sessions are visible in project history. Pilot launches follow its Show started sessions preference. reveal defaults to false and controls navigation, not history visibility. placement right/down explicitly opens a split beside besideSessionId (or the caller).
sessions.read returns up to three prose exchanges and nextBefore for older turns; pass that turn ID as before.
notes.write requires body for a new note; existing notes accept title, body or tags independently.
ticket.get {key?}; ticket.comment {body,key?}; ticket.status {} lists the statuses; ticket.status {target,fields?,key?} moves the ticket
card.subtasks {}; card.subtask_done {subtaskId,done}
ticket.* and card.* act on this session's own linked ticket and cards, in any agent turn, without Operator. Orchestration leads and workers have neither. Read-only turns may read and list only.
Artifacts, memory, soul, habits, cards and notifyOnComplete:true require a Pilot.
The CLI returns {ok,result,requestId}. Retry uncertain mutations with the same request ID and identical input.
Assigned projects alone may be addressed.
"#;
pub struct Call {
    pub session: i64,
    pub request_id: String,
    pub action: String,
    pub input: Value,
    pub reply: mpsc::Sender<Result<Value, String>>,
}
/// Which sessions each app dispatcher serves, by surface. An app window, the
/// Pilots and an orchestration run each hold their own.
type Routes = HashMap<(Surface, i64), (u64, async_channel::Sender<Call>)>;

fn routes() -> &'static Mutex<Routes> {
    static ROUTES: OnceLock<Mutex<Routes>> = OnceLock::new();
    ROUTES.get_or_init(Default::default)
}

/// One dispatcher of app calls. Dropping it ends what it served.
pub struct Server {
    id: u64,
    calls: async_channel::Sender<Call>,
}
impl Drop for Server {
    fn drop(&mut self) {
        routes()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, (id, _)| *id != self.id);
    }
}
impl Server {
    pub fn start() -> Result<(Self, async_channel::Receiver<Call>), String> {
        let (calls, rx) = async_channel::bounded(32);
        let id = rand::random();
        Ok((Self { id, calls }, rx))
    }
    fn route(&self, surface: Surface, session: i64) -> HashMap<String, String> {
        routes()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert((surface, session), (self.id, self.calls.clone()));
        HashMap::from([(surface.marker().to_owned(), session.to_string())])
    }
    /// Marks a session as holding app access. Carries no secret: the session's
    /// hub credential is what proves the caller.
    pub fn environment(&self, session: i64) -> HashMap<String, String> {
        self.route(Surface::App, session)
    }
    /// A lead's capability: workers never hold it.
    pub fn control_environment(&self, session: i64) -> HashMap<String, String> {
        self.route(Surface::Control, session)
    }
}

/// One toolkit on this route: `app`, or a lead's `control`.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum Surface {
    App,
    Control,
}
pub const APP_MARKER: &str = "GESPAN_APP_SESSION";
pub const CONTROL_MARKER: &str = "GESPAN_CONTROL_SESSION";
impl Surface {
    fn marker(self) -> &'static str {
        match self {
            Self::App => APP_MARKER,
            Self::Control => CONTROL_MARKER,
        }
    }
    fn tool(self) -> &'static str {
        match self {
            Self::App => "app_call",
            Self::Control => "control_call",
        }
    }
    fn help(self) -> &'static str {
        match self {
            Self::App => HELP,
            Self::Control => crate::orchestration::CONTROL_HELP,
        }
    }
    fn help_tool(self) -> &'static str {
        match self {
            Self::App => "app_help",
            Self::Control => "control_help",
        }
    }
}

/// The app's actions as tools, for a grant that names them.
pub(crate) struct Toolkit(Surface);
impl Toolkit {
    pub(crate) const APP: Self = Self(Surface::App);
    pub(crate) const CONTROL: Self = Self(Surface::Control);
}
impl mcp_hub::Toolkit for Toolkit {
    fn name(&self) -> &'static str {
        match self.0 {
            Surface::App => crate::tool_servers::APP,
            Surface::Control => crate::tool_servers::CONTROL,
        }
    }
    fn tools(&self, _: &mcp_hub::Grant) -> Vec<Value> {
        let (help, call) = match self.0 {
            Surface::App => (
                "List every Gespan app action and the JSON each takes.",
                "Act on Gespan itself: sessions, worktrees, folders, notes and more. Read app_help first for the actions.",
            ),
            Surface::Control => (
                "List every orchestration action and the JSON each takes, with the retry rule.",
                "Supervise this orchestration run. Read control_help first for the actions.",
            ),
        };
        vec![
            json!({"name":self.0.help_tool(),"description":help,"inputSchema":{"type":"object","properties":{},"additionalProperties":false},"annotations":{"readOnlyHint":true,"openWorldHint":false}}),
            json!({"name":self.0.tool(),"description":call,"inputSchema":{"type":"object","properties":{
                "action":{"type":"string"},
                "input":{"type":"object","description":"The action's JSON fields"},
                "requestId":{"type":"string","maxLength":128,"description":"Reuse when retrying an uncertain mutation, with identical input"},
            },"required":["action"],"additionalProperties":false},"annotations":{"openWorldHint":false}}),
        ]
    }
    fn call(&self, grant: &mcp_hub::Grant, tool: &str, arguments: Value) -> Result<Value, String> {
        if tool == self.0.help_tool() {
            return Ok(json!({"help":self.0.help()}));
        }
        let action = arguments["action"]
            .as_str()
            .ok_or("action must be a string")?;
        let input = match &arguments["input"] {
            Value::Null => json!({}),
            input if input.is_object() => input.clone(),
            _ => return Err("input must be a JSON object".into()),
        };
        let request_id = match &arguments["requestId"] {
            Value::Null => super::new_id(),
            Value::String(id) if !id.is_empty() && id.len() <= 128 => id.clone(),
            _ => return Err("requestId must contain 1–128 bytes".into()),
        };
        let result = if self.0 == Surface::App && crate::ticket_bridge::is_app_action(action) {
            crate::ticket_bridge::app_call(grant.session, action, &input, &request_id)
        } else {
            dispatch(self.0, grant.session, &request_id, action, input)
        };
        match result {
            Ok(value) => Ok(json!({"result":value,"requestId":request_id})),
            Err(error) => Err(format!("{error} (requestId: {request_id})")),
        }
    }
}

fn dispatch(
    surface: Surface,
    session: i64,
    request_id: &str,
    action: &str,
    input: Value,
) -> Result<Value, String> {
    let calls = routes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&(surface, session))
        .map(|(_, calls)| calls.clone())
        .ok_or("App closed")?;
    let (reply, result) = mpsc::channel();
    calls
        .send_blocking(Call {
            session,
            request_id: request_id.to_owned(),
            action: action.to_owned(),
            input,
            reply,
        })
        .map_err(|_| "App closed")?;
    result
        .recv_timeout(Duration::from_secs(60))
        .map_err(|_| "App request timed out; retry using the same request ID")?
}

/// Runs before any GUI initialization, so a provider tool never opens a window.
pub fn client(args: &[String]) -> Result<Value, String> {
    client_for(Surface::App, args)
}
/// The lead's `control` CLI, on the same terms.
pub fn control_client(args: &[String]) -> Result<Value, String> {
    client_for(Surface::Control, args)
}
fn client_for(surface: Surface, args: &[String]) -> Result<Value, String> {
    if args
        .first()
        .is_none_or(|s| matches!(s.as_str(), "help" | "--help" | "-h"))
    {
        return Ok(json!({"help":surface.help()}));
    }
    let action = args[0].clone();
    let mut input = json!({});
    let mut request_id = super::new_id();
    let mut args = args[1..].iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => {
                input = serde_json::from_str(args.next().ok_or("--json needs JSON")?)
                    .map_err(|e| e.to_string())?
            }
            "--input" => {
                let path = args.next().ok_or("--input needs a path")?;
                let text = if path == "-" {
                    let mut text = String::new();
                    std::io::stdin()
                        .take(1_048_577)
                        .read_to_string(&mut text)
                        .map_err(|e| e.to_string())?;
                    text
                } else {
                    std::fs::read_to_string(path).map_err(|e| e.to_string())?
                };
                if text.len() > 1_048_576 {
                    return Err("Input too large".into());
                }
                input = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            }
            "--request-id" => request_id = args.next().ok_or("--request-id needs an ID")?.clone(),
            json if json.starts_with('{') => {
                input = serde_json::from_str(json).map_err(|e| e.to_string())?
            }
            _ => return Err(format!("Unknown app argument: {arg}")),
        }
    }
    if surface == Surface::App && crate::ticket_bridge::is_app_action(&action) {
        if let Err(error) = crate::ticket_bridge::app_tool(&action) {
            return Ok(json!({"ok":false,"error":error,"requestId":request_id}));
        }
    }
    Ok(cli_reply(surface, &action, input, &request_id))
}
fn cli_reply(surface: Surface, action: &str, input: Value, request_id: &str) -> Value {
    let arguments = json!({"action":action,"input":input,"requestId":request_id});
    match crate::tool_servers::call_tool(surface.tool(), arguments) {
        Ok(reply) => json!({"ok":true,"result":reply["result"],"requestId":request_id}),
        Err(error) => json!({
            "ok":false,
            "error":error.strip_suffix(&format!(" (requestId: {request_id})")).unwrap_or(&error),
            "requestId":request_id,
        }),
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use mcp_hub::Toolkit as _;

    fn grant(session: i64) -> mcp_hub::Grant {
        mcp_hub::Grant {
            session,
            workspace: "/work".into(),
            toolkits: [crate::tool_servers::APP.to_owned()].into(),
            servers: Default::default(),
        }
    }

    #[test]
    fn unknown_ticket_and_card_actions_fail_with_the_callers_request_id() {
        for action in ["ticket.delete", "card.move"] {
            let args = [action.to_owned(), "--request-id".into(), "r-1".into()];
            let value = client(&args).unwrap();
            assert_eq!(value["ok"], false);
            assert_eq!(value["requestId"], "r-1");
            assert_eq!(
                value["error"],
                format!("Unknown app action: {action}"),
                "{action}"
            );
        }
    }
    #[test]
    fn a_control_capability_is_not_an_app_capability() {
        let (server, _calls) = Server::start().unwrap();
        let app = server.environment(7);
        let control = server.control_environment(7);
        assert_eq!(app.keys().collect::<Vec<_>>(), [APP_MARKER]);
        assert_eq!(control.keys().collect::<Vec<_>>(), [CONTROL_MARKER]);
        assert!(app
            .values()
            .chain(control.values())
            .all(|value| value == "7"));
        // A session routed for `app` alone has no route for `control`.
        let _ = server.environment(9);
        let refused = Toolkit::CONTROL
            .call(&grant(9), "control_call", json!({"action":"list"}))
            .unwrap_err();
        assert!(refused.starts_with("App closed"), "{refused}");
    }
    #[test]
    fn the_control_help_explains_every_action() {
        let value = control_client(&["--help".to_owned()]).unwrap();
        let help = value["help"].as_str().unwrap();
        for action in [
            "list", "delegate", "get", "wait", "respond", "answer", "steer", "message", "retry",
            "cancel", "review", "finish",
        ] {
            assert!(help.contains(&format!("  {action} ")), "{action}");
        }
        assert!(help.contains("--request-id"));
    }
    #[test]
    fn a_call_reaches_the_app_as_its_session_and_returns_its_request_id() {
        let (server, rx) = Server::start().unwrap();
        let _ = server.environment(70);
        let worker = std::thread::spawn(|| {
            Toolkit::APP.call(
                &grant(70),
                "app_call",
                json!({"action":"models.list","requestId":"r-7","session":71}),
            )
        });
        let call = rx.recv_blocking().unwrap();
        assert_eq!(call.session, 70);
        assert_eq!(call.action, "models.list");
        assert_eq!(call.request_id, "r-7");
        call.reply.send(Ok(json!({"models":[]}))).unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap(),
            json!({"result":{"models":[]},"requestId":"r-7"})
        );
    }
    #[test]
    fn a_failed_call_names_its_request_id_for_the_retry() {
        let (server, rx) = Server::start().unwrap();
        let _ = server.environment(72);
        let worker = std::thread::spawn(|| {
            Toolkit::APP.call(
                &grant(72),
                "app_call",
                json!({"action":"sessions.send","requestId":"r-2"}),
            )
        });
        let call = rx.recv_blocking().unwrap();
        call.reply.send(Err("Not allowed".into())).unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap_err(),
            "Not allowed (requestId: r-2)"
        );
    }
    #[test]
    fn a_closed_dispatcher_fails_at_once() {
        let (server, rx) = Server::start().unwrap();
        let _ = server.environment(73);
        drop(rx);
        let error = Toolkit::APP
            .call(&grant(73), "app_call", json!({"action":"models.list"}))
            .unwrap_err();
        assert!(error.starts_with("App closed"), "{error}");
        drop(server);
        let error = Toolkit::APP
            .call(&grant(73), "app_call", json!({"action":"models.list"}))
            .unwrap_err();
        assert!(error.starts_with("App closed"), "{error}");
    }
    #[test]
    fn the_arguments_are_checked_before_the_app_sees_them() {
        let (server, rx) = Server::start().unwrap();
        let _ = server.environment(74);
        for arguments in [
            json!({}),
            json!({"action":7}),
            json!({"action":"models.list","input":"x"}),
            json!({"action":"models.list","requestId":""}),
        ] {
            assert!(Toolkit::APP
                .call(&grant(74), "app_call", arguments)
                .is_err());
        }
        assert!(rx.try_recv().is_err());
    }
}
