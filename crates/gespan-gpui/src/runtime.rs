use gespan_core::agent::{Approval, Control, Settings};
use gespan_core::{
    provider::{self, Event, Kind, Request},
    store::{Session, Store},
    transcript::{Row, Transcript},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
#[cfg(all(test, unix))]
mod follow_up_tests;
mod handoff;
#[cfg(all(test, unix))]
mod handoff_tests;
mod persistence;
pub(crate) mod remote;
mod ticket_context;
pub(crate) mod ticket_scope;
#[cfg(all(test, unix))]
mod ticket_tests;
mod visual_discovery;
#[cfg(all(test, unix))]
mod visual_tests;
use persistence::PendingSave;
pub(crate) type Database = Arc<Mutex<Store>>;
struct TurnInput {
    summary: Session,
    prompt: String,
    settings: Settings,
}
pub(crate) struct Update {
    pub(crate) worktree_cwd: Option<String>,
    pub(crate) commands: Option<Vec<provider::NativeCommand>>,
    pub(crate) provider_turn_id: Option<String>,
    pub(crate) turn_ready: bool,
    pub(crate) follow_up_delivered: Vec<String>,
    pub(crate) follow_up_errors: Vec<(String, String)>,
    pub(crate) delivered: Option<String>,
    pub(crate) start: usize,
    pub(crate) rows: Vec<Row>,
    pub(crate) provider_id: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) config_model: Option<String>,
    pub(crate) config_settings: Option<std::collections::HashMap<String, String>>,
    pub(crate) context: Option<gespan_core::context::Usage>,
    pub(crate) metrics: Option<gespan_core::agent::TurnMetrics>,
    pub(crate) usage_limit: Option<gespan_core::usage::Limit>,
    /// The tasks a yielded turn still waits on, when that changed.
    pub(crate) background: Option<Vec<provider::BackgroundTask>>,
    /// A held turn reached its safe point.
    pub(crate) hold: Option<provider::HoldPoint>,
    pub(crate) approvals: Vec<Approval>,
    pub(crate) approvals_resolved: Vec<String>,
    pub(crate) questions: Vec<gespan_core::agent::Question>,
    pub(crate) questions_resolved: Vec<String>,
    pub(crate) handoff: Option<Option<gespan_core::agent::Handoff>>,
    pub(crate) status: String,
    /// On the final update: the provider finished the turn without an error.
    /// It stays true when saving the transcript, metadata or handoff fails
    /// afterwards: the work itself is done, and `status` reports the save.
    pub(crate) succeeded: bool,
    /// A ticket tool call changed the linked ticket.
    pub(crate) ticket: Option<TicketChange>,
    /// On the final update of a completed turn: its last assistant or plan
    /// block, which write-back posts to the session's cards.
    pub(crate) reply: Option<String>,
    pub(crate) finished: bool,
}
/// Stop and Hold both end a turn on purpose: no error row, status Ready.
pub(crate) fn ended_by_user<T>(cancel: &AtomicBool, result: &Result<T, String>) -> bool {
    cancel.load(Ordering::Relaxed)
        || result
            .as_ref()
            .is_err_and(|error| error == "Stopped" || error == "Held")
}
fn publish(
    tx: &async_channel::Sender<Update>,
    cancel: &AtomicBool,
    mut update: Update,
) -> Result<(), String> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Stopped".into());
        }
        match tx.try_send(update) {
            Ok(()) => return Ok(()),
            Err(async_channel::TrySendError::Closed(_)) => return Err("Window closed".into()),
            Err(async_channel::TrySendError::Full(value)) => update = value,
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}
pub(crate) fn locked(store: &Database) -> Result<std::sync::MutexGuard<'_, Store>, String> {
    store
        .lock()
        .map_err(|_| "Native session store unavailable".into())
}

pub(crate) fn run_turn(
    summary: Session,
    prompt: String,
    settings: Settings,
    controls: std::sync::mpsc::Receiver<Control>,
    store: Database,
    cancel: Arc<AtomicBool>,
    tx: async_channel::Sender<Update>,
) {
    if summary.cwd.starts_with("remote://") {
        remote::run(summary, prompt, settings, controls, store, cancel, tx);
        return;
    }
    let _session = enter_turn(&summary, false);
    run_turn_using(
        TurnInput {
            summary,
            prompt,
            settings,
        },
        controls,
        store,
        cancel,
        tx,
        provider::resolve,
    );
}

/// Enters the turn's `agent_session` span and logs its start. Everything the
/// turn's thread logs while the span is held, here and in gespan-core, names
/// its session. Local and remote turns both start here.
pub(crate) fn enter_turn(summary: &Session, remote: bool) -> tracing::span::EnteredSpan {
    let span = tracing::info_span!(
        "agent_session",
        session_id = summary.id,
        provider = %summary.provider,
        remote,
    )
    .entered();
    tracing::info!("agent turn started");
    span
}

