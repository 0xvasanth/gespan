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

/// The `app` surface's help: usage, every action's fields from
/// `actions`, and the rules that span actions.
pub fn help() -> &'static str {
    static HELP: OnceLock<String> = OnceLock::new();
    HELP.get_or_init(|| {
        let mut lines = vec![
            "Gespan app access".to_owned(),
            "Usage: gespan-gpui app ACTION --json '{...}' [--request-id ID]".into(),
            "Pass --input FILE or --input - to read a JSON object instead.".into(),
            "Run gespan-gpui app help ACTION for one action's input schema and an example that works. Fields marked ? are optional.".into(),
            "Actions:".into(),
        ];
        let mut namespaces: Vec<(&str, Vec<String>)> = Vec::new();
        for action in super::actions::all() {
            let namespace = action.name.split('.').next().unwrap_or_default();
            if matches!(namespace, "ticket" | "card" | "chat") {
                continue;
            }
            match namespaces.iter_mut().find(|(n, _)| *n == namespace) {
                Some((_, entries)) => entries.push(action.signature()),
                None => namespaces.push((namespace, vec![action.signature()])),
            }
        }
        lines.extend(namespaces.into_iter().map(|(_, entries)| entries.join("; ")));
        lines.push(HELP_RULES.into());
        lines.join("\n")
    })
}
const HELP_RULES: &str = r#"chat.card {type:"pr",repo?,number,note?} / {type:"session",sessionId,note?} / {type:"choices",options:[text]} / {type:"habit",name,instructions,schedule}
A Pilot's SOPs, on its SOP Roadmap tab, are Flows. flows.save creates one; to change one, flows.read it and save the whole Flow with its id.
Ordinary Operator sessions are visible in project history. Pilot launches follow its Show started sessions preference. reveal defaults to false and controls navigation, not history visibility. placement right/down explicitly opens a split beside besideSessionId (or the caller).
sessions.read returns up to three prose exchanges and nextBefore for older turns; pass that turn ID as before.
notes.write requires body for a new note; existing notes accept title, body or tags independently.
ticket.get {key?}; ticket.comment {body,key?}; ticket.status {} lists the statuses; ticket.status {target,fields?,key?} moves the ticket
card.subtasks {}; card.subtask_done {subtaskId,done}
ticket.* and card.* act on this session's own linked ticket and cards, in any agent turn, without Operator. Orchestration leads and workers have neither. Read-only turns may read and list only.
Artifacts, memory, soul, habits, cards and notifyOnComplete:true require a Pilot.
The CLI returns {ok,result,requestId}. Retry uncertain mutations with the same request ID and identical input.
Assigned projects alone may be addressed."#;
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
            Self::App => help(),
            Self::Control => crate::orchestration::CONTROL_HELP,
        }
    }
    /// The surface's help as a tool caller reads it: the same actions, called
    /// with fields instead of flags.
    fn tool_help(self) -> String {
        let tool = self.tool();
        format!(
            "Through the {tool} tool, ACTION below is its `action` field, the JSON object is `input` and a request ID is `requestId`. The command line usage and its flags don't apply. A reply is an object with `ok`, `result` and `requestId`, and a failure is a tool error whose text ends with the request ID.{}\n\n{}",
            if self == Self::App {
                " Call app_help with {\"action\": ACTION} for one action's input schema and an example that works."
            } else {
                ""
            },
            self.help()
        )
    }
    /// What the help tool returns: one action's schema, or every action.
    fn help_for(self, action: Option<&Value>) -> Result<Value, String> {
        match (self, action) {
            (_, None | Some(Value::Null)) | (Self::Control, _) => {
                Ok(json!({"help":self.tool_help()}))
            }
            (Self::App, Some(Value::String(name))) => super::actions::find(name)
                .map(super::actions::Action::describe)
                .ok_or_else(|| super::api::unknown_action(name)),
            (Self::App, Some(_)) => Err("action must be an action name".into()),
        }
    }
    fn unavailable(self) -> &'static str {
        match self {
            Self::App => "App closed",
            Self::Control => {
                "No Gespan connection. Confirm the Orchestrator proposal in Gespan first."
            }
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
        let request_id = json!({"type":"string","maxLength":128,"description":"Reuse when retrying an uncertain mutation, with identical input"});
        match self.0 {
            Surface::App => {
                let names: Vec<&str> = super::actions::all().iter().map(|a| a.name).collect();
                vec![
                    json!({"name":"app_help","description":"With action, return that app action's input JSON Schema, which fields are required, and an app_call that works. Without it, list every action. SOPs are Flows: see flows.save.","inputSchema":{"type":"object","properties":{
                        "action":{"enum":names,"description":"The action to describe"},
                    },"additionalProperties":false},"annotations":{"readOnlyHint":true,"openWorldHint":false}}),
                    json!({"name":"app_call","description":"Act on Gespan itself: sessions, worktrees, folders, notes, memory, soul, habits, and your SOPs (Flows). Each action takes its own input object; before an action's first call, or after an error, call app_help with {\"action\": ACTION} for its schema.","inputSchema":{"type":"object","properties":{
                        "action":{"enum":names},
                        "input":{"type":"object","description":"The action's own fields, as app_help describes them. Not the action name."},
                        "requestId":request_id,
                    },"required":["action"],"additionalProperties":false},"annotations":{"openWorldHint":false}}),
                ]
            }
            Surface::Control => vec![
                json!({"name":"control_help","description":"List every orchestration action and the JSON each takes, with the retry rule.","inputSchema":{"type":"object","properties":{},"additionalProperties":false},"annotations":{"readOnlyHint":true,"openWorldHint":false}}),
                json!({"name":"control_call","description":"Supervise this orchestration run. Read control_help first for the actions.","inputSchema":{"type":"object","properties":{
                    "action":{"type":"string"},
                    "input":{"type":"object","description":"The action's JSON fields"},
                    "requestId":request_id,
                },"required":["action"],"additionalProperties":false},"annotations":{"openWorldHint":false}}),
            ],
        }
    }
    fn call(&self, grant: &mcp_hub::Grant, tool: &str, arguments: Value) -> Result<Value, String> {
        if tool == self.0.help_tool() {
            return self.0.help_for(arguments.get("action"));
        }
        let action = arguments["action"]
            .as_str()
            .ok_or("action must be a string")?;
        let input = match &arguments["input"] {
            Value::Null => json!({}),
            input if input.is_object() => input.clone(),
            _ => return Err("input must be a JSON object".into()),
        };
        // `app_call {action:"app_help", input:{action}}` is a call meant for
        // the help tool: answer it rather than fail.
        if self.0 == Surface::App && matches!(action, "app_help" | "help") {
            return self.0.help_for(input.get("action"));
        }
        let request_id = match &arguments["requestId"] {
            Value::Null => super::new_id(),
            Value::String(id) if !id.is_empty() && id.len() <= 128 => id.clone(),
            _ => return Err("requestId must contain 1–128 bytes".into()),
        };
        let result = if self.0 == Surface::App && crate::ticket_bridge::is_app_action(action) {
            crate::ticket_bridge::app_call(grant.session, action, &input, &request_id)
        } else if self.0 == Surface::App {
            precheck(action, &input)
                .and_then(|()| dispatch(self.0, grant.session, &request_id, action, input))
        } else {
            dispatch(self.0, grant.session, &request_id, action, input)
        };
        match result {
            Ok(value) => Ok(json!({"ok":true,"result":value,"requestId":request_id})),
            Err(error) => Err(format!("{error} (requestId: {request_id})")),
        }
    }
}

