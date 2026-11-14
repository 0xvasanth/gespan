//! Every app action an agent can call through `app_call` or the `app` CLI:
//! its fields, which are required, what each holds, and an input that works.
//! The input check (`api::validate`), the help text and `app_help`'s
//! per-action schemas all come from this one list, so they can't disagree.
use serde_json::{json, Value};
use std::sync::LazyLock;

/// What a field holds, and the error a wrong value gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Text,
    Bool,
    /// A non-negative integer.
    Count,
    /// A string or a non-negative integer.
    Id,
    /// A turn ID from `nextBefore`: a string or a non-negative integer.
    Turn,
    Choice(&'static [&'static str]),
    /// An object whose values are all text.
    TextMap,
    /// Up to `notes::MAX_TAGS` short strings.
    Tags,
    /// One to four strings.
    Options,
    /// Any object; the action checks its fields.
    Object,
    Schedule,
    Flow,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    pub about: &'static str,
}

#[derive(Clone, Debug)]
pub struct Action {
    pub name: &'static str,
    pub about: &'static str,
    pub fields: Vec<Field>,
    /// An `input` this action accepts.
    pub example: Value,
}

/// A placeholder for an assigned project in examples.
pub const PROJECT: &str = "/path/to/assigned/project";
const RUNTIME_MODES: &[&str] = &[
    "read-only",
    "supervised",
    "auto-accept-edits",
    "auto",
    "full-access",
];
const SCHEDULE_KINDS: &[&str] = &["hourly", "daily", "weekdays", "weekly"];

fn req(name: &'static str, ty: Ty, about: &'static str) -> Field {
    Field {
        name,
        ty,
        required: true,
        about,
    }
}
fn opt(name: &'static str, ty: Ty, about: &'static str) -> Field {
    Field {
        name,
        ty,
        required: false,
        about,
    }
}
fn project() -> Field {
    opt("project", Ty::Text, "An assigned project's absolute path")
}
fn session_id() -> Field {
    req("sessionId", Ty::Id, "A session ID from sessions.list")
}
fn action(name: &'static str, about: &'static str, fields: Vec<Field>, example: Value) -> Action {
    Action {
        name,
        about,
        fields,
        example,
    }
}