fn run_turn_using(
    input: TurnInput,
    controls: std::sync::mpsc::Receiver<Control>,
    store: Database,
    cancel: Arc<AtomicBool>,
    tx: async_channel::Sender<Update>,
    resolve_binary: impl FnOnce(Kind) -> Option<std::path::PathBuf>,
) {
    let TurnInput {
        summary,
        prompt,
        mut settings,
    } = input;
    if settings.account.root.is_none() && settings.account.id != gespan_core::accounts::DEFAULT {
        settings.account.root = locked(&store)
            .ok()
            .and_then(|s| s.data_dir().map(std::path::Path::to_path_buf));
    }
    let source_account = settings.account.id.clone();
    let user_request = prompt.clone();
    let mut transcript = Transcript::default();
    let mut history_loaded = false;
    let shared = locked(&store)
        .map(|s| s.uses_gespan_schema())
        .unwrap_or(false);
    let mut pending = PendingSave::new(shared, std::time::Instant::now());
    pending.own_context(&summary.provider, &source_account);
    let mut initial_delivery = None;
    let mut accepted_messages = Vec::new();
    let mut provider_id = summary.provider_id.clone();
    let mut turn_start = 0;
    let mut turn_receipt: Option<(usize, gespan_core::agent::TurnInfo)> = None;
    let mut pilot_fingerprint = None;
    let mut visual_introduction = None;
    let mut ticket_introduction = None;
    let mut ticket_tools = None;
    let ticket_changes: TicketChanges = Arc::default();
    let mut prepared_handoff = None;
    let mut handoff_accepted = false;
    // SessionReview's recorded turn: its project and id, once the baseline is taken.
    let mut checkpoint: Option<(String, String)> = None;
    let mut created_worktree = None;
    let mut context = locked(&store)
        .ok()
        .and_then(|s| s.context_usage(summary.id).ok().flatten())
        .unwrap_or_default();
    let mut usage_limit = None;
    let result = (|| {
        if let Some(reason) = locked(&store)?.edit_recovery_reason(summary.id)? {
            return Err(reason);
        }
        if settings.worktree_removed {
            return Err("Select a branch or worktree to continue this conversation.".into());
        }
        if settings.account.id != gespan_core::accounts::DEFAULT
            && !locked(&store)?
                .provider_accounts()?
                .exists(Kind::from_id(&summary.provider), &settings.account.id)
        {
            return Err(
                "This provider account was removed. Choose another account to continue.".into(),
            );
        }
        transcript = locked(&store)?.load(summary.id)?;
        history_loaded = true;
        turn_start = transcript.rows.len();
        settings.operator_enabled |= settings.operator;
        let mut persisted = settings.clone();
        persisted.handoff = None;
        // These modes belong to this turn, not the next composer draft.
        persisted.plan = false;
        persisted.operator = false;
        persisted.orchestrator = false;
        let start = transcript.begin(if settings.internal_message {
            "pilot-system"
        } else {
            "user"
        });
        if let Some(at) = settings
            .turn_model
            .as_ref()
            .and_then(|turn| turn.started_at)
        {
            transcript.rows[start].sent_at = Some(at);
        }
        if settings.pilot.is_some()
            && let Some(at) = settings.pilot_sent_at
        {
            transcript.rows[start].sent_at = Some(at);
        }
        let started_at = transcript.rows[start].sent_at;
        transcript.append(&prompt);
        transcript.attach_files(&settings.attachments);
        // Show the working receipt while connecting, before the first token.
        transcript.begin("assistant");
        // Keep the already painted prompt in the failure replay even if saving
        // the turn's configuration fails before its first transcript write.
        locked(&store)?.configure_agent(summary.id, &summary.provider, &persisted)?;
        locked(&store)?.write_rows_with_message(
            summary.id,
            start,
            &transcript.rows[start..],
            settings.pilot_message_id.as_deref(),
        )?;
        let mut info =
            settings
                .turn_model
                .clone()
                .unwrap_or_else(|| gespan_core::agent::TurnInfo {
                    provider: summary.provider.clone(),
                    model_id: settings.model.clone(),
                    ..Default::default()
                });
        info.started_at = info.started_at.or(started_at);
        // The receipt's key follows all continuation rows of the prompt.
        let turn = transcript
            .rows
            .iter()
            .rposition(Row::is_prompt)
            .map_or(0, |at| at + 1);
        persistence::save_turn_receipt(&store, summary.id, turn, info.clone())?;
        turn_receipt = Some((turn, info));
        initial_delivery = settings.pilot_message_id.clone();
        let creating_worktree = settings.new_worktree && settings.worktree_cwd.is_none();
        publish(
            &tx,
            &cancel,
            Update {
                worktree_cwd: None,
                commands: None,
                provider_turn_id: None,
                turn_ready: false,
                follow_up_delivered: Vec::new(),
                follow_up_errors: Vec::new(),
                delivered: settings.pilot_message_id.clone(),
                start,
                rows: transcript.rows[start..].to_vec(),
                provider_id: None,
                model: None,
                config_model: None,
                config_settings: None,
                context: None,
                metrics: None,
                usage_limit: None,
                background: None,
                hold: None,
                approvals: Vec::new(),
                approvals_resolved: Vec::new(),
                questions: Vec::new(),
                questions_resolved: Vec::new(),
                handoff: None,
                status: if creating_worktree {
                    "Creating worktree…"
                } else if settings.pending_provider_switch.is_some() {
                    "Preparing handoff…"
                } else {
                    "Connecting…"
                }
                .into(),
                succeeded: false,
                ticket: None,
                reply: None,
                finished: false,
            },
        )?;
        if creating_worktree {
            let path = gespan_core::git::create_worktree(
                std::path::Path::new(&summary.cwd),
                settings.worktree_base.as_deref().unwrap_or("HEAD"),
                summary.id,
            )?;
            let cwd = path.to_string_lossy().into_owned();
            created_worktree = Some(cwd.clone());
            settings.worktree_cwd = Some(cwd.clone());
            settings.new_worktree = false;
            {
                let store = locked(&store)?;
                let mut persisted = store.agent_settings(summary.id)?.unwrap_or_default();
                persisted.worktree_cwd = Some(cwd.clone());
                persisted.new_worktree = false;
                store.save_agent_settings(summary.id, &persisted)?;
            }
            publish(
                &tx,
                &cancel,
                Update {
                    worktree_cwd: Some(cwd),
                    commands: None,
                    provider_turn_id: None,
                    turn_ready: false,
                    follow_up_delivered: Vec::new(),
                    follow_up_errors: Vec::new(),
                    delivered: None,
                    start: transcript.rows.len(),
                    rows: Vec::new(),
                    provider_id: None,
                    model: None,
                    config_model: None,
                    config_settings: None,
                    context: None,
                    metrics: None,
                    usage_limit: None,
                    background: None,
                    hold: None,
                    approvals: Vec::new(),
                    approvals_resolved: Vec::new(),
                    questions: Vec::new(),
                    questions_resolved: Vec::new(),
                    handoff: None,
                    status: "Connecting…".into(),
                    succeeded: false,
                    ticket: None,
                    reply: None,
                    finished: false,
                },
            )?;
        }
        if let Some(source) = settings.pending_provider_switch.take() {
            // The turn's first step: the outgoing model's recap can take a while.
            let mut transfer =
                gespan_core::tool::ToolCall::new("provider-handoff", gespan_core::tool::HANDOFF);
            let from = if source.model_label.is_empty() {
                Kind::from_id(&source.from).label()
            } else {
                source.model_label.as_str()
            };
            transfer.input = serde_json::json!({ "from": from }).to_string();
            let transfer_row = transcript.begin("tool");
            transcript.rows[transfer_row].text =
                serde_json::to_string(&transfer).map_err(|e| e.to_string())?;
            pending.changed(transfer_row);
            publish(
                &tx,
                &cancel,
                Update {
                    worktree_cwd: None,
                    commands: None,
                    provider_turn_id: None,
                    turn_ready: false,
                    follow_up_delivered: Vec::new(),
                    follow_up_errors: Vec::new(),
                    delivered: None,
                    start: transfer_row,
                    rows: transcript.rows[transfer_row..].to_vec(),
                    provider_id: None,
                    model: None,
                    config_model: None,
                    config_settings: None,
                    context: None,
                    metrics: None,
                    usage_limit: None,
                    background: None,
                    approvals: Vec::new(),
                    approvals_resolved: Vec::new(),
                    questions: Vec::new(),
                    questions_resolved: Vec::new(),
                    hold: None,
                    handoff: None,
                    status: "Preparing handoff…".into(),
                    succeeded: false,
                    ticket: None,
                    reply: None,
                    finished: false,
                },
            )?;
            let root = locked(&store)?.data_dir().map(std::path::Path::to_path_buf);
            let prepared = handoff::prepare(
                summary.id,
                &source,
                settings.worktree_cwd.as_deref().unwrap_or(&summary.cwd),
                &user_request,
                &transcript.rows[..turn_start],
                root,
                cancel.clone(),
            );
            transfer.state = if cancel.load(Ordering::Relaxed) {
                gespan_core::tool::ToolState::Cancelled
            } else {
                gespan_core::tool::ToolState::Completed
            };
            transcript.rows[transfer_row].text =
                serde_json::to_string(&transfer).map_err(|e| e.to_string())?;
            pending.changed(transfer_row);
            settings.handoff = Some(prepared.clone());
            prepared_handoff = Some(prepared.clone());
            provider_id = None;
            {
                let store = locked(&store)?;
                store.reset_provider_id(summary.id)?;
                let mut persisted = store.agent_settings(summary.id)?.unwrap_or_default();
                persisted.pending_provider_switch = None;
                persisted.handoff = Some(prepared.clone());
                store.save_agent_settings(summary.id, &persisted)?;
            }
            publish(
                &tx,
                &cancel,
                Update {
                    worktree_cwd: None,
                    commands: None,
                    provider_turn_id: None,
                    turn_ready: false,
                    follow_up_delivered: Vec::new(),
                    follow_up_errors: Vec::new(),
                    delivered: None,
                    start: transfer_row,
                    rows: transcript.rows[transfer_row..].to_vec(),
                    provider_id: None,
                    model: None,
                    config_model: None,
                    config_settings: None,
                    context: None,
                    metrics: None,
                    usage_limit: None,
                    background: None,
                    hold: None,
                    approvals: Vec::new(),
                    approvals_resolved: Vec::new(),
                    questions: Vec::new(),
                    questions_resolved: Vec::new(),
                    handoff: Some(Some(prepared)),
                    status: "Connecting…".into(),
                    succeeded: false,
                    ticket: None,
                    reply: None,
                    finished: false,
                },
            )?;
        }
        let kind = Kind::parse(&summary.provider)
            .ok_or_else(|| format!("Unknown provider: {}", summary.provider))?;
        let binary = resolve_binary(kind).ok_or_else(|| {
            kind.missing_binary_or(format!(
                "{} Set GESPAN_{}_BINARY to choose a different path.",
                kind.unavailable_hint(),
                kind.id().to_uppercase()
            ))
        })?;
        // These providers parse slash arguments themselves. App context before
        // the command would turn it into an ordinary message instead.
        let prompt = settings.prepared_prompt.take().unwrap_or(prompt);
        let raw_native_command = matches!(kind, Kind::Omp | Kind::Devin)
            && provider::native_prompt(kind, &prompt)
                .trim_start()
                .starts_with('/');
        let prompt = if raw_native_command {
            prompt
        } else if let Some(pilot) = &settings.pilot {
            let files = gespan_core::pilot::load_files(&pilot.root, &pilot.pilot)?;
            let added = gespan_core::pilot::context(&pilot.pilot, &files, &pilot.executable);
            let fingerprint = gespan_core::pilot::files::content_hash(&added);
            pilot_fingerprint = Some(fingerprint.clone());
            let rotated = provider_id.is_some() && context.ratio().is_some_and(|r| r >= 0.8);
            if rotated {
                provider_id = None;
                locked(&store)?.reset_provider_id(summary.id)?;
            }
            let receipt = locked(&store)?.setting(&format!("pilot-context:{}", summary.id))?;
            let expected =
                serde_json::json!({"provider":provider_id,"hash":fingerprint}).to_string();
            let intro = if provider_id.is_none() || receipt.as_deref() != Some(&expected) {
                added
            } else {
                String::new()
            };
            let brief = if provider_id.is_none() && turn_start > 0 {
                gespan_core::pilot::rotation_brief(&transcript.rows[..turn_start])
            } else {
                String::new()
            };
            let prompt = if let Some(handoff) = &settings.handoff {
                gespan_core::response::wrap_handoff_prompt(&handoff.from, &handoff.brief, &prompt)
            } else {
                prompt
            };
            format!("{intro}\n\n{brief}\n\n{prompt}")
        } else if let Some(handoff) = &settings.handoff {
            gespan_core::response::wrap_handoff_prompt(&handoff.from, &handoff.brief, &prompt)
        } else if provider_id.is_none() && turn_start > 0 {
            let mut history = String::new();
            for row in transcript.rows[..turn_start]
                .iter()
                .rev()
                .filter(|r| matches!(r.role.as_str(), "user" | "assistant"))
            {
                if history.len() + row.text.len() > 32_000 {
                    break;
                }
                history = format!("{}: {}\n\n{history}", row.role, row.text);
            }
            format!(
                "Continue this conversation, using the prior messages as context.\n\n{history}\nCurrent user message:\n{prompt}"
            )
        } else {
            prompt
        };
        let cwd = settings.worktree_cwd.as_deref().unwrap_or(&summary.cwd);
        if settings.pilot.is_none()
            && cwd != "~"
            && let Some(store) = gespan_core::checkpoint::store()
        {
            let id = gespan_core::checkpoint::new_turn_id();
            let session = summary.id.to_string();
            // Capture errors are persisted and shown in the review.
            let _ = store.exclusive(|store| {
                // An interrupted turn continues with this session's edit evidence.
                store.capture(&session, cwd, &edited_paths(&transcript.rows[..turn_start]))?;
                store.begin_turn(&session, cwd, &id)
            });
            checkpoint = Some((cwd.to_owned(), id));
        }
        let mut prompt = provider::prepare_prompt(kind, &prompt, cwd, &settings.attachments)?;
        if !raw_native_command && prompt.contains("@note:") {
            let notes = locked(&store)?.notes()?;
            for note in notes {
                let token = format!("@note:{}", note.id);
                if prompt
                    .lines()
                    .filter(|line| !line.trim_start().starts_with('>'))
                    .flat_map(str::split_whitespace)
                    .any(|word| word.trim_end_matches([',', '.', ';']) == token)
                {
                    prompt = format!(
                        "Referenced note: {}\n<note>\n{}\n</note>\n\n{prompt}",
                        note.title,
                        gespan_core::response::limit(&note.body, 64_000)
                    );
                }
            }
        }
        if !raw_native_command && settings.orchestrator {
            prompt = format!(
                "{}\n\nUser request:\n{prompt}",
                gespan_core::orchestration::INSTRUCTIONS
            );
        }
        if !raw_native_command && settings.operator_enabled {
            let cli = std::env::current_exe().map_err(|e| e.to_string())?;
            prompt = format!(
                "You have Gespan app access for this project. Use the CLI: {} app ACTION --json JSON. Supported actions: models.list {{}}, sessions.list {{}}, sessions.read {{sessionId}}, sessions.start {{prompt,harness?,model?,title?,draft?,reveal?}}, sessions.send/draft {{sessionId,prompt}}, sessions.stop {{sessionId}}, folders.list {{}}, folders.move {{sessionId,folderId?,newFolderName?}}, notes.list {{}}, notes.read {{id}}, notes.write {{id?,title,body}}, worktrees.list {{}}. Scope is this thread's project. App credentials are in the environment; never print them. Respect the user’s request and selected access policy.\n\n{prompt}",
                shlex_quote(&cli.to_string_lossy())
            );
        }
        // Every foreground session receives an independent visual capability.
        // This happens after configuration was persisted, and Settings' env is
        // transport-only, so credentials never enter saved conversation settings.
        let visual_bridge = if !settings.text_only && !raw_native_command {
            Some(gespan_core::visual_bridge::Server::start(summary.id)?)
        } else {
            None
        };
        if let Some(visual_bridge) = &visual_bridge {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            settings
                .environment
                .extend(visual_bridge.environment(&executable));
            visual_introduction = visual_discovery::Discovery::prepare(
                &*locked(&store)?,
                summary.id,
                kind,
                &settings.account.id,
                provider_id.as_deref(),
                &executable,
            )?;
            if let Some(discovery) = &visual_introduction {
                prompt = discovery.prepend(prompt);
            }
        }
        // A linked ticket gets its own capability on the same terms, but only
        // where the provider mounts MCP servers: there is no CLI fallback.
        let ticket = summary
            .work_item
            .as_ref()
            .and_then(gespan_core::session_groups::WorkItem::ticket);
        let data_dir = locked(&store)?.data_dir().map(std::path::Path::to_path_buf);
        // Ticket tools are optional: a refused start (logged by the bridge) or
        // an unknown executable skips them and the turn goes on.
        let executable = std::env::current_exe()
            .inspect_err(|error| tracing::warn!(%error, "ticket tools need the app executable"))
            .ok();
        // Leads and workers stay out of the ticket and card tools, as in the
        // Tauri app: the same refusal is applied again to every answered call.
        let restricted = settings.orchestrator || settings.orchestration_worker;
        let ticket = ticket.filter(|_| !restricted);
        let tool_servers = !settings.text_only
            && !raw_native_command
            && !visual_discovery::lacks_tool_servers(kind, provider_id.as_deref());
        let read_only =
            settings.plan || settings.access == gespan_core::agent::AccessMode::ReadOnly;
        // The board's half of the scope: the status permission (session
        // override, else the strictest linked board) and the write-back cards.
        let source = tool_servers.then(|| {
            Arc::new(ticket_scope::ScopeSource::new(
                store.clone(),
                summary.id,
                read_only,
                restricted,
            ))
        });
        let (may_change_status, cards) = source
            .as_ref()
            .map_or((false, false), |source| source.board_scope());
        if let (Some(source), Some(data_dir), Some(executable)) = (source, data_dir, executable)
            && (ticket.is_some() || cards)
        {
            let scope = gespan_core::ticket_bridge::Scope {
                ticket: ticket.cloned(),
                may_change_status,
                read_only,
                cards,
            };
            if let Ok(server) = gespan_core::ticket_bridge::Server::start(summary.id, scope.clone())
            {
                settings.environment.extend(server.environment(&executable));
                if let Some(ticket) = ticket {
                    ticket_introduction = ticket_context::TicketContext::prepare(
                        &*locked(&store)?,
                        summary.id,
                        ticket,
                        may_change_status,
                        read_only,
                        provider_id.as_deref(),
                    )?;
                }
                if let Some(introduction) = &ticket_introduction {
                    prompt = introduction.prepend(prompt);
                }
                ticket_tools = Some(TicketTools::new(
                    summary.id,
                    server,
                    Arc::new(gespan_core::tickets::Tickets::new(data_dir)),
                    ticket_changes.clone(),
                    source,
                    scope,
                ));
            }
        }
        if kind == Kind::Claude {
            // Seeds the provider's agent registry so a resumed sub-agent's
            // work finds the Agent call that started it in an earlier turn.
            settings.agent_runs =
                gespan_core::tool::tasks::agent_runs(&transcript.rows[..turn_start]);
            // A resumed conversation's TaskUpdate calls refer to earlier tasks.
            settings.claude_task_lists =
                gespan_core::tool::tasks::claude_task_lists(&transcript.rows[..turn_start]);
        }
        let request = Request {
            kind,
            binary,
            cwd: settings
                .worktree_cwd
                .as_deref()
                .unwrap_or(&summary.cwd)
                .into(),
            prompt,
            provider_id: provider_id.clone(),
            model: settings.model.clone(),
            settings,
        };
        let mut tool_rows = std::collections::HashMap::new();
        // The running-work list attributes the provider and its tools to this session.
        let result = gespan_core::process::owned_by(summary.id, || {
            provider::run_with_controls(request, cancel.clone(), Some(controls), |events| {
                // Cancellation freezes the transcript, even while the native
                // provider drains its interrupt acknowledgement.
                if cancel.load(Ordering::Relaxed) {
                    return Err("Stopped".into());
                }
                let publications = visual_bridge
                    .as_ref()
                    .map(|bridge| bridge.drain())
                    .unwrap_or_default();
                if let Some(tools) = &ticket_tools {
                    tools.refresh_scope();
                    tools.answer_pending();
                }
                let ticket_change = take_ticket_change(&ticket_changes);
                if events.is_empty() && publications.is_empty() && ticket_change.is_none() {
                    return Ok(());
                }
                let mut start = transcript.rows.len();
                let mut status = "Streaming…".to_string();
                let mut changed_id = None;
                let mut model = None;
                let mut config_model = None;
                let mut config_settings = None;
                let mut context_changed = false;
                let mut metrics = None;
                let mut background = None;
                let mut hold = None;
                let mut approvals = Vec::new();
                let mut approvals_resolved = Vec::new();
                let mut questions = Vec::new();
                let mut questions_resolved = Vec::new();
                let mut handoff = None;
                let mut turn_ready = false;
                let mut follow_up_delivered = Vec::new();
                let mut follow_up_errors = Vec::new();
                let mut boundary = false;
                let mut commands = None;
                let mut provider_turn_id = None;
                for event in events {
                    match event {
                        Event::Commands(discovered) => commands = Some(discovered),
                        Event::TurnId(id) => {
                            if let Some((_, info)) = &mut turn_receipt {
                                info.provider_turn_id = Some(id.clone());
                            }
                            provider_turn_id = Some(id);
                        }
                        Event::Ready => {
                            turn_ready = true;
                            boundary = true;
                            if prepared_handoff.is_some() && !handoff_accepted {
                                handoff_accepted = true;
                                handoff = Some(None);
                                let store = locked(&store)?;
                                let mut persisted =
                                    store.agent_settings(summary.id)?.unwrap_or_default();
                                persisted.handoff = None;
                                store.save_agent_settings(summary.id, &persisted)?;
                            }
                        }
                        Event::FollowUp(message) => {
                            let at = transcript.begin("user");
                            transcript.rows[at].sent_at =
                                message.sent_at.or(transcript.rows[at].sent_at);
                            start = start.min(at);
                            transcript.append(&message.text);
                            transcript.attach_files(&message.attachments);
                            // Paint the working receipt with the accepted steer,
                            // even when the provider has no new output yet.
                            transcript.begin("assistant");
                            let turn = transcript
                                .rows
                                .iter()
                                .rposition(Row::is_prompt)
                                .map_or(0, |at| at + 1);
                            pending.accept(
                                &store,
                                summary.id,
                                at,
                                &transcript,
                                &message.id,
                                std::time::Instant::now(),
                            )?;
                            if let Some((root_turn, root)) = &turn_receipt {
                                // A steer keeps its send time and model identity,
                                // while timing/metrics belong to the original run.
                                let steer = gespan_core::agent::TurnInfo {
                                    joined_turn: Some(*root_turn),
                                    provider: root.provider.clone(),
                                    model: root.model.clone(),
                                    model_id: root.model_id.clone(),
                                    provider_turn_id: root.provider_turn_id.clone(),
                                    ..Default::default()
                                };
                                persistence::save_turn_receipt(&store, summary.id, turn, steer)?;
                            }
                            accepted_messages.push(message.id.clone());
                            follow_up_delivered.push(message.id);
                        }
                        Event::FollowUpFailed { id, error } => follow_up_errors.push((id, error)),
                        Event::BeginAssistant => {
                            if transcript
                                .rows
                                .last()
                                .is_none_or(|r| r.role != "assistant" || !r.text.is_empty())
                            {
                                let at = transcript.begin("assistant");
                                start = start.min(at);
                                pending.changed(at);
                            }
                        }
                        Event::Text(text) => {
                            if transcript.rows.last().is_none_or(|r| r.role != "assistant") {
                                start = start.min(transcript.begin("assistant"));
                            }
                            let at = transcript.append(&text);
                            start = start.min(at);
                            pending.changed(at);
                        }
                        Event::Reasoning(text) => {
                            if text.is_empty() {
                                continue;
                            }
                            if transcript.rows.last().is_none_or(|r| r.role != "reasoning") {
                                start = start.min(transcript.begin("reasoning"));
                            }
                            let at = transcript.append(&text);
                            start = start.min(at);
                            pending.changed(at);
                        }
                        Event::Tool(tool) => {
                            if tool.is_compaction() {
                                status = if tool.state == gespan_core::tool::ToolState::Running {
                                    "Compacting context…"
                                } else {
                                    "Working…"
                                }
                                .into();
                            }
                            // Saved rows leave live elapsed out; the view reads it here.
                            gespan_core::tool::live::record(&tool);
                            let ix = *tool_rows
                                .entry(tool.id.clone())
                                .or_insert_with(|| transcript.begin("tool"));
                            transcript.rows[ix].text =
                                serde_json::to_string(&tool).map_err(|e| e.to_string())?;
                            start = start.min(ix);
                            pending.changed(ix);
                        }
                        Event::SessionId(id) => {
                            if provider_id.as_ref() != Some(&id) {
                                let store = locked(&store)?;
                                let mut settings =
                                    store.agent_settings(summary.id)?.unwrap_or_default();
                                if let Some(pending) = settings.pending_provider_switch.as_mut()
                                    && pending.from == summary.provider
                                    && pending.account.id == source_account
                                {
                                    pending.provider_id = Some(id.clone());
                                    store.save_agent_settings(summary.id, &settings)?;
                                } else if settings.account.id == source_account
                                    && store.sessions()?.iter().any(|session| {
                                        session.id == summary.id
                                            && session.provider == summary.provider
                                    })
                                {
                                    store.provider_id(summary.id, &id)?;
                                }
                                provider_id = Some(id.clone());
                                changed_id = Some(id);
                            }
                        }
                        Event::Model(id) => model = Some(id),
                        Event::ConfigChanged {
                            model: changed_model,
                            model_settings,
                        } => {
                            let store = locked(&store)?;
                            let mut settings =
                                store.agent_settings(summary.id)?.unwrap_or_default();
                            if let Some(pending) = settings.pending_provider_switch.as_mut()
                                && pending.from == summary.provider
                                && pending.account.id == source_account
                            {
                                if let Some(id) = &changed_model {
                                    pending.model = Some(id.clone());
                                }
                                pending.model_settings.extend(model_settings.clone());
                            } else if settings.account.id == source_account
                                && store.sessions()?.iter().any(|session| {
                                    session.id == summary.id && session.provider == summary.provider
                                })
                            {
                                if let Some(id) = &changed_model {
                                    settings.model = Some(id.clone());
                                }
                                settings.model_settings.extend(model_settings.clone());
                            }
                            store.save_agent_settings(summary.id, &settings)?;
                            if changed_model.is_some() {
                                model = changed_model.clone();
                                config_model = changed_model;
                            }
                            config_settings
                                .get_or_insert_with(std::collections::HashMap::new)
                                .extend(model_settings);
                        }
                        Event::Interjection {
                            text,
                            custom_type,
                            severity,
                        } => {
                            let note = gespan_core::transcript::Interjection {
                                text,
                                custom_type,
                                severity,
                            };
                            let at = transcript.begin("interjection");
                            transcript
                                .append(&serde_json::to_string(&note).map_err(|e| e.to_string())?);
                            start = start.min(at);
                            pending.changed(at);
                            boundary = true;
                        }
                        Event::Metrics(reading) => {
                            if let Some((_, info)) = &mut turn_receipt {
                                info.metrics = Some(reading);
                            }
                            metrics = Some(reading);
                        }
                        Event::Context { used, window } => {
                            context.merge(used, window);
                            context_changed = true;
                            pending.context_changed();
                        }
                        Event::Approval(approval) => {
                            status = "Waiting for approval".into();
                            approvals.push(approval);
                            boundary = true;
                        }
                        Event::ApprovalResolved(id) => {
                            approvals_resolved.push(id);
                            boundary = true;
                        }
                        Event::Question(question) => {
                            status = "Waiting for your answer".into();
                            questions.push(question);
                            boundary = true;
                        }
                        Event::QuestionResolved(id) => {
                            questions_resolved.push(id);
                            boundary = true;
                        }
                        Event::Status(text) => status = text,
                        Event::Background(tasks) => background = Some(tasks),
                        Event::HoldSettled(point) => {
                            hold = Some(point);
                            boundary = true;
                        }
                        Event::UsageLimited(limit) => usage_limit = Some(limit),
                        Event::Done => {
                            status = "Ready".into();
                            boundary = true;
                        }
                    }
                }
                for publication in &publications {
                    let document =
                        serde_json::to_string(&publication.document).map_err(|e| e.to_string())?;
                    let at = transcript.begin("visual");
                    transcript.append(&document);
                    start = start.min(at);
                    pending.changed(at);
                    boundary = true;
                }
                let now = std::time::Instant::now();
                let publication_result = (|| {
                    if pending.due(boundary, now) {
                        pending.flush(&store, summary.id, &transcript, &context, now)?;
                    }
                    publish(
                        &tx,
                        &cancel,
                        Update {
                            worktree_cwd: None,
                            commands,
                            provider_turn_id,
                            turn_ready,
                            follow_up_delivered,
                            follow_up_errors,
                            delivered: None,
                            start,
                            rows: transcript.rows[start..].to_vec(),
                            provider_id: changed_id,
                            model,
                            config_model,
                            config_settings,
                            context: context_changed.then_some(context),
                            metrics,
                            usage_limit: None,
                            background,
                            hold,
                            approvals,
                            approvals_resolved,
                            questions,
                            questions_resolved,
                            handoff,
                            status,
                            succeeded: false,
                            ticket: ticket_change,
                            reply: None,
                            finished: false,
                        },
                    )
                })();
                // A tool succeeds only after both durable storage and the live
                // transcript update have accepted the visual.
                for publication in publications {
                    let result = publication_result.as_ref().map(|_| {
                        serde_json::json!({"id":publication.document.id,"title":publication.document.title,"saved":true})
                    }).map_err(Clone::clone);
                    let _ = publication.reply.send(result);
                }
                publication_result
            })
        });
        if result.is_ok()
            && let Some(fingerprint) = &pilot_fingerprint
        {
            locked(&store)?.save_setting(
                &format!("pilot-context:{}", summary.id),
                &serde_json::json!({"provider":provider_id,"hash":fingerprint}).to_string(),
            )?;
        }
        if result.is_ok()
            && !cancel.load(Ordering::Relaxed)
            && let Some(discovery) = &visual_introduction
        {
            discovery.remember(&*locked(&store)?, provider_id.as_deref())?;
        }
        if result.is_ok()
            && !cancel.load(Ordering::Relaxed)
            && let Some(introduction) = &ticket_introduction
        {
            introduction.remember(&*locked(&store)?, provider_id.as_deref())?;
        }
        result
    })();
    let cancelled = ended_by_user(&cancel, &result);
    let handoff = prepared_handoff.map(|mut ready| {
        if handoff_accepted {
            None
        } else {
            // This accepted user message never reached the incoming provider.
            // Carry it with the ready packet when the user continues later.
            if !user_request.trim().is_empty() {
                ready
                    .brief
                    .push_str("\n\nAfter the switch, before this message, the user also sent:\n\n");
                ready.brief.push_str(&user_request);
            }
            Some(ready)
        }
    });
    let handoff_save = if let Some(ready) = &handoff {
        locked(&store).and_then(|store| {
            let mut persisted = store.agent_settings(summary.id)?.unwrap_or_default();
            persisted.pending_provider_switch = None;
            persisted.handoff = ready.clone();
            store.save_agent_settings(summary.id, &persisted)
        })
    } else {
        Ok(())
    };
    // A storage error or cancelled publish can leave this turn's latest rows
    // unseen by the UI. Replay the turn on failure before sending an error or
    // settled tools; starting after those missing rows would abort in patch().
    let mut final_start = if result.is_err() {
        turn_start
    } else {
        transcript.rows.len()
    };
    if let Some(ix) = settle_running_tools(&mut transcript.rows, turn_start, cancelled, &result) {
        final_start = final_start.min(ix);
    }
    if let Some((cwd, id)) = &checkpoint
        && let Some(store) = gespan_core::checkpoint::store()
    {
        // The provider has exited, so this snapshot is the turn's settled result.
        let session = summary.id.to_string();
        let _ = store.exclusive(|store| {
            store.capture(&session, cwd, &edited_paths(&transcript.rows[turn_start..]))?;
            store.finish_turn(&session, cwd, id)
        });
    }
    if !cancelled && let Err(error) = &result {
        log_turn_failure(error);
    }
    let succeeded = history_loaded && result.is_ok();
    // Write-back takes only a completed turn's reply: never a failed or
    // stopped one's.
    let reply = (succeeded && !cancelled)
        .then(|| {
            ticket_scope::final_reply(&transcript.rows[turn_start.min(transcript.rows.len())..])
        })
        .flatten();
    let (start, rows, mut status) = if !history_loaded {
        // A rejected preflight has no row baseline. Writing an error at row 0
        // would replace a real conversation's first saved message.
        (
            0,
            Vec::new(),
            if cancelled {
                "Ready".into()
            } else {
                result
                    .err()
                    .unwrap_or_else(|| "Conversation was not loaded".into())
            },
        )
    } else if let Err(error) = result
        && !cancelled
    {
        let start = final_start.min(transcript.begin("error"));
        transcript.append(&error);
        pending.changed(start);
        let error = match pending.flush(
            &store,
            summary.id,
            &transcript,
            &context,
            std::time::Instant::now(),
        ) {
            Ok(()) => error,
            Err(storage) => format!("{error} · Saving failed: {storage}"),
        };
        (start, transcript.rows[start..].to_vec(), error)
    } else {
        if final_start < transcript.rows.len() {
            pending.changed(final_start);
        }
        let status = pending
            .flush(
                &store,
                summary.id,
                &transcript,
                &context,
                std::time::Instant::now(),
            )
            .map(|_| "Ready".into())
            .unwrap_or_else(|error| format!("Saving failed: {error}"));
        (final_start, transcript.rows[final_start..].to_vec(), status)
    };
    // Settle provenance in storage before notifying the view, including when
    // its window has closed or the provider was cancelled.
    if let Some((turn, mut info)) = turn_receipt {
        let completed = gespan_core::time::now_ms();
        info.duration_ms = info.started_at.map_or(0, |at| completed.saturating_sub(at));
        info.duration_known = true;
        info.completed_at = info.started_at.map(|_| completed);
        if let Err(error) = persistence::save_turn_receipt(&store, summary.id, turn, info) {
            status = format!("{status} · Saving turn metadata failed: {error}");
        }
    }
    if let Err(error) = handoff_save {
        status = format!("{status} · Saving handoff failed: {error}");
    }
    // Final status must still arrive after cancellation. Dropping a window closes
    // the receiver, so the worker never keeps a closed viewport alive.
    let _ = tx.send_blocking(Update {
        // A stop can make the UI skip intermediate updates. Keep the created
        // checkout in the final receipt so the next Send cannot create it twice.
        worktree_cwd: created_worktree,
        commands: None,
        provider_turn_id: None,
        turn_ready: false,
        follow_up_delivered: accepted_messages,
        follow_up_errors: Vec::new(),
        delivered: initial_delivery,
        start,
        rows,
        provider_id,
        model: None,
        config_model: None,
        config_settings: None,
        context: None,
        metrics: None,
        usage_limit: if cancelled { None } else { usage_limit },
        background: None,
        hold: None,
        approvals: Vec::new(),
        approvals_resolved: Vec::new(),
        questions: Vec::new(),
        questions_resolved: Vec::new(),
        handoff,
        status,
        succeeded,
        ticket: take_ticket_change(&ticket_changes),
        reply,
        finished: true,
    });
}