/// The app's own input check, run before the call reaches the app so a
/// malformed input fails at once and says how to call the action.
fn precheck(action: &str, input: &Value) -> Result<(), String> {
    super::api::validate(action, input).map_err(|error| match super::actions::find(action) {
        Some(_) => format!("{error}. {}", super::actions::hint(action)),
        None => error,
    })
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
        .ok_or(surface.unavailable())?;
    let (reply, result) = mpsc::channel();
    calls
        .send_blocking(Call {
            session,
            request_id: request_id.to_owned(),
            action: action.to_owned(),
            input,
            reply,
        })
        .map_err(|_| surface.unavailable())?;
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
        return match args.get(1) {
            Some(action) if surface == Surface::App => Ok(surface
                .help_for(Some(&json!(action)))
                .unwrap_or_else(|error| json!({"ok":false,"error":error}))),
            _ => Ok(json!({"help":surface.help()})),
        };
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
    let (tool, arguments) = match cli_route(surface, &action, input, &request_id) {
        Ok(route) => route,
        Err(error) => return Ok(json!({"ok":false,"error":error,"requestId":request_id})),
    };
    let wrapped = tool == surface.tool();
    let reply = crate::tool_servers::call_tool(tool, arguments).map(|reply| {
        if wrapped {
            reply["result"].clone()
        } else {
            reply
        }
    });
    Ok(cli_reply(reply, &request_id))
}
/// The tool behind a CLI call. `ticket.*` and `card.*` go straight to the
/// ticket toolkit: a session linked to a ticket holds that without holding
/// `app`.
fn cli_route(
    surface: Surface,
    action: &str,
    mut input: Value,
    request_id: &str,
) -> Result<(&'static str, Value), String> {
    if surface == Surface::App && crate::ticket_bridge::is_app_action(action) {
        let tool = crate::ticket_bridge::app_tool(action)?;
        let fields = input.as_object_mut().ok_or("Input must be a JSON object")?;
        fields.insert("requestId".into(), request_id.into());
        return Ok((tool, input));
    }
    Ok((
        surface.tool(),
        json!({"action":action,"input":input,"requestId":request_id}),
    ))
}
fn cli_reply(reply: Result<Value, String>, request_id: &str) -> Value {
    match reply {
        Ok(result) => json!({"ok":true,"result":result,"requestId":request_id}),
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

    #[test]
    fn a_tickets_cli_actions_need_only_the_ticket_toolkit() {
        let (tool, arguments) = cli_route(
            Surface::App,
            "ticket.comment",
            json!({"body":"Done"}),
            "request-1",
        )
        .unwrap();
        assert_eq!(tool, "ticket_comment");
        assert_eq!(arguments, json!({"body":"Done","requestId":"request-1"}));
        let (tool, _) = cli_route(Surface::App, "card.subtasks", json!({}), "r").unwrap();
        assert_eq!(tool, "card_subtasks");
    }

    #[test]
    fn other_cli_actions_stay_on_their_own_surface() {
        let (tool, arguments) =
            cli_route(Surface::App, "sessions.list", json!({"limit":1}), "r").unwrap();
        assert_eq!(tool, "app_call");
        assert_eq!(
            arguments,
            json!({"action":"sessions.list","input":{"limit":1},"requestId":"r"})
        );
        let (tool, _) = cli_route(Surface::Control, "ticket.get", json!({}), "r").unwrap();
        assert_eq!(tool, "control_call", "only the app surface has tickets");
    }

    #[test]
    fn a_cli_route_refuses_what_no_tool_could_take() {
        for (action, input) in [("ticket.unknown", json!({})), ("ticket.get", json!("text"))] {
            assert!(
                cli_route(Surface::App, action, input, "r").is_err(),
                "{action}"
            );
        }
    }

    #[test]
    fn the_cli_prints_ok_or_the_error_without_repeating_the_request_id() {
        assert_eq!(
            cli_reply(Ok(json!({"id":1})), "r"),
            json!({"ok":true,"result":{"id":1},"requestId":"r"})
        );
        assert_eq!(
            cli_reply(Err("App closed (requestId: r)".to_owned()), "r"),
            json!({"ok":false,"error":"App closed","requestId":"r"})
        );
        assert_eq!(
            cli_reply(Err("Not reachable".to_owned()), "r"),
            json!({"ok":false,"error":"Not reachable","requestId":"r"})
        );
    }

    #[test]
    fn help_through_a_tool_describes_its_fields() {
        let grant = |toolkit: &str| mcp_hub::Grant {
            session: 75,
            workspace: "/work".into(),
            toolkits: [toolkit.to_owned()].into(),
            servers: Default::default(),
        };
        for (toolkit, tool, surface) in [
            (Toolkit::APP, "app_help", crate::tool_servers::APP),
            (
                Toolkit::CONTROL,
                "control_help",
                crate::tool_servers::CONTROL,
            ),
        ] {
            let help = toolkit.call(&grant(surface), tool, json!({})).unwrap();
            let help = help["help"].as_str().unwrap();
            assert!(
                help.contains("`requestId`") && help.contains("`input`"),
                "{help}"
            );
        }
    }

    #[test]
    fn app_help_describes_one_action_with_a_call_that_works() {
        let described = Toolkit::APP
            .call(&grant(80), "app_help", json!({"action":"memory.add"}))
            .unwrap();
        assert_eq!(described["input"]["required"], json!(["fact"]));
        assert_eq!(
            described["example"],
            json!({"action":"memory.add","input":{"fact":"The user ships releases on Fridays.","topic":"decisions"}})
        );
        let flow = Toolkit::APP
            .call(&grant(80), "app_help", json!({"action":"flows.save"}))
            .unwrap();
        assert_eq!(
            flow["input"]["properties"]["flow"]["properties"]["states"]["items"]["oneOf"][0]
                ["required"],
            json!(["id", "title", "harness", "model", "brief", "transitions"])
        );
        let unknown = Toolkit::APP
            .call(&grant(80), "app_help", json!({"action":"flow.save"}))
            .unwrap_err();
        assert_eq!(
            unknown,
            "Unknown app action flow.save. Did you mean flows.save?"
        );
    }

    /// A model seen calling `app_call {action:"app_help", input:{action}}`
    /// gets the help it meant instead of an unknown-action error.
    #[test]
    fn help_asked_through_app_call_is_answered() {
        let described = Toolkit::APP
            .call(
                &grant(81),
                "app_call",
                json!({"action":"app_help","input":{"action":"memory.add"}}),
            )
            .unwrap();
        assert_eq!(described["action"], "memory.add");
        let all = Toolkit::APP
            .call(&grant(81), "app_call", json!({"action":"app_help"}))
            .unwrap();
        assert!(all["help"]
            .as_str()
            .unwrap()
            .contains("memory.add {fact,topic?,until?}"));
    }

    /// The guesses from a real Pilot transcript, which each cost a round trip
    /// to the app: now refused before dispatch, naming what to call.
    #[test]
    fn a_malformed_input_fails_before_the_app_and_says_how_to_call_it() {
        let (server, rx) = Server::start().unwrap();
        let _ = server.environment(82);
        for (input, error) in [
            (
                json!({"text":"Ships on Fridays"}),
                "memory.add does not take text",
            ),
            (
                json!({"content":"Ships on Fridays"}),
                "memory.add does not take content",
            ),
            (json!({"topic":"decisions"}), "memory.add needs fact"),
        ] {
            let refused = Toolkit::APP
                .call(
                    &grant(82),
                    "app_call",
                    json!({"action":"memory.add","input":input,"requestId":"r"}),
                )
                .unwrap_err();
            assert_eq!(
                refused,
                format!("{error}. Call the app_help tool with {{\"action\":\"memory.add\"}} for its input schema and an example that works. (requestId: r)")
            );
        }
        let unknown = Toolkit::APP
            .call(
                &grant(82),
                "app_call",
                json!({"action":"flows.create","requestId":"r"}),
            )
            .unwrap_err();
        assert!(
            unknown.starts_with("Unknown app action flows.create. flows actions are flows.list, flows.read, flows.save,"),
            "{unknown}"
        );
        assert!(rx.try_recv().is_err(), "nothing reached the app");
    }

    #[test]
    fn the_app_tools_list_every_action_by_name() {
        let tools = Toolkit::APP.tools(&grant(83));
        let names = |tool: &Value, path: &str| -> Vec<String> {
            serde_json::from_value(tool["inputSchema"]["properties"][path]["enum"].clone()).unwrap()
        };
        let all: Vec<String> = super::super::actions::all()
            .iter()
            .map(|a| a.name.to_owned())
            .collect();
        assert_eq!(tools[0]["name"], "app_help");
        assert_eq!(names(&tools[0], "action"), all);
        assert_eq!(tools[1]["name"], "app_call");
        assert_eq!(names(&tools[1], "action"), all);
        for action in ["flows.save", "memory.add", "ticket.comment"] {
            assert!(all.iter().any(|a| a == action), "{action}");
        }
        let help = help();
        for action in super::super::actions::all() {
            assert!(help.contains(action.name), "{}", action.name);
        }
    }

    #[test]
    fn the_cli_describes_one_action() {
        let value = client(&["help".to_owned(), "flows.start".to_owned()]).unwrap();
        assert_eq!(
            value["input"]["required"],
            json!(["flowId", "goal", "task"])
        );
        let value = client(&["help".to_owned(), "flow.start".to_owned()]).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(
            value["error"],
            "Unknown app action flow.start. Did you mean flows.start?"
        );
        let value = client(&["help".to_owned()]).unwrap();
        assert!(value["help"].as_str().unwrap().contains("app help ACTION"));
    }

    #[test]
    fn a_lead_with_no_route_is_told_to_confirm_the_proposal() {
        let refused = dispatch(Surface::Control, 76, "r", "list", json!({})).unwrap_err();
        assert!(
            refused.contains("Confirm the Orchestrator proposal"),
            "{refused}"
        );
        let refused = dispatch(Surface::App, 76, "r", "models.list", json!({})).unwrap_err();
        assert_eq!(refused, "App closed");
    }

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
        assert!(refused.starts_with("No Gespan connection"), "{refused}");
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
            json!({"ok":true,"result":{"models":[]},"requestId":"r-7"})
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
                json!({"action":"sessions.send","input":{"sessionId":"5","prompt":"Go on"},"requestId":"r-2"}),
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