static ACTIONS: LazyLock<Vec<Action>> = LazyLock::new(|| {
    use Ty::*;
    let flow_id = || req("flowId", Text, "A Flow ID from flows.list");
    let run_id = || req("runId", Text, "A run ID from flows.runs or flows.start");
    let schedule = |required| Field {
        name: "schedule",
        ty: Schedule,
        required,
        about: "When the habit runs",
    };
    vec![
        action("models.list", "List the harnesses and models a session can use, with each model's settings.", vec![], json!({})),
        action("sessions.list", "List a project's sessions.", vec![project()], json!({"project":PROJECT})),
        action(
            "sessions.read",
            "Read up to three prose exchanges of a session. Pass the returned nextBefore as before for older turns.",
            vec![session_id(), project(), opt("before", Turn, "nextBefore from an earlier read"), opt("limit", Count, "Exchanges to return"), opt("maxChars", Count, "Character budget")],
            json!({"sessionId":"1234","project":PROJECT}),
        ),
        action(
            "sessions.start",
            "Start a session for substantial independent work.",
            vec![
                req("prompt", Text, "The task"),
                project(),
                opt("harness", Text, "A harness id from models.list"),
                opt("model", Text, "A model id from models.list"),
                opt("modelSettings", TextMap, "Setting id to value, from models.list"),
                opt("effort", Text, "An effort from models.list"),
                opt("runtimeMode", Choice(RUNTIME_MODES), "What the session may do without asking"),
                opt("title", Text, "Session title"),
                opt("draft", Bool, "Leave the prompt unsent"),
                opt("reveal", Bool, "Navigate to the session"),
                opt("workspaceMode", Choice(&["current", "checkout", "worktree"]), "Where it works"),
                opt("worktreeBase", Text, "Branch a new worktree starts from"),
                opt("worktreeCwd", Text, "An existing worktree to work in"),
                opt("placement", Choice(&["tab", "right", "down"]), "right or down opens a split beside besideSessionId"),
                opt("besideSessionId", Id, "The session a split opens beside; needs placement right or down"),
                opt("notifyOnComplete", Bool, "Report back when it stops (Pilots only)"),
            ],
            json!({"prompt":"Fix the failing login test and open a PR.","project":PROJECT,"notifyOnComplete":true}),
        ),
        action(
            "sessions.send",
            "Send a message to an idle session.",
            vec![session_id(), req("prompt", Text, "The message"), project(), opt("notifyOnComplete", Bool, "Report back when it stops (Pilots only)")],
            json!({"sessionId":"1234","prompt":"Also update the changelog.","project":PROJECT}),
        ),
        action(
            "sessions.draft",
            "Put an unsent message in a session's composer.",
            vec![session_id(), req("prompt", Text, "The draft"), project()],
            json!({"sessionId":"1234","prompt":"Ready to merge?","project":PROJECT}),
        ),
        action("sessions.stop", "Stop a session's current turn.", vec![session_id(), project()], json!({"sessionId":"1234","project":PROJECT})),
        action("sessions.archive", "Archive a session.", vec![session_id(), project()], json!({"sessionId":"1234","project":PROJECT})),
        action("sessions.delete", "Delete a session.", vec![session_id(), project()], json!({"sessionId":"1234","project":PROJECT})),
        action("worktrees.list", "List a project's worktrees.", vec![project()], json!({"project":PROJECT})),
        action(
            "worktrees.create",
            "Create a worktree on a new or existing branch.",
            vec![req("branch", Text, "Branch name"), opt("base", Text, "Branch a new branch starts from"), opt("existing", Bool, "Use an existing branch"), project()],
            json!({"branch":"fix/login-test","base":"main","project":PROJECT}),
        ),
        action("folders.list", "List a project's session folders.", vec![project()], json!({"project":PROJECT})),
        action(
            "folders.move",
            "Move a session into a folder, a new folder, or out of folders.",
            vec![session_id(), opt("folderId", Text, "A folder ID from folders.list"), opt("newFolderName", Text, "Create this folder"), project()],
            json!({"sessionId":"1234","newFolderName":"Release","project":PROJECT}),
        ),
        action("notes.list", "List the user's Notes.", vec![opt("limit", Count, ""), opt("offset", Count, "")], json!({"limit":20})),
        action("notes.read", "Read one of the user's Notes.", vec![req("id", Id, "A note ID from notes.list")], json!({"id":"n1"})),
        action(
            "notes.write",
            "Create a Note (body required), or change an existing note's title, body or tags.",
            vec![opt("id", Id, "An existing note's ID"), opt("title", Text, ""), opt("body", Text, "Markdown"), opt("tags", Tags, "Up to 20 short tags")],
            json!({"title":"Release plan","body":"- Cut the branch\n- Run QA"}),
        ),
        action("artifacts.list", "List this Pilot's saved reports.", vec![opt("kind", Text, "document or html"), opt("limit", Count, ""), opt("offset", Count, "")], json!({})),
        action("artifacts.read", "Read one of this Pilot's saved reports.", vec![req("id", Id, "An artifact ID from artifacts.list")], json!({"id":"a1"})),
        action(
            "artifacts.write",
            "Save a substantial report; it appears as a card in the chat.",
            vec![opt("id", Id, "Replace this artifact"), opt("kind", Choice(&["document", "html"]), "Defaults to document"), req("title", Text, ""), req("body", Text, "The complete report"), opt("summary", Text, "One or two sentences")],
            json!({"kind":"document","title":"Weekly review","summary":"Two regressions found.","body":"# Weekly review\n..."}),
        ),
        action("soul.read", "Read your soul (standing instructions) and its hash.", vec![], json!({})),
        action(
            "soul.update",
            "Replace your soul. Pass the hash soul.read returned, so a change made meanwhile isn't lost.",
            vec![req("text", Text, "The whole new soul"), req("expectedHash", Text, "hash from soul.read")],
            json!({"text":"# Soul\n\n## Standing instructions\n- Review PRs daily.\n","expectedHash":"<hash from soul.read>"}),
        ),
        action("memory.read", "Read your memory, or one topic note.", vec![opt("topic", Text, "A topic name")], json!({"topic":"decisions"})),
        action(
            "memory.search",
            "Search your memory.",
            vec![req("query", Text, ""), opt("since", Text, "YYYY-MM-DD, or 24h, 7d, 2w")],
            json!({"query":"release","since":"7d"}),
        ),
        action(
            "memory.add",
            "Remember a fact, in your memory or a topic note.",
            vec![req("fact", Text, "1–1000 characters"), opt("topic", Text, "A topic name"), opt("until", Text, "Forget after this date, YYYY-MM-DD")],
            json!({"fact":"The user ships releases on Fridays.","topic":"decisions"}),
        ),
        action(
            "memory.replace",
            "Replace the remembered fact that contains find.",
            vec![req("find", Text, "Text in the fact to replace"), req("fact", Text, "The new fact"), opt("topic", Text, ""), opt("until", Text, "YYYY-MM-DD")],
            json!({"find":"ships releases on Fridays","fact":"The user ships releases on Thursdays."}),
        ),
        action(
            "memory.remove",
            "Forget the remembered fact that contains find.",
            vec![req("find", Text, "Text in the fact to forget"), opt("topic", Text, "")],
            json!({"find":"ships releases on"}),
        ),
        action("habits.list", "List your habits (scheduled work).", vec![], json!({})),
        action(
            "habits.add",
            "Add a habit, only after the user agreed to it.",
            vec![req("name", Text, ""), req("instructions", Text, "What each run does, and whether it may change anything"), schedule(true)],
            json!({"name":"Morning PR sweep","instructions":"Read-only: list open PRs that need review. Return NOTHING_TO_REPORT if none.","schedule":{"kind":"weekdays","time":"09:00"}}),
        ),
        action(
            "habits.update",
            "Change a habit.",
            vec![req("id", Id, "A habit ID from habits.list"), opt("name", Text, ""), opt("instructions", Text, ""), schedule(false), opt("enabled", Bool, "")],
            json!({"id":"h1","enabled":false}),
        ),
        action("habits.run", "Run a habit now.", vec![req("id", Id, "A habit ID from habits.list")], json!({"id":"h1"})),
        action("habits.remove", "Remove a habit.", vec![req("id", Id, "A habit ID from habits.list")], json!({"id":"h1"})),
        action("flows.list", "List your SOPs (Flows), the ones on your SOP Roadmap tab.", vec![], json!({})),
        action("flows.read", "Read one SOP (Flow) in full, in the shape flows.save takes.", vec![flow_id()], json!({"flowId":"f1"})),
        action(
            "flows.save",
            "Create an SOP (Flow), or replace one: flows.read it, change it, and save the whole Flow with its id. With draft, an existing Flow keeps its live version and the change waits as a draft the user saves as the next version on the Pipeline tab.",
            vec![req("flow", Flow, "The whole Flow"), opt("draft", Bool, "Keep the change as a draft instead of making it live")],
            json!({"flow":crate::flow::pilot_schema::example(PROJECT)}),
        ),
        action("flows.delete", "Delete an SOP (Flow).", vec![flow_id()], json!({"flowId":"f1"})),
        action(
            "flows.start",
            "Start one run of an SOP (Flow) for one task.",
            vec![flow_id(), req("goal", Text, "The goal this task belongs to"), req("task", Text, "This run's task"), opt("inputs", TextMap, "The Flow's inputs by name")],
            json!({"flowId":"f1","goal":"Release 2.0","task":"Ship the login fix","inputs":{"version":"2.0.1"}}),
        ),
        action("flows.runs", "List your SOP runs, optionally for one goal.", vec![opt("goal", Text, "")], json!({"goal":"Release 2.0"})),
        action("flows.run", "Read one run, with its visits and outputs.", vec![run_id()], json!({"runId":"r1"})),
        action(
            "flows.editRun",
            "Change one run's remaining steps: pass the run's whole Flow with your changes. Steps that already ran can't change.",
            vec![run_id(), req("flow", Flow, "The run's whole Flow"), req("note", Text, "Why, for the run's history")],
            json!({"runId":"r1","flow":crate::flow::pilot_schema::example(PROJECT),"note":"Allow one more build attempt."}),
        ),
        action("flows.cancel", "Cancel a run.", vec![run_id()], json!({"runId":"r1"})),
        action(
            "flows.resume",
            "Resume a failed run from one of its states.",
            vec![run_id(), req("stateId", Text, "The state to run again")],
            json!({"runId":"r1","stateId":"build"}),
        ),
        action(
            "chat.card",
            "Show a card in the chat: a pull request, a session, up to four choices, or a suggested habit.",
            vec![
                req("type", Choice(&["pr", "session", "choices", "habit"]), "The card's kind"),
                opt("repo", Text, "pr: owner/name"),
                opt("number", Count, "pr: the PR number"),
                opt("note", Text, "pr or session"),
                opt("sessionId", Id, "session"),
                opt("options", Options, "choices: one to four answers"),
                opt("name", Text, "habit"),
                opt("instructions", Text, "habit"),
                schedule(false),
            ],
            json!({"type":"choices","options":["Ship now","Wait for QA"]}),
        ),
        action("ticket.get", "Read this session's linked ticket.", vec![opt("key", Text, "Another linked ticket's key")], json!({})),
        action("ticket.comment", "Comment on this session's linked ticket.", vec![req("body", Text, "Markdown"), opt("key", Text, "")], json!({"body":"Fixed in #42."})),
        action(
            "ticket.status",
            "Without target, list the statuses the linked ticket can move to; with target, move it.",
            vec![opt("target", Text, "A status id or name from the list"), opt("fields", Object, "Values a move requires, by field key"), opt("key", Text, "")],
            json!({"target":"In Review"}),
        ),
        action("card.subtasks", "List the subtasks of this session's board cards.", vec![], json!({})),
        action(
            "card.subtask_done",
            "Mark a board card subtask done or not done.",
            vec![req("subtaskId", Text, "From card.subtasks"), req("done", Bool, "")],
            json!({"subtaskId":"s1","done":true}),
        ),
    ]
});