/// The most of a turn error the log keeps.
const LOGGED_ERROR_CHARS: usize = 200;

/// How gespan-core's provider worker starts the error for a CLI that exits
/// mid-turn; up to 8 KB of the CLI's stderr follows after a space. The log
/// keeps only this part; the stderr stays in the transcript.
const PROVIDER_EXITED: &str = "Provider exited before completing the turn.";

/// What the view does with the linked ticket after the agent's tool calls.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TicketChange {
    /// Fetch again: something changed that the answer does not describe.
    Refresh,
    /// The tracker's snapshot after a status move; no fetch needed.
    Moved(gespan_core::tickets::TicketRef),
}

type TicketChanges = Arc<Mutex<Option<TicketChange>>>;

fn take_ticket_change(changes: &TicketChanges) -> Option<TicketChange> {
    changes.lock().ok()?.take()
}

/// Reads, listings and failures change nothing. A move's snapshot beats a
/// pending refresh, since it is the newer state.
fn record_ticket_change(changes: &TicketChanges, effect: gespan_core::ticket_bridge::Effect) {
    use gespan_core::ticket_bridge::Effect;
    let Ok(mut slot) = changes.lock() else {
        return;
    };
    match effect {
        Effect::None => {}
        Effect::Commented => {
            slot.get_or_insert(TicketChange::Refresh);
        }
        Effect::Moved(ticket) => *slot = Some(TicketChange::Moved(ticket)),
    }
}

