//! The app's side of the decision log: what the user wrote in a session, so a
//! proposal can be checked against the user's own words.
use crate::runtime::{Database, locked};
use gespan_core::decisions::Embedder;
use gespan_core::outside_turns::without_injected;
use gespan_core::pilot::files::content_hash as digest;
use gespan_core::store::Store;
use gespan_core::transcript::Row;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

/// How many of a session's answers are kept, newest last.
const ANSWERS_KEPT: usize = 50;
/// How many of a session's typed messages are remembered, newest last.
const TYPED_KEPT: usize = 500;

/// The user's answers to agents' questions, per session. They reach the
/// provider, not the transcript, so they are kept here for this run of the app.
static ANSWERS: LazyLock<Mutex<HashMap<i64, VecDeque<String>>>> = LazyLock::new(Default::default);

/// Where a session's typed messages are recorded: the SHA-256 of each `user`
/// message the human typed or sent, never the text. A session without it was
/// opened by an agent, a Pilot, a habit or a Flow (or before Gespan recorded
/// this), and nothing in it counts as the user's.
fn typed_key(session: i64) -> String {
    format!("decision-typed:{session}")
}

/// Whether this launch's message is the human's own words: typed or sent by
/// them, and not a Pilot's, an orchestration worker's or an app message.
/// Turns the app or an agent starts (habits, Flows, resumes, `sessions.*`
/// sends, reports) are launched with `user_typed` off.
pub(crate) fn typed_by_user(settings: &gespan_core::agent::Settings) -> bool {
    settings.user_typed
        && !settings.internal_message
        && settings.pilot.is_none()
        && !settings.orchestration_worker
}

/// Records `text`, a `user` message exactly as saved, as one the human typed
/// in `session`. `opens_session` says it is the session's first message: only
/// that starts the record, so a session an agent opened never gets one.
pub(crate) fn record_typed(
    store: &Store,
    session: i64,
    text: &str,
    opens_session: bool,
) -> Result<(), String> {
    let key = typed_key(session);
    let mut typed: Vec<String> = match store.setting(&key)? {
        Some(saved) => serde_json::from_str(&saved).map_err(|e| e.to_string())?,
        None if opens_session => Vec::new(),
        None => return Ok(()),
    };
    let hash = digest(text);
    typed.retain(|kept| *kept != hash);
    typed.push(hash);
    let excess = typed.len().saturating_sub(TYPED_KEPT);
    typed.drain(..excess);
    store.save_setting(
        &key,
        &serde_json::to_string(&typed).map_err(|e| e.to_string())?,
    )
}

/// The hashes of `session`'s typed messages; `None` without a usable record.
fn typed_record(store: &Store, session: i64) -> Option<HashSet<String>> {
    let saved = store.setting(&typed_key(session)).ok()??;
    serde_json::from_str::<Vec<String>>(&saved)
        .map(|typed| typed.into_iter().collect())
        .ok()
}

/// Remembers the user's answers to an agent's question in `session`: each
/// chosen option or typed answer on its own.
pub(crate) fn record_answers(session: i64, answers: &[Vec<String>]) {
    let mut kept = ANSWERS.lock().unwrap_or_else(|e| e.into_inner());
    let session = kept.entry(session).or_default();
    for answer in answers.iter().flatten() {
        let answer = answer.trim();
        if answer.is_empty() {
            continue;
        }
        session.push_back(answer.to_owned());
        if session.len() > ANSWERS_KEPT {
            session.pop_front();
        }
    }
}

fn answers(session: i64) -> Vec<String> {
    let kept = ANSWERS.lock().unwrap_or_else(|e| e.into_inner());
    kept.get(&session)
        .map(|answers| answers.iter().cloned().collect())
        .unwrap_or_default()
}

/// What the user wrote in `session`: the messages they typed, as saved in its
/// transcript, then their answers to its agent's questions. A session with no
/// record of typed messages (opened by an agent, a Pilot, a habit or a Flow)
/// gives nothing, and so does a remote one: its transcript lives elsewhere.
/// The app reads it in two halves ([`session_rows`] under the store's lock,
/// [`texts_from`] after); tests read it whole.
#[cfg(test)]
fn user_texts(store: &Store, session: i64) -> Vec<String> {
    texts_from(session_rows(store, session), session)
}

/// What [`user_texts`] needs from the app database: the session's rows and
/// the hashes of its typed messages; `None` for a session with no record or
/// a remote one. The only part that needs the store, so the caller can
/// release its lock before the rows are rejoined, hashed and filtered.
fn session_rows(store: &Store, session: i64) -> Option<(Vec<Row>, HashSet<String>)> {
    if matches!(store.remote_session(session), Ok(Some(_))) {
        return None;
    }
    let typed_hashes = typed_record(store, session)?;
    match store.load(session) {
        Ok(transcript) => Some((transcript.rows, typed_hashes)),
        Err(error) => {
            tracing::warn!(session, %error, "the decision log could not read a transcript");
            Some((Vec::new(), typed_hashes))
        }
    }
}

/// The user's typed messages among `rows`, then their answers; nothing for
/// a session [`session_rows`] gave nothing for.
fn texts_from(rows: Option<(Vec<Row>, HashSet<String>)>, session: i64) -> Vec<String> {
    let Some((rows, typed_hashes)) = rows else {
        return Vec::new();
    };
    let mut texts = typed(&rows, &typed_hashes);
    texts.extend(answers(session));
    texts
}