pub fn all() -> &'static [Action] {
    &ACTIONS
}
pub fn find(name: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|action| action.name == name)
}

/// The action most like `name`, for a misspelled or guessed one.
pub fn suggest(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase().replace(['_', '/', ' '], ".");
    ACTIONS
        .iter()
        .map(|action| {
            (
                distance(&lower, &action.name.to_ascii_lowercase()),
                action.name,
            )
        })
        .filter(|(distance, _)| *distance <= 2)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, name)| name)
}
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous + usize::from(ca != *cb);
            previous = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(previous + 1);
        }
    }
    row[b.len()]
}

/// How a caller learns one action's input, for an error to end with.
pub fn hint(name: &str) -> String {
    format!("Call the app_help tool with {{\"action\":\"{name}\"}} for its input schema and an example that works.")
}

impl Ty {
    /// The error for a value of the wrong kind, or None when it fits.
    pub fn check(self, key: &str, value: &Value) -> Option<String> {
        let id = value.is_string() || value.as_u64().is_some();
        let fits = match self {
            Self::Text => value.is_string(),
            Self::Bool => value.is_boolean(),
            Self::Count => value.as_u64().is_some(),
            Self::Id | Self::Turn => id,
            Self::Choice(values) => value.as_str().is_some_and(|v| values.contains(&v)),
            Self::TextMap => value
                .as_object()
                .is_some_and(|map| map.values().all(Value::is_string)),
            Self::Tags => value.as_array().is_some_and(|tags| {
                tags.len() <= crate::notes::MAX_TAGS
                    && tags.iter().all(|tag| {
                        tag.as_str()
                            .is_some_and(|tag| tag.len() <= crate::notes::MAX_TAG_LENGTH)
                    })
            }),
            Self::Options => value.as_array().is_some_and(|options| {
                (1..=4).contains(&options.len()) && options.iter().all(Value::is_string)
            }),
            Self::Object | Self::Schedule | Self::Flow => value.is_object(),
        };
        (!fits).then(|| match self {
            Self::Text => format!("{key} must be text"),
            Self::Bool => format!("{key} must be true or false"),
            Self::Count => format!("{key} must be a non-negative integer"),
            Self::Id => format!("{key} must be an ID"),
            Self::Turn => format!("{key} must be a turn ID"),
            Self::Choice(values) => format!("{key} must be one of {}", values.join(", ")),
            Self::TextMap => format!("{key} must be an object of text values"),
            Self::Tags => format!(
                "{key} must be an array of up to {} short strings",
                crate::notes::MAX_TAGS
            ),
            Self::Options => format!("{key} must be a list of one to four strings"),
            Self::Object | Self::Schedule => format!("{key} must be an object"),
            Self::Flow => format!("{key} must be a Flow object"),
        })
    }