/// Settles one answered ticket call. The change is recorded before the
/// reply goes out: the reply can let the provider end its turn at once, and
/// the turn's final update carries only what is recorded by then, so a move
/// recorded after the reply could leave the session's ticket on its old
/// status.
fn settle_ticket_call(
    changes: &TicketChanges,
    answer: Result<gespan_core::ticket_bridge::Answered, String>,
    reply: impl FnOnce(Result<serde_json::Value, String>),
) {
    match answer {
        Ok(answered) => {
            record_ticket_change(changes, answered.effect);
            reply(Ok(answered.value));
        }
        Err(error) => {
            tracing::warn!(%error, "ticket tool call failed");
            reply(Err(error));
        }
    }
}

/// How often a turn re-reads its scope when only another connection's write
/// (not a reported change) suggests it may have moved.
const SCOPE_POLL: Duration = Duration::from_secs(1);

/// A turn's ticket and card capability and the services that answer its calls.
struct TicketTools {
    server: gespan_core::ticket_bridge::Server,
    tickets: Arc<gespan_core::tickets::Tickets>,
    changes: TicketChanges,
    source: Arc<ticket_scope::ScopeSource>,
    watch: ticket_scope::Watch,
    /// The scope the bridge holds now.
    scope: std::cell::RefCell<gespan_core::ticket_bridge::Scope>,
    version: std::cell::Cell<Option<i64>>,
    polled: std::cell::Cell<std::time::Instant>,
}