/// The human's own messages among `rows`: `user` messages, rejoined from
/// their stored fragments, whose text is in `typed_hashes`, without the
/// context Gespan put in front of them. Everything else (agent text, tools,
/// attachments, app messages, and prompts an agent or the app sent as `user`)
/// is left out.
fn typed(rows: &[Row], typed_hashes: &HashSet<String>) -> Vec<String> {
    let mut messages: Vec<String> = Vec::new();
    let mut in_user = false;
    for row in rows {
        if row.first {
            in_user = row.role == "user";
            if in_user {
                messages.push(String::new());
            }
        }
        if in_user
            && row.role == "user"
            && let Some(message) = messages.last_mut()
        {
            message.push_str(&row.text);
        }
    }
    messages
        .iter()
        .filter(|message| typed_hashes.contains(&digest(message)))
        .map(|message| without_injected(message).to_owned())
        .filter(|message| !message.is_empty())
        .collect()
}

/// Registers [`user_texts`] as the decision log's source of the user's words,
/// and the harness writer as its regeneration hook.
pub(crate) fn install(store: Database) {
    install_harness_writer(store.clone());
    install_user_text_source(store);
}

/// The hub calls the source on its own thread, so it reads the app database
/// itself: the rows under the store's lock, the rest after releasing it.
fn install_user_text_source(store: Database) {
    gespan_core::decisions::service::set_user_text_source(Box::new(move |session| {
        let rows = match locked(&store) {
            Ok(store) => session_rows(&store, session),
            Err(_) => None,
        };
        texts_from(rows, session)
    }));
}

/// The open projects, as the shell last saved them, so the harness writer
/// (which runs off the UI thread) knows which checkouts to write for.
static KNOWN_PROJECTS: LazyLock<Mutex<Vec<String>>> = LazyLock::new(Default::default);

/// Records the open projects; true when the list changed.
pub(crate) fn set_known_projects(projects: &[String]) -> bool {
    let mut known = KNOWN_PROJECTS.lock().unwrap_or_else(|e| e.into_inner());
    if *known == projects {
        return false;
    }
    *known = projects.to_vec();
    true
}

/// Every local checkout Gespan knows: the open projects and their
/// worktrees. Remote (SSH) projects get nothing in v1. Runs git, so call it
/// off the UI thread.
fn known_workspaces() -> Vec<PathBuf> {
    let projects = KNOWN_PROJECTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let mut out: Vec<PathBuf> = Vec::new();
    let sessions = SESSION_CHECKOUTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    for project in projects {
        let path = PathBuf::from(&project);
        if gespan_core::files::remote::is_remote(&path) || !path.is_dir() {
            continue;
        }
        let trees = gespan_core::git::worktrees(&path).unwrap_or_default();
        let found = trees
            .into_iter()
            .filter(|tree| !tree.missing)
            .map(|tree| PathBuf::from(tree.path));
        for candidate in std::iter::once(path.clone()).chain(found) {
            if !out.contains(&candidate) {
                out.push(candidate);
            }
        }
    }
    for checkout in sessions {
        if checkout.is_dir() && !out.contains(&checkout) {
            out.push(checkout);
        }
    }
    out
}

/// Checkouts sessions started in that are not an open project (a worktree
/// made after the last regeneration, say), so they get their files too.
static SESSION_CHECKOUTS: LazyLock<Mutex<Vec<PathBuf>>> = LazyLock::new(Default::default);

/// Called as a session starts in `workspace`. The first session in a
/// checkout Gespan has not seen schedules a regeneration (it runs off the UI
/// thread); until it lands, `start_context` injects the block there. True
/// when it scheduled one.
pub(crate) fn note_session_workspace(workspace: &Path) -> bool {
    if gespan_core::files::remote::is_remote(workspace) {
        return false;
    }
    let known = KNOWN_PROJECTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|p| Path::new(p) == workspace);
    if known {
        return false;
    }
    {
        let mut seen = SESSION_CHECKOUTS.lock().unwrap_or_else(|e| e.into_inner());
        if seen.iter().any(|p| p == workspace) {
            return false;
        }
        seen.push(workspace.to_path_buf());
    }
    gespan_core::decisions::service::regenerate();
    true
}

/// What the harness writer should do when the log's switch is applied.
#[derive(Debug, PartialEq, Eq)]
enum SwitchAction {
    /// Turned on by the user: move old decisions to their project's scope,
    /// offer their existing harness preferences for review, then write the
    /// files.
    ImportThenRegenerate,
    /// On at startup: move old decisions to their project's scope, then
    /// write the files.
    Regenerate,
    RemoveAll,
    /// Off at startup: remove what an earlier run wrote, if it recorded
    /// anything (it was turned off while Gespan could not clean up).
    RemoveLeftovers,
}

/// The first apply at startup regenerates when the log is on and removes
/// leftovers when it is off; turning it on imports the user's existing
/// preferences once, then regenerates; turning it off removes what Gespan
/// wrote. An unchanged switch does nothing.
fn switch_action(previous: Option<bool>, on: bool) -> Option<SwitchAction> {
    match (previous, on) {
        (Some(was), now) if was == now => None,
        (Some(false), true) => Some(SwitchAction::ImportThenRegenerate),
        (_, true) => Some(SwitchAction::Regenerate),
        (Some(true), false) => Some(SwitchAction::RemoveAll),
        (None, false) => Some(SwitchAction::RemoveLeftovers),
        (Some(false), false) => None,
    }
}