    fn schema(self) -> Value {
        match self {
            Self::Text => json!({"type":"string"}),
            Self::Bool => json!({"type":"boolean"}),
            Self::Count => json!({"type":"integer","minimum":0}),
            Self::Id | Self::Turn => json!({"type":["string","integer"]}),
            Self::Choice(values) => json!({"enum":values}),
            Self::TextMap => json!({"type":"object","additionalProperties":{"type":"string"}}),
            Self::Tags => {
                json!({"type":"array","maxItems":crate::notes::MAX_TAGS,"items":{"type":"string","maxLength":crate::notes::MAX_TAG_LENGTH}})
            }
            Self::Options => {
                json!({"type":"array","minItems":1,"maxItems":4,"items":{"type":"string"}})
            }
            Self::Object => json!({"type":"object"}),
            Self::Schedule => json!({
                "type":"object",
                "properties":{
                    "kind":{"enum":SCHEDULE_KINDS},
                    "time":{"type":"string","pattern":"^[0-2][0-9]:[0-5][0-9]$","default":"09:00","description":"24-hour HH:MM, for daily, weekdays and weekly"},
                    "minute":{"type":"integer","minimum":0,"maximum":59,"default":0,"description":"Minute past the hour, for hourly"},
                    "dayOfWeek":{"type":"integer","minimum":0,"maximum":6,"default":1,"description":"For weekly: 0 is Sunday"},
                },
                "required":["kind"],
            }),
            Self::Flow => crate::flow::pilot_schema::schema(),
        }
    }