impl TicketTools {
    fn new(
        session: i64,
        server: gespan_core::ticket_bridge::Server,
        tickets: Arc<gespan_core::tickets::Tickets>,
        changes: TicketChanges,
        source: Arc<ticket_scope::ScopeSource>,
        scope: gespan_core::ticket_bridge::Scope,
    ) -> Self {
        let watch = ticket_scope::Watch::new(session);
        let version = source.data_version();
        Self {
            server,
            tickets,
            changes,
            source,
            watch,
            scope: std::cell::RefCell::new(scope),
            version: std::cell::Cell::new(version),
            polled: std::cell::Cell::new(std::time::Instant::now()),
        }
    }

    /// Gives the bridge the current scope when the app reported a change to
    /// the session's ticket link, status override or card links, or another
    /// connection wrote to the database (at most once a `SCOPE_POLL`).
    fn refresh_scope(&self) {
        let reported = self.watch.take();
        let version = self.source.data_version();
        let moved = version != self.version.get() && self.polled.get().elapsed() >= SCOPE_POLL;
        if reported || moved {
            self.version.set(version);
            self.polled.set(std::time::Instant::now());
            self.apply(self.source.read());
        }
    }

    fn apply(&self, scope: gespan_core::ticket_bridge::Scope) {
        if *self.scope.borrow() != scope {
            self.server.set_scope(scope.clone());
            *self.scope.borrow_mut() = scope;
        }
    }

