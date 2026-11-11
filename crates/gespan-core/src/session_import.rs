//! Sessions an agent kept in its own folder, found on disk so they can be
//! brought into Gespan. Nothing here writes to an agent's files.
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::outside_turns::{OutsideTurn, TurnRole};
use crate::provider::Kind;
use crate::store::{Session, Store};
use crate::transcript::{Interjection, Row, Transcript};

pub use placement::{already_imported, place, Placement, Skip};

mod claude;
mod codex;
mod cursor;
mod kiro;
mod placement;
#[cfg(test)]
mod tests;

/// Bytes read from the start of each file when listing, so a session of
/// hundreds of megabytes lists as fast as a small one.
pub(crate) const HEAD_LIMIT: u64 = 1 << 20;

/// One session found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub kind: Kind,
    /// The agent's own session id.
    pub provider_id: String,
    /// The folder the session ran in.
    pub cwd: String,
    /// The first user prompt, first line, trimmed.
    pub title: String,
    pub branch: Option<String>,
    /// File modification time, in Unix milliseconds.
    pub updated_at: u64,
    /// User and assistant turns; `None` until counted.
    pub messages: Option<u32>,
    /// The session file or folder.
    pub path: PathBuf,
}

/// Sessions of `kind` under its `home`, newest first.
pub fn list(kind: Kind, home: &Path) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = match kind {
        Kind::Claude => claude::list(home),
        Kind::Codex => codex::list(home),
        Kind::Cursor => cursor::list(home),
        Kind::Kiro => kiro::list(home),
        _ => Vec::new(),
    };
    found.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
    found
}

/// Every visible turn of the session, oldest first.
pub fn read_turns(candidate: &Candidate) -> Result<Vec<OutsideTurn>, String> {
    match candidate.kind {
        Kind::Claude => claude::read(&candidate.path),
        Kind::Codex => codex::read(&candidate.path),
        Kind::Cursor => cursor::read(&candidate.path),
        Kind::Kiro => kiro::read(&candidate.path),
        _ => Err("not supported".into()),
    }
}

/// How many user and assistant turns the session holds.
pub fn count_messages(candidate: &Candidate) -> Option<u32> {
    let turns = match candidate.kind {
        Kind::Claude => claude::read(&candidate.path).ok()?,
        Kind::Codex => codex::read(&candidate.path).ok()?,
        Kind::Cursor => cursor::read(&candidate.path).ok()?,
        Kind::Kiro => kiro::read(&candidate.path).ok()?,
        _ => return None,
    };
    u32::try_from(turns.len()).ok()
}

/// A prompt's first line, trimmed.
pub(crate) fn title_of(prompt: &str) -> String {
    prompt.lines().next().unwrap_or("").trim().to_string()
}

/// `custom_type` of the divider that opens an imported history.
pub const IMPORTED_TYPE: &str = "imported";

/// A session brought into Gespan.
#[derive(Debug, Clone)]
pub struct Imported {
    pub session: Session,
    /// The project it was filed under.
    pub project: String,
}

/// What an import did with each pick.
#[derive(Debug, Default)]
pub struct Report {
    pub imported: Vec<Imported>,
    pub skipped: Vec<(Candidate, Skip)>,
}

/// The agent's product name, as the divider shows it.
fn agent_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Kiro => "Kiro",
        other => other.label(),
    }
}

/// The imported history: a divider, then each turn with its send time.
fn rows_of(kind: Kind, turns: &[OutsideTurn]) -> Vec<Row> {
    let mut transcript = Transcript::default();
    let divider = Interjection {
        text: agent_name(kind).into(),
        custom_type: IMPORTED_TYPE.into(),
        severity: None,
    };
    let at = transcript.begin("interjection");
    if let Some(sent_at) = turns.first().and_then(|t| t.sent_at) {
        transcript.rows[at].sent_at = Some(sent_at);
    }
    transcript.append(&serde_json::to_string(&divider).expect("divider serializes"));
    for turn in turns {
        let at = transcript.begin(match turn.role {
            TurnRole::User => "user",
            TurnRole::Assistant => "assistant",
        });
        if let Some(sent_at) = turn.sent_at {
            transcript.rows[at].sent_at = Some(sent_at);
        }
        transcript.append(&turn.text);
    }
    transcript.rows
}