    /// How the field reads in the help's one-line summary.
    fn signature(self, name: &str) -> String {
        match self {
            Self::Schedule => format!("{name}:{{kind,time?,minute?,dayOfWeek?}}"),
            _ => name.to_owned(),
        }
    }
}

impl Action {
    /// `input`'s JSON Schema.
    pub fn schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        for field in &self.fields {
            let mut schema = field.ty.schema();
            if !field.about.is_empty() && schema.get("description").is_none() {
                schema["description"] = json!(field.about);
            }
            properties.insert(field.name.into(), schema);
        }
        let required: Vec<&str> = self
            .fields
            .iter()
            .filter(|field| field.required)
            .map(|field| field.name)
            .collect();
        json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
    }
    /// The help's one line: `flows.start {flowId,goal,task,inputs?}`.
    pub fn signature(&self) -> String {
        let fields: Vec<String> = self
            .fields
            .iter()
            .map(|field| {
                let name = field.ty.signature(field.name);
                if field.required {
                    name
                } else {
                    format!("{name}?")
                }
            })
            .collect();
        format!("{} {{{}}}", self.name, fields.join(","))
    }
    /// What `app_help` returns for this action.
    pub fn describe(&self) -> Value {
        json!({
            "action":self.name,
            "description":self.about,
            "input":self.schema(),
            "example":{"action":self.name,"input":self.example},
        })
    }
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_example_passes_its_own_check() {
        for action in all() {
            assert_eq!(
                crate::pilot::api::validate(action.name, &action.example),
                Ok(()),
                "{}",
                action.name
            );
        }
    }

    #[test]
    fn the_flow_examples_save_as_written() {
        for name in ["flows.save", "flows.editRun"] {
            let example = &find(name).unwrap().example["flow"];
            assert!(
                crate::flow::pilot_schema::pilot_flow(example, "p1").is_ok(),
                "{name}"
            );
        }
    }

    #[test]
    fn names_are_unique_and_every_field_is_in_the_schema() {
        let mut names: Vec<&str> = all().iter().map(|a| a.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all().len());
        for action in all() {
            let schema = action.schema();
            for field in &action.fields {
                assert!(schema["properties"].get(field.name).is_some());
                assert_eq!(
                    schema["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(field.name)),
                    field.required,
                    "{} {}",
                    action.name,
                    field.name
                );
            }
        }
    }

    #[test]
    fn a_described_action_carries_its_schema_and_a_call_that_works() {
        let described = find("flows.save").unwrap().describe();
        assert_eq!(described["input"]["required"], json!(["flow"]));
        assert_eq!(
            described["input"]["properties"]["flow"]["required"],
            json!(["name", "cwd", "initial", "states"])
        );
        assert_eq!(described["example"]["action"], "flows.save");
        assert!(described["description"].as_str().unwrap().contains("SOP"));
        let memory = find("memory.add").unwrap().describe();
        assert_eq!(memory["input"]["required"], json!(["fact"]));
        assert_eq!(
            memory["input"]["properties"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            ["fact", "topic", "until"]
        );
    }

    #[test]
    fn a_guessed_action_is_matched_to_the_nearest_one() {
        for (guess, meant) in [
            ("memory_add", Some("memory.add")),
            ("flow.save", Some("flows.save")),
            ("sessions.starts", Some("sessions.start")),
            ("flows.edit_run", Some("flows.editRun")),
        ] {
            assert_eq!(suggest(guess), meant, "{guess}");
        }
        assert_eq!(suggest("deploy"), None);
    }

    #[test]
    fn signatures_mark_optional_fields_and_nested_schedules() {
        assert_eq!(
            find("flows.start").unwrap().signature(),
            "flows.start {flowId,goal,task,inputs?}"
        );
        assert_eq!(
            find("habits.add").unwrap().signature(),
            "habits.add {name,instructions,schedule:{kind,time?,minute?,dayOfWeek?}}"
        );
        assert_eq!(find("models.list").unwrap().signature(), "models.list {}");
    }
}