    /// Answers the calls the bridge accepted, against the scope read now:
    /// the link a call was accepted under must still be the session's link,
    /// and the permission and cards are this moment's. Tracker and board
    /// calls run off the provider loop.
    fn answer_pending(&self) {
        use gespan_core::ticket_bridge::BridgeTool;
        let calls = self.server.drain();
        if calls.is_empty() {
            return;
        }
        let current = self.source.read();
        self.apply(current.clone());
        for call in calls {
            match &call.tool {
                BridgeTool::Ticket(tool) => {
                    let ticket = match (&current.ticket, &call.ticket) {
                        (None, _) | (_, None) => {
                            Err(gespan_tracker_client::tools::NO_TICKET.to_string())
                        }
                        (Some(now), Some(accepted))
                            if !ticket_scope::same_ticket(now, accepted) =>
                        {
                            Err(format!("This session is linked to {}", now.key))
                        }
                        (Some(now), Some(_)) => gespan_tracker_client::tools::refusal(
                            tool,
                            current.may_change_status,
                            current.read_only,
                        )
                        .map_or(Ok(now.clone()), |refused| Err(refused.to_string())),
                    };
                    let ticket = match ticket {
                        Ok(ticket) => ticket,
                        Err(refused) => {
                            let _ = call.reply.send(Err(refused));
                            continue;
                        }
                    };
                    let tool = tool.clone();
                    let tickets = self.tickets.clone();
                    let changes = self.changes.clone();
                    std::thread::spawn(move || {
                        let answer = gespan_core::ticket_bridge::answer(&tickets, &ticket, &tool);
                        settle_ticket_call(&changes, answer, |reply| {
                            let _ = call.reply.send(reply);
                        });
                    });
                }
                BridgeTool::Card(tool) => {
                    if !current.cards {
                        let _ = call.reply.send(Err(ticket_scope::NO_CARDS.into()));
                        continue;
                    }
                    let tool = tool.clone();
                    let source = self.source.clone();
                    std::thread::spawn(move || {
                        let result = source.answer_card(&tool);
                        if let Err(error) = &result {
                            tracing::warn!(%error, "card tool call failed");
                        }
                        let _ = call.reply.send(result);
                    });
                }
            }
        }
    }
}