/// The work behind each [`SwitchAction`]. The real steps touch the log, git
/// and the harness homes; tests record the calls.
struct SwitchSteps<'a> {
    rescope: &'a dyn Fn(),
    import: &'a dyn Fn(),
    regenerate: &'a dyn Fn(),
    remove: &'a dyn Fn(),
    has_leftovers: &'a dyn Fn() -> bool,
}

/// Runs `action`'s steps in order. Old decisions are rescoped only while
/// the log is on, and before the import and the files read them.
fn run_switch(action: &SwitchAction, steps: &SwitchSteps) {
    match action {
        SwitchAction::Regenerate => {
            (steps.rescope)();
            (steps.regenerate)();
        }
        SwitchAction::ImportThenRegenerate => {
            (steps.rescope)();
            (steps.import)();
            (steps.regenerate)();
        }
        SwitchAction::RemoveAll => (steps.remove)(),
        SwitchAction::RemoveLeftovers => {
            if (steps.has_leftovers)() {
                (steps.remove)();
            }
        }
    }
}

static SWITCH: Mutex<Option<bool>> = Mutex::new(None);
static HARNESS_STORE: Mutex<Option<Database>> = Mutex::new(None);

fn harness_store() -> Option<Database> {
    HARNESS_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Decisions saved before the log, still keyed by their project's path,
/// move to the scope that project has now (a repo, a main checkout). Runs
/// git, so never on the UI thread.
fn rescope_old_decisions() {
    let moved = gespan_core::decisions::service::with(|log| {
        log.rescope_project_rows(gespan_core::decisions::scope_for)
    });
    if let Ok(Err(error)) = moved {
        tracing::warn!(%error, "the decision log could not rescope old decisions");
    }
}

/// Whether an earlier run recorded writing harness files.
fn recorded_leftovers() -> bool {
    let data_dir = harness_store().and_then(|store| {
        locked(&store)
            .ok()
            .and_then(|s| s.data_dir().map(Path::to_path_buf))
    });
    data_dir.is_some_and(|dir| gespan_core::decisions::harness::has_recorded_files(&dir))
}

/// Applies the log's switch to the harness files: called with every
/// preferences apply, including the first one at startup. The work runs on
/// its own thread, never the UI's, and not under `locked`.
pub(crate) fn decision_log_switched(on: bool) {
    let previous = SWITCH.lock().unwrap_or_else(|e| e.into_inner()).replace(on);
    let Some(action) = switch_action(previous, on) else {
        return;
    };
    std::thread::spawn(move || {
        let remove = || {
            if let Some(store) = harness_store() {
                write_harness_files(&store, false);
            }
        };
        run_switch(
            &action,
            &SwitchSteps {
                rescope: &rescope_old_decisions,
                import: &|| {
                    import_global_preferences(&gespan_core::decisions::harness::default_homes)
                },
                regenerate: &gespan_core::decisions::service::regenerate,
                remove: &remove,
                has_leftovers: &recorded_leftovers,
            },
        );
    });
}

/// Offers the user's existing global harness preferences as pending entries
/// (once: rules already imported are skipped) and tells My tasks to look
/// again. Does nothing while the log is off or not open.
fn import_global_preferences(homes: gespan_core::decisions::harness::Homes) {
    // One import at a time: a quick on, off, on must not run two together.
    static IMPORTING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = IMPORTING.lock().unwrap_or_else(|e| e.into_inner());
    if !gespan_core::decisions::enabled() {
        return;
    }
    let now = gespan_core::time::now_ms() as i64;
    match gespan_core::decisions::service::with(|log| import_from_homes(log, homes, now)) {
        Ok(Ok(0)) | Err(_) => {}
        Ok(Ok(_)) => gespan_core::decisions::service::changed(),
        Ok(Err(error)) => tracing::warn!(%error, "existing preferences could not be imported"),
    }
}

/// Imports the rule files found under `homes` into `log`; returns how many
/// entries were added.
fn import_from_homes(
    log: &gespan_core::decisions::DecisionStore,
    homes: gespan_core::decisions::harness::Homes,
    now_ms: i64,
) -> Result<usize, String> {
    let files = gespan_core::decisions::import::global_files(homes);
    gespan_core::decisions::import::import_once(log, &files, now_ms)
}

/// The paths whose "edit replaced" notice is waiting for a window to show.
static EDIT_NOTICES: LazyLock<(
    async_channel::Sender<String>,
    async_channel::Receiver<String>,
)> = LazyLock::new(async_channel::unbounded);

/// Notices for user edits Gespan replaced, as they come in.
pub(crate) fn edit_notices() -> async_channel::Receiver<String> {
    EDIT_NOTICES.1.clone()
}

fn edit_notice_key(path: &Path) -> String {
    format!("decision-edit-notice:{}", path.display())
}

pub(crate) fn edit_notice_text(path: &Path) -> String {
    format!(
        "Gespan replaced your edit inside its section of {}. Edit decisions in My tasks instead.",
        path.display()
    )
}

/// The notices to show for `paths` (files whose Gespan section the user had
/// edited): one per path, ever. Marks each as shown.
fn unseen_edit_notices(store: &Store, paths: &[PathBuf]) -> Vec<String> {
    let mut out = Vec::new();
    for path in paths {
        let key = edit_notice_key(path);
        if matches!(store.setting(&key), Ok(Some(_))) {
            continue;
        }
        if let Err(error) = store.save_setting(&key, "shown") {
            tracing::warn!(%error, "the decision edit notice could not be recorded");
            continue;
        }
        out.push(edit_notice_text(path));
    }
    out
}

/// Registers the harness writer as the log's regeneration hook. Each run
/// happens on its own thread, never the UI's; the core serializes them.
fn install_harness_writer(store: Database) {
    *HARNESS_STORE.lock().unwrap_or_else(|e| e.into_inner()) = Some(store.clone());
    gespan_core::decisions::service::set_regenerate(Box::new(move || {
        if !gespan_core::decisions::enabled() {
            return;
        }
        let store = store.clone();
        std::thread::spawn(move || {
            write_harness_files(&store, true);
            // Entries changed outside the log's own writes (My tasks saves
            // and edits) get their vectors here; nothing while it is off.
            refresh_vectors();
        });
    }));
}

/// Whether search by meaning should be on: its setting, and the log's.
pub(crate) fn meaning_wanted(prefs: &crate::settings::Preferences) -> bool {
    prefs.decision_log && prefs.decision_meaning
}

/// What the "Find related decisions by meaning" row says, including how the
/// model is doing.
pub(crate) fn meaning_description() -> String {
    let about = "Match decisions by what they mean, not only their words, when agents \
                 search and when you review a proposal. Downloads a 137 MB model the first \
                 time and runs it on this computer.";
    if cfg!(not(feature = "nomic")) {
        return format!("{about} This build of Gespan does not include the model.");
    }
    match &*MEANING_STATUS.lock().unwrap_or_else(|e| e.into_inner()) {
        Some(status) => format!("{about} {status}"),
        None => about.to_string(),
    }
}

/// The last thing worth telling the user about the model (downloading, or
/// why it could not be used); `None` once it is in use or switched off.
static MEANING_STATUS: Mutex<Option<String>> = Mutex::new(None);
/// What search by meaning was last switched to, so each change is applied
/// once.
static MEANING: Mutex<Option<bool>> = Mutex::new(None);
/// Bumped on every switch, so a slow load that was switched off meanwhile
/// does not turn it back on.
static MEANING_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn set_meaning_status(status: Option<String>) {
    *MEANING_STATUS.lock().unwrap_or_else(|e| e.into_inner()) = status;
}

/// Loads the embedder for search by meaning; given whether its switch is
/// still the latest, so a long download can stop early. `None` when it was
/// switched again meanwhile.
type Loader = dyn Fn(&dyn Fn() -> bool) -> Result<Option<Arc<dyn Embedder>>, String> + Send + Sync;

/// Applies the search-by-meaning switch: called with every preferences
/// apply. On: the model is downloaded if it is missing, loaded and handed to
/// the log, which embeds every entry that needs it. Off: the log goes back
/// to keyword search. All of it on its own thread, never the UI's, and
/// never under `locked`.
pub(crate) fn meaning_switched(on: bool) {
    switch_meaning(on, Arc::new(load_embedder));
}

/// [`meaning_switched`] with its loader given; the thread doing the work,
/// when the switch changed anything.
fn switch_meaning(on: bool, loader: Arc<Loader>) -> Option<std::thread::JoinHandle<()>> {
    use std::sync::atomic::Ordering;
    let previous = MEANING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .replace(on);
    if previous == Some(on) || (previous.is_none() && !on) {
        return None;
    }
    let generation = MEANING_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    Some(std::thread::spawn(move || {
        apply_meaning(on, generation, &*loader)
    }))
}

/// One switch's work. Switches run one at a time (`SWITCHING`), and each
/// re-checks under that lock that no later switch was made, so an "off"
/// made during a slow "on" always wins. The re-embedding pass runs after the
/// lock is released: a later "off" stops it before its next entry.
fn apply_meaning(on: bool, generation: u64, loader: &Loader) {
    use std::sync::atomic::Ordering;
    static SWITCHING: Mutex<()> = Mutex::new(());
    let current = || MEANING_GENERATION.load(Ordering::SeqCst) == generation;
    let embedder = {
        let _one_at_a_time = SWITCHING.lock().unwrap_or_else(|e| e.into_inner());
        if !current() {
            return;
        }
        if !on {
            set_meaning_status(None);
            let _ = gespan_core::decisions::service::with(|log| log.set_embedder(None));
            // Every decision goes back into the files.
            gespan_core::decisions::service::regenerate();
            return;
        }
        let embedder = match loader(&current) {
            Ok(Some(embedder)) => embedder,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%error, "the decision embedding model is not available");
                set_meaning_status(Some(error));
                return;
            }
        };
        if !current() {
            return;
        }
        match gespan_core::decisions::service::with(|log| log.switch_embedder(embedder.clone())) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, "search by meaning could not be turned on");
                set_meaning_status(Some(format!("Search by meaning failed: {error}")));
                return;
            }
            Err(_) => return,
        }
        set_meaning_status(None);
        embedder
    };
    if let Ok(Err(error)) =
        gespan_core::decisions::service::with(|log| log.reembed_or_turn_off(&embedder))
    {
        set_meaning_status(Some(format!("Embedding failed: {error}")));
    }
    // With it on, the files keep only preferences and pinned decisions (the
    // rest reach each turn); if the pass turned it back off, they get every
    // decision again. Either way they follow what the log is now.
    gespan_core::decisions::service::regenerate();
}