/// Brings each pick into Gespan as a resumable session. `projects` are the
/// Gespan projects, `root` maps a linked worktree to its main checkout and
/// `home` gives each agent's default home, where its checkpoint is taken.
///
/// Files are read and checkpoints saved outside the store lock; the lock is
/// held only to re-check for a duplicate and create the session, so two
/// imports of one session never both land. One bad pick never stops the rest.
pub fn import(
    store: &Mutex<Store>,
    picks: &[Candidate],
    projects: &[String],
    root: impl Fn(&str) -> Option<String>,
    home: impl Fn(Kind) -> Option<PathBuf>,
) -> Report {
    let mut report = Report::default();
    for pick in picks {
        let (project, worktree) = match place(&pick.cwd, projects, &root) {
            Placement::Project { project } => (project, None),
            Placement::Worktree { project, worktree } => (project, Some(worktree)),
            Placement::Skip(why) => {
                report.skipped.push((pick.clone(), why));
                continue;
            }
        };
        let turns = match read_turns(pick) {
            Ok(turns) => turns,
            Err(error) => {
                report.skipped.push((pick.clone(), Skip::Unreadable(error)));
                continue;
            }
        };
        let rows = rows_of(pick.kind, &turns);
        let settings = crate::agent::Settings {
            account: crate::accounts::Profile::default(),
            worktree_cwd: worktree,
            ..Default::default()
        };
        match create(store, pick, &project, &settings, &rows) {
            Ok(Some(session)) => {
                save_checkpoint(store, pick, session.id, &home);
                report.imported.push(Imported { session, project });
            }
            Ok(None) => report.skipped.push((pick.clone(), Skip::AlreadyImported)),
            Err(error) => report.skipped.push((pick.clone(), Skip::Unreadable(error))),
        }
    }
    report
}

/// Creates the session under the store lock; `None` when Gespan already
/// holds it. A session whose provider id cannot be saved is removed again,
/// since it could not resume.
fn create(
    store: &Mutex<Store>,
    pick: &Candidate,
    project: &str,
    settings: &crate::agent::Settings,
    rows: &[Row],
) -> Result<Option<Session>, String> {
    let mut store = store.lock().unwrap_or_else(|e| e.into_inner());
    if already_imported(&store.sessions()?, pick.kind, &pick.provider_id) {
        return Ok(None);
    }
    let mut session = store.create_from(pick.kind.id(), project, &pick.title, settings, rows)?;
    if let Err(error) = store.provider_id(session.id, &pick.provider_id) {
        let _ = store.delete(session.id);
        return Err(error);
    }
    session.provider_id = Some(pick.provider_id.clone());
    Ok(Some(session))
}

/// Marks everything now in the agent's file as already shown, so the
/// terminal sync picks up only what is added later. Best effort.
fn save_checkpoint(
    store: &Mutex<Store>,
    pick: &Candidate,
    session: i64,
    home: &impl Fn(Kind) -> Option<PathBuf>,
) {
    if !matches!(pick.kind, Kind::Claude | Kind::Codex) {
        return;
    }
    let Some(home) = home(pick.kind) else {
        return;
    };
    let Some(checkpoint) =
        crate::outside_turns::checkpoint_at_end(pick.kind, &home, &pick.provider_id, None)
    else {
        return;
    };
    let saved = serde_json::to_string(&checkpoint)
        .map_err(|e| e.to_string())
        .and_then(|value| {
            store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .save_setting(&crate::outside_turns::checkpoint_key(session), &value)
        });
    if let Err(error) = saved {
        tracing::warn!(%error, "could not save the sync checkpoint of an imported session");
    }
}