/// Logs a failed turn with the first line of its error, capped, and its
/// full length. A provider exit logs none of the provider's stderr.
pub(crate) fn log_turn_failure(error: &str) {
    let first = if error.starts_with(PROVIDER_EXITED) {
        PROVIDER_EXITED
    } else {
        error.lines().next().unwrap_or_default()
    };
    let mut logged: String = first.chars().take(LOGGED_ERROR_CHARS).collect();
    if logged.len() < first.len() {
        logged.push('…');
    }
    tracing::warn!(error = %logged, error_len = error.len(), "agent turn failed");
}

/// Flip tools still Running at turn end to Cancelled or Failed, and forget their
/// live elapsed. Returns the first row changed.
fn settle_running_tools<T>(
    rows: &mut [Row],
    turn_start: usize,
    cancelled: bool,
    result: &Result<T, String>,
) -> Option<usize> {
    let mut first = None;
    for (ix, row) in rows.iter_mut().enumerate().skip(turn_start) {
        if row.role != "tool" {
            continue;
        }
        if let Ok(mut tool) = serde_json::from_str::<gespan_core::tool::ToolCall>(&row.text)
            && tool.state == gespan_core::tool::ToolState::Running
        {
            tool.state = if cancelled {
                gespan_core::tool::ToolState::Cancelled
            } else {
                gespan_core::tool::ToolState::Failed
            };
            if tool.output.is_empty() {
                tool.output = if cancelled {
                    "Cancelled".into()
                } else {
                    result.as_ref().err().cloned().unwrap_or_else(|| {
                        "No completion result was received before the turn ended.".into()
                    })
                };
            }
            gespan_core::tool::live::remove(&tool.id);
            row.text = serde_json::to_string(&tool).unwrap();
            first = Some(first.map_or(ix, |f: usize| f.min(ix)));
        }
    }
    first
}