/// Downloads (when missing) and loads the Nomic model. `None` when it was
/// switched off while downloading.
#[cfg(feature = "nomic")]
fn load_embedder(current: &dyn Fn() -> bool) -> Result<Option<Arc<dyn Embedder>>, String> {
    use gespan_core::decisions::embed::{NOMIC_MODEL, Nomic};
    use gespan_models::{CancelToken, ModelStatus, ModelStore, default_root};
    let models = ModelStore::new(default_root());
    if models.status(&NOMIC_MODEL) != ModelStatus::Ready {
        set_meaning_status(Some("Downloading the model…".into()));
        let cancel = CancelToken::new();
        let downloaded = models.download(&NOMIC_MODEL, &cancel, |progress| {
            if !current() {
                cancel.cancel();
            }
            let percent = progress.downloaded * 100 / progress.total.max(1);
            set_meaning_status(Some(format!("Downloading the model… {percent}%")));
        });
        match downloaded {
            Ok(()) => {}
            Err(gespan_models::DownloadError::Cancelled) => return Ok(None),
            Err(e) => return Err(format!("The model could not be downloaded: {e}")),
        }
        if !current() {
            return Ok(None);
        }
    }
    set_meaning_status(Some("Loading the model…".into()));
    let nomic = Nomic::load(&models.dir(&NOMIC_MODEL)).map_err(|e| e.to_string())?;
    Ok(Some(Arc::new(nomic)))
}