/// checkpointEdits.ts's sessionEditPaths: files named by successful edit tools.
pub(crate) fn edited_paths(rows: &[Row]) -> Vec<String> {
    let mut paths = Vec::new();
    for row in rows.iter().filter(|r| r.role == "tool") {
        let Ok(tool) = serde_json::from_str::<gespan_core::tool::ToolCall>(&row.text) else {
            continue;
        };
        if tool.state != gespan_core::tool::ToolState::Completed {
            continue;
        }
        let shown = tool.presentation();
        if matches!(
            shown.action.as_str(),
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit"
        ) {
            let input = serde_json::from_str::<serde_json::Value>(&tool.input).unwrap_or_default();
            let many = input["paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|path| path.as_str());
            let desktop = tool.display.as_ref().unwrap_or(&input["__desktop"]);
            let desktop_paths = desktop["paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|path| path.as_str());
            for path in shown
                .file_path
                .as_deref()
                .into_iter()
                .chain(many)
                .chain(desktop_paths)
            {
                if !path.is_empty() && !paths.iter().any(|p| p == path) {
                    paths.push(path.to_owned());
                }
            }
        }
    }
    paths
}

fn shlex_quote(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('\"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`")
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_comments_and_moves_change_the_ticket_and_a_move_wins() {
        use gespan_core::ticket_bridge::Effect;
        let moved = gespan_core::tickets::TicketRef {
            tracker: gespan_core::tickets::Tracker::Jira,
            key: "ENG-1".into(),
            url: None,
            title: None,
            status: Some("Done".into()),
            status_group: None,
            fetched_at: None,
        };
        let changes = TicketChanges::default();
        record_ticket_change(&changes, Effect::None);
        assert_eq!(take_ticket_change(&changes), None);
        record_ticket_change(&changes, Effect::Commented);
        assert_eq!(take_ticket_change(&changes), Some(TicketChange::Refresh));
        assert_eq!(take_ticket_change(&changes), None, "taken once");
        record_ticket_change(&changes, Effect::Commented);
        record_ticket_change(&changes, Effect::Moved(moved.clone()));
        record_ticket_change(&changes, Effect::Commented);
        assert_eq!(
            take_ticket_change(&changes),
            Some(TicketChange::Moved(moved))
        );
    }
    use super::*;
    use gespan_core::tool::{ToolCall, ToolState, live};

    #[test]
    fn turn_failures_log_only_the_first_line_of_the_error() {
        let error = "Provider exited before completing the turn.\nAuthorization: Bearer sk-secret";
        let lines = crate::log_capture::capture(|| log_turn_failure(error));
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["level"], "warn");
        assert_eq!(lines[0]["message"], "agent turn failed");
        assert_eq!(
            lines[0]["fields"]["error"],
            "Provider exited before completing the turn."
        );
        assert_eq!(lines[0]["fields"]["error_len"], error.len());
        assert!(!lines[0].to_string().contains("sk-secret"));
    }

    #[test]
    fn provider_exit_failures_log_no_provider_stderr() {
        // gespan-core joins the CLI's stderr to the message with a space.
        let stderr = "Error: 401 Authorization: Bearer sk-secret";
        let error = format!("{PROVIDER_EXITED} {stderr}");
        let lines = crate::log_capture::capture(|| log_turn_failure(&error));
        assert_eq!(lines[0]["fields"]["error"], PROVIDER_EXITED);
        assert_eq!(lines[0]["fields"]["error_len"], error.len());
        assert!(!lines[0].to_string().contains("sk-secret"), "{lines:?}");
    }

    #[test]
    fn provider_exit_message_matches_the_one_gespan_core_builds() {
        let provider = include_str!("../../gespan-core/src/provider.rs");
        assert!(
            provider.contains(&format!("\"{PROVIDER_EXITED} {{}}\"")),
            "gespan-core changed its provider exit message; update PROVIDER_EXITED"
        );
    }

    #[test]
    fn long_turn_failure_lines_are_capped_on_a_character_boundary() {
        let error = "é".repeat(500);
        let lines = crate::log_capture::capture(|| log_turn_failure(&error));
        let logged = lines[0]["fields"]["error"].as_str().unwrap();
        assert_eq!(logged, format!("{}…", "é".repeat(200)));
        assert_eq!(lines[0]["fields"]["error_len"], error.len());
    }

    fn tool_row(id: &str, state: ToolState) -> Row {
        let mut call = ToolCall::new(id, "Bash");
        call.state = state;
        Row {
            role: "tool".into(),
            text: serde_json::to_string(&call).unwrap(),
            first: true,
            sent_at: None,
        }
    }

    #[test]
    fn turn_end_settling_forgets_live_elapsed_of_running_tools_only() {
        for (cancelled, expected) in [(true, ToolState::Cancelled), (false, ToolState::Failed)] {
            live::set_elapsed("toolu_rt_running", 90);
            live::set_elapsed("toolu_rt_before_turn", 40);
            let mut rows = vec![
                tool_row("toolu_rt_before_turn", ToolState::Running),
                tool_row("toolu_rt_running", ToolState::Running),
                tool_row("toolu_rt_done", ToolState::Completed),
            ];
            let first = settle_running_tools::<()>(&mut rows, 1, cancelled, &Ok(()));
            assert_eq!(first, Some(1));
            let settled: ToolCall = serde_json::from_str(&rows[1].text).unwrap();
            assert_eq!(settled.state, expected);
            assert_eq!(live::elapsed("toolu_rt_running"), None);
            // A row from an earlier turn is not this turn's to settle.
            assert_eq!(live::elapsed("toolu_rt_before_turn"), Some(40));
            live::remove("toolu_rt_before_turn");
        }
    }

    #[test]
    fn turn_end_settling_with_nothing_running_changes_nothing() {
        let mut rows = vec![tool_row("toolu_rt_ok", ToolState::Completed)];
        assert_eq!(
            settle_running_tools::<()>(&mut rows, 0, false, &Ok(())),
            None
        );
    }
}

#[cfg(test)]
mod edited_paths_tests {
    use super::*;
    use gespan_core::tool::{ToolCall, ToolState};
    use serde_json::json;

    #[test]
    fn successful_edits_keep_real_paths_across_the_whole_session() {
        let row = |name: &str, state, input: serde_json::Value| {
            let mut tool = ToolCall::new("tool", name);
            tool.state = state;
            tool.input = input.to_string();
            Row {
                role: "tool".into(),
                text: serde_json::to_string(&tool).unwrap(),
                first: true,
                sent_at: None,
            }
        };
        let rows = vec![
            row(
                "Edit",
                ToolState::Completed,
                json!({"file_path":"/repo/a.rs", "cwd":"/repo"}),
            ),
            row(
                "Read",
                ToolState::Completed,
                json!({"file_path":"/repo/read.rs"}),
            ),
            row(
                "Write",
                ToolState::Failed,
                json!({"file_path":"/repo/failed.rs"}),
            ),
            row(
                "Edit",
                ToolState::Running,
                json!({"file_path":"/repo/running.rs"}),
            ),
            Row {
                role: "user".into(),
                text: "A plain question".into(),
                first: true,
                sent_at: None,
            },
            row(
                "fileChange",
                ToolState::Completed,
                json!({"path":"/repo/a.rs", "paths":["/repo/a.rs", "relative.rs"]}),
            ),
        ];
        assert_eq!(edited_paths(&rows), ["/repo/a.rs", "relative.rs"]);
    }
}