/// This build has no embedding model: search stays keyword only.
#[cfg(not(feature = "nomic"))]
fn load_embedder(_current: &dyn Fn() -> bool) -> Result<Option<Arc<dyn Embedder>>, String> {
    Err("This build of Gespan has no embedding model, so search stays by words.".into())
}

/// Embeds the log's entries that need it, while search by meaning is on.
fn refresh_vectors() {
    match gespan_core::decisions::service::with(|log| log.reembed()) {
        Ok(Ok(_)) | Err(_) => {}
        Ok(Err(error)) => tracing::warn!(%error, "the decision log could not embed its entries"),
    }
}

/// Regenerates (or, with `regenerate` off, removes) every harness file for
/// each harness's real home (honouring its config variables), the app's
/// data dir, its accounts and its known checkouts. The core re-checks the
/// switch under its lock, so a run that lost a race with it does nothing.
fn write_harness_files(store: &Database, regenerate: bool) {
    use gespan_core::decisions::harness;
    let Ok((book, data_dir)) = locked(store).map(|s| {
        (
            s.provider_accounts().unwrap_or_default(),
            s.data_dir().map(Path::to_path_buf),
        )
    }) else {
        return;
    };
    let Some(data_dir) = data_dir else {
        return;
    };
    let workspaces = known_workspaces();
    let report = if regenerate {
        match gespan_core::decisions::service::with(|log| {
            harness::regenerate(log, &book, &data_dir, &harness::default_homes, &workspaces)
        }) {
            Ok(report) => report,
            Err(error) => {
                tracing::warn!(%error, "the decision log could not write harness files");
                return;
            }
        }
    } else {
        harness::remove_all(&book, &data_dir, &harness::default_homes, &workspaces)
    };
    for (path, error) in &report.errors {
        tracing::warn!(path = %path.display(), %error, "a decision log harness file was not written");
    }
    if report.user_edits_overwritten.is_empty() {
        return;
    }
    let notices = match locked(store) {
        Ok(store) => unseen_edit_notices(&store, &report.user_edits_overwritten),
        Err(_) => return,
    };
    for notice in notices {
        let _ = EDIT_NOTICES.0.try_send(notice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::agent::Settings;
    use std::path::Path;

    fn row(role: &str, text: &str, first: bool) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first,
            sent_at: None,
        }
    }

    /// An answered question as the transcript keeps it: its output is the
    /// provider's text, not the user's.
    fn question_tool() -> String {
        let mut call = gespan_core::tool::ToolCall::new("ask-1", "AskUserQuestion");
        call.input = r#"{"questions":[{"question":"Use SQLite?"}]}"#.into();
        call.output = "The user answered: SQLite".into();
        call.state = gespan_core::tool::ToolState::Completed;
        serde_json::to_string(&call).unwrap()
    }

    /// A conversation where Gespan's context, the agent, tools, attachments
    /// and app messages all say "SQLite"; only the user never does.
    fn conversation() -> Vec<Row> {
        vec![
            row(
                "user",
                "<gespan_context>\nUse SQLite for everything.\n</gespan_context>\n\n<gespan_browser>\nbrowser SQLite\n</gespan_browser>\nKeep the log in one Postgres table",
                true,
            ),
            row("user-attachments", r#"[{"name":"SQLite.md"}]"#, true),
            row("assistant", "Should we use SQLite?", true),
            row("tool", &question_tool(), true),
            row("reasoning", "SQLite is simpler", true),
            row("pilot-system", "Session 4 says use SQLite", true),
            row("user", "Tabs, not ", true),
            row("user", "spaces", false),
        ]
    }

    /// The tests share the process-wide answers: each holds this and starts
    /// with none, so `cargo test`'s threads cannot see each other's.
    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        static TESTS: Mutex<()> = Mutex::new(());
        let guard = TESTS.lock().unwrap_or_else(|e| e.into_inner());
        ANSWERS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        guard
    }

    fn typed_settings() -> Settings {
        Settings {
            user_typed: true,
            ..Settings::default()
        }
    }

    /// Saves `rows` as a session's history, recording each `user` message as
    /// the human's when `by_user` says so, the way a turn records it.
    fn session(store: &mut Store, rows: &[(Row, bool)]) -> i64 {
        let id = store.create("codex", "/project").unwrap().id;
        let plain: Vec<Row> = rows.iter().map(|(row, _)| row.clone()).collect();
        store.write_rows(id, 0, &plain).unwrap();
        let mut opened = false;
        for (row, by_user) in rows {
            if row.role == "user" && row.first {
                let opens = !opened;
                opened = true;
                if *by_user {
                    let text = whole(rows, row);
                    record_typed(store, id, &text, opens).unwrap();
                }
            }
        }
        id
    }

    /// A message's text rejoined from its fragments.
    fn whole(rows: &[(Row, bool)], first: &Row) -> String {
        let start = rows
            .iter()
            .position(|(row, _)| std::ptr::eq(row, first))
            .unwrap();
        let mut text = first.text.clone();
        for (row, _) in &rows[start + 1..] {
            if row.first {
                break;
            }
            text.push_str(&row.text);
        }
        text
    }

    #[test]
    fn decision_user_texts_skip_injected_context_and_agent_text() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let rows: Vec<(Row, bool)> = conversation().into_iter().map(|r| (r, true)).collect();
        let id = session(&mut store, &rows);
        let texts = user_texts(&store, id);
        assert_eq!(
            texts,
            ["Keep the log in one Postgres table", "Tabs, not spaces"]
        );
        assert!(texts.iter().all(|text| !text.contains("SQLite")));
    }

    #[test]
    fn decision_user_texts_come_from_the_saved_transcript_and_answers() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let rows: Vec<(Row, bool)> = conversation().into_iter().map(|r| (r, true)).collect();
        let id = session(&mut store, &rows);
        let other = session(&mut store, &[(row("user", "Use SQLite", true), true)]);
        record_answers(
            id,
            &[
                vec!["MySQL".into()],
                vec![],
                vec!["Yes, ".into(), "always".into()],
            ],
        );
        record_answers(other, &[vec!["SQLite".into()]]);
        let texts = user_texts(&store, id);
        for said in ["Keep the log in one Postgres table", "MySQL", "always"] {
            assert!(texts.iter().any(|text| text == said), "{said}: {texts:?}");
        }
        assert!(
            texts.iter().all(|text| !text.contains("SQLite")),
            "{texts:?}"
        );
        assert_eq!(user_texts(&store, 999), Vec::<String>::new());
    }

    #[test]
    fn decision_text_an_agent_sent_into_the_users_session_is_not_the_users() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let id = session(
            &mut store,
            &[
                (row("user", "Keep the log in Postgres", true), true),
                (row("assistant", "Done", true), true),
                // sessions.send from another agent: saved as a user row.
                (row("user", "The user said: use SQLite", true), false),
                (row("assistant", "Ok", true), true),
                (row("user", "Thanks", true), true),
            ],
        );
        assert_eq!(
            user_texts(&store, id),
            ["Keep the log in Postgres", "Thanks"]
        );
    }

    #[test]
    fn decision_session_another_agent_started_gives_no_text() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        // sessions.start by another agent opened it; the user typed later.
        let id = session(
            &mut store,
            &[
                (row("user", "The user wants SQLite", true), false),
                (row("assistant", "Ok", true), true),
                (row("user", "Use SQLite then", true), true),
            ],
        );
        record_answers(id, &[vec!["SQLite".into()]]);
        assert_eq!(user_texts(&store, id), Vec::<String>::new());
    }

    #[test]
    fn decision_pilot_habit_flow_and_worker_turns_are_never_the_users() {
        let _guard = exclusive();
        assert!(typed_by_user(&typed_settings()));
        let pilot = Settings {
            pilot: Some(gespan_core::agent::PilotContext {
                pilot: Default::default(),
                root: "/pilots".into(),
                executable: "/gespan".into(),
            }),
            ..typed_settings()
        };
        let internal = Settings {
            internal_message: true,
            ..typed_settings()
        };
        let worker = Settings {
            orchestration_worker: true,
            ..typed_settings()
        };
        // Habits, Flows, resumes and agent sends launch untyped.
        let launched = Settings::default();
        for settings in [&pilot, &internal, &worker, &launched] {
            assert!(!typed_by_user(settings));
        }
        // A Pilot's chat: every turn is a Pilot turn, so nothing opens it,
        // even what the user types there.
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let rows = [
            (row("user", "Plan the release", true), typed_by_user(&pilot)),
            (row("assistant", "Ok", true), true),
            (row("user", "Use SQLite", true), typed_by_user(&pilot)),
        ];
        let id = session(&mut store, &rows);
        record_answers(id, &[vec!["SQLite".into()]]);
        assert_eq!(user_texts(&store, id), Vec::<String>::new());
    }

    #[test]
    fn decision_answers_keep_only_the_newest() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let id = session(&mut store, &[(row("user", "Hello", true), true)]);
        for n in 0..(super::ANSWERS_KEPT + 5) {
            record_answers(id, &[vec![format!("answer {n}")]]);
        }
        let texts = user_texts(&store, id);
        assert_eq!(texts.len(), super::ANSWERS_KEPT + 1);
        assert!(!texts.contains(&"answer 0".to_owned()));
        assert!(texts.contains(&format!("answer {}", super::ANSWERS_KEPT + 4)));
    }

    #[test]
    fn decision_log_source_reads_the_app_database_off_the_ui_thread() {
        let _guard = exclusive();
        let mut store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let id = session(
            &mut store,
            &[(row("user", "Ship it on Friday", true), true)],
        );
        // Only the source: the harness writer would write to the real homes.
        install_user_text_source(std::sync::Arc::new(std::sync::Mutex::new(store)));
        let texts = std::thread::spawn(move || gespan_core::decisions::service::user_texts(id))
            .join()
            .unwrap();
        assert_eq!(texts, ["Ship it on Friday"]);
    }

    #[test]
    fn decision_edit_notice_shows_once_per_path() {
        let store = Store::open_gespan(Path::new(":memory:")).unwrap();
        let claude = PathBuf::from("home").join(".claude").join("CLAUDE.md");
        let codex = PathBuf::from("home").join(".codex").join("AGENTS.md");
        let first = unseen_edit_notices(&store, std::slice::from_ref(&claude));
        assert_eq!(
            first,
            [format!(
                "Gespan replaced your edit inside its section of {}. Edit decisions in My tasks instead.",
                claude.display()
            )]
        );
        assert_eq!(
            store
                .setting(&format!("decision-edit-notice:{}", claude.display()))
                .unwrap()
                .as_deref(),
            Some("shown")
        );
        // The same file again stays quiet; another file still gets its one.
        let next = unseen_edit_notices(&store, &[claude.clone(), codex.clone()]);
        assert_eq!(next, [edit_notice_text(&codex)]);
        assert!(unseen_edit_notices(&store, &[claude, codex]).is_empty());
    }

    #[test]
    fn decision_switch_regenerates_when_on_and_cleans_up_only_when_turned_off() {
        // At startup: on writes the files, off removes only what an earlier
        // run recorded.
        assert_eq!(switch_action(None, true), Some(SwitchAction::Regenerate));
        assert_eq!(
            switch_action(None, false),
            Some(SwitchAction::RemoveLeftovers)
        );
        // Turning it off removes what Gespan wrote; on writes it back.
        assert_eq!(
            switch_action(Some(true), false),
            Some(SwitchAction::RemoveAll)
        );
        assert_eq!(
            switch_action(Some(false), true),
            Some(SwitchAction::ImportThenRegenerate)
        );
        // Other preference changes re-apply an unchanged switch: nothing.
        assert_eq!(switch_action(Some(true), true), None);
        assert_eq!(switch_action(Some(false), false), None);
    }

    /// Each action's steps, in the order they ran.
    fn steps_of(action: SwitchAction, leftovers: bool) -> Vec<&'static str> {
        let ran = std::cell::RefCell::new(Vec::new());
        {
            let step = |name: &'static str| {
                let ran = &ran;
                move || ran.borrow_mut().push(name)
            };
            let (rescope, import, regenerate, remove) = (
                step("rescope"),
                step("import"),
                step("regenerate"),
                step("remove"),
            );
            run_switch(
                &action,
                &SwitchSteps {
                    rescope: &rescope,
                    import: &import,
                    regenerate: &regenerate,
                    remove: &remove,
                    has_leftovers: &|| leftovers,
                },
            );
        }
        ran.into_inner()
    }

    /// Old decisions move to their project's scope only while the log is
    /// on, before the import and the files read them; off at startup cleans
    /// up only what an earlier run recorded writing.
    #[test]
    fn decision_switch_rescopes_only_when_on_and_cleans_leftovers_when_off() {
        assert_eq!(
            steps_of(SwitchAction::Regenerate, false),
            ["rescope", "regenerate"]
        );
        assert_eq!(
            steps_of(SwitchAction::ImportThenRegenerate, false),
            ["rescope", "import", "regenerate"]
        );
        assert_eq!(steps_of(SwitchAction::RemoveAll, false), ["remove"]);
        assert_eq!(steps_of(SwitchAction::RemoveLeftovers, true), ["remove"]);
        assert!(steps_of(SwitchAction::RemoveLeftovers, false).is_empty());
    }

    #[test]
    fn decision_import_reads_the_injected_homes_and_only_once() {
        use gespan_core::decisions::{Config, DecisionStore, Status};
        use gespan_core::provider::Kind;
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        let codex = dir.path().join("codex");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(
            claude.join("CLAUDE.md"),
            "# Style\n\n- Prefer small functions.\n",
        )
        .unwrap();
        std::fs::write(codex.join("AGENTS.md"), "- Run tests before committing.\n").unwrap();
        let homes = |kind: Kind| match kind {
            Kind::Claude => Some(claude.clone()),
            Kind::Codex => Some(codex.clone()),
            _ => None,
        };
        let log = DecisionStore::open(&dir.path().join("gespan.db"), Config::app()).unwrap();
        assert_eq!(import_from_homes(&log, &homes, 1).unwrap(), 2);
        assert_eq!(import_from_homes(&log, &homes, 2).unwrap(), 0);
        let pending = log.pending().unwrap();
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().all(|e| e.status == Status::Pending));
    }

    #[test]
    fn decision_known_projects_report_changes_only() {
        let _guard = exclusive();
        let saved = KNOWN_PROJECTS.lock().unwrap().clone();
        let a = vec!["/a".to_string(), "/b".to_string()];
        set_known_projects(&[]);
        assert!(set_known_projects(&a));
        assert!(!set_known_projects(&a));
        assert!(set_known_projects(&a[..1]));
        *KNOWN_PROJECTS.lock().unwrap() = saved;
    }

    #[test]
    fn decision_first_session_in_an_unknown_checkout_schedules_one_regenerate() {
        let _guard = exclusive();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        gespan_core::decisions::service::set_regenerate(Box::new(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        let project = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        let saved = KNOWN_PROJECTS.lock().unwrap().clone();
        set_known_projects(&[project.path().to_string_lossy().into_owned()]);
        // An open project is already written for.
        assert!(!note_session_workspace(project.path()));
        // A new checkout: one regeneration, and it joins the known ones.
        assert!(note_session_workspace(worktree.path()));
        assert!(!note_session_workspace(worktree.path()));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(known_workspaces().contains(&worktree.path().to_path_buf()));
        *KNOWN_PROJECTS.lock().unwrap() = saved;
        SESSION_CHECKOUTS.lock().unwrap().clear();
    }

    /// The decision log on a gespan.db this process's tests share, search
    /// by meaning off and its switch never applied.
    fn meaning_log() -> std::sync::MutexGuard<'static, ()> {
        static DIR: LazyLock<tempfile::TempDir> = LazyLock::new(|| tempfile::tempdir().unwrap());
        let guard = exclusive();
        gespan_core::decisions::init(&DIR.path().join("gespan.db")).unwrap();
        *MEANING.lock().unwrap() = None;
        gespan_core::decisions::service::with(|log| log.set_embedder(None).unwrap()).unwrap();
        guard
    }

    fn model_in_use() -> Option<String> {
        gespan_core::decisions::service::with(|log| {
            log.embedder().map(|e| e.model_id().to_string())
        })
        .unwrap()
    }

    fn hash_loader() -> Arc<Loader> {
        Arc::new(|_: &dyn Fn() -> bool| {
            Ok(Some(
                Arc::new(gespan_core::decisions::HashEmbedder::new()) as Arc<dyn Embedder>
            ))
        })
    }

    #[test]
    fn decision_meaning_on_hands_the_log_an_embedder_and_off_takes_it_away() {
        let _log = meaning_log();
        // Off before it was ever on: nothing to do.
        assert!(switch_meaning(false, hash_loader()).is_none());
        switch_meaning(true, hash_loader()).unwrap().join().unwrap();
        assert_eq!(model_in_use().as_deref(), Some("hash-v1"));
        // The same switch again changes nothing.
        assert!(switch_meaning(true, hash_loader()).is_none());
        switch_meaning(false, hash_loader())
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(model_in_use(), None);
    }

    /// Files follow the switch: with search by meaning on they hold only
    /// preferences and pinned decisions, so each switch rewrites them, once
    /// its work is done.
    #[test]
    fn decision_meaning_switch_regenerates_the_harness_files() {
        let _log = meaning_log();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        gespan_core::decisions::service::set_regenerate(Box::new(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        let count = || calls.load(std::sync::atomic::Ordering::SeqCst);
        switch_meaning(true, hash_loader()).unwrap().join().unwrap();
        assert_eq!(count(), 1);
        switch_meaning(false, hash_loader())
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(count(), 2);
        // A model that cannot load changes nothing, so nothing is rewritten.
        let broken: Arc<Loader> = Arc::new(|_: &dyn Fn() -> bool| Err("no model".into()));
        switch_meaning(true, broken).unwrap().join().unwrap();
        assert_eq!(count(), 2);
        *MEANING.lock().unwrap() = None;
    }

    /// The user turns it off while the model is still loading: the slow
    /// "on" finishes after the "off" was made, and must not win.
    #[test]
    fn decision_meaning_off_during_a_slow_on_leaves_it_off() {
        let _log = meaning_log();
        let (started, loading) = std::sync::mpsc::channel::<()>();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let released = Mutex::new(released);
        let slow: Arc<Loader> = Arc::new(move |_: &dyn Fn() -> bool| {
            started.send(()).unwrap();
            released.lock().unwrap().recv().unwrap();
            Ok(Some(
                Arc::new(gespan_core::decisions::HashEmbedder::new()) as Arc<dyn Embedder>
            ))
        });
        let on = switch_meaning(true, slow).unwrap();
        loading.recv().unwrap();
        let off = switch_meaning(false, hash_loader()).unwrap();
        release.send(()).unwrap();
        on.join().unwrap();
        off.join().unwrap();
        assert_eq!(model_in_use(), None);
        // And on again after that works.
        switch_meaning(true, hash_loader()).unwrap().join().unwrap();
        assert_eq!(model_in_use().as_deref(), Some("hash-v1"));
    }
}
