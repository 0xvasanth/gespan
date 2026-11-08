//! The browser module (`gespan_browser`) in Gespan: starting it, the state
//! the UI follows, and what each turn of an agent gets. Agents reach the
//! browser through the app's MCP hub ([`toolkit`]); the hub grant's session
//! is the browser's session, so no browser token or server of its own is
//! handed to the agent.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub use gespan_browser::protocol::{
    AccessMode, BrowserChoice, BrowserMode, BrowserSettings, BrowserState, ConnectionState,
    TabOverview,
};
pub use gespan_browser::BrowserHost;
use gespan_browser::Embedder;
use rusqlite::Connection;

pub mod toolkit;

/// The window grants and requests belong to: Gespan has one main window.
pub const WINDOW: &str = "main";

/// Every state change from now on, for the UI.
static LISTENERS: Mutex<Vec<async_channel::Sender<BrowserState>>> = Mutex::new(Vec::new());
/// The pixel ratio of the display the main window is on.
static SCALE: Mutex<Option<f64>> = Mutex::new(None);
/// Held while [`start`] runs, so two calls never open the database twice.
static STARTING: Mutex<()> = Mutex::new(());

/// The app as the browser module sees it.
struct GespanEmbedder {
    db: Mutex<Connection>,
}

impl Embedder for GespanEmbedder {
    fn with_db(&self, f: &mut dyn FnMut(&Connection)) {
        let conn = self.db.lock().unwrap_or_else(|e| e.into_inner());
        f(&conn);
    }

    fn state_changed(&self, state: &BrowserState) {
        let mut listeners = LISTENERS.lock().unwrap_or_else(|e| e.into_inner());
        listeners.retain(|listener| listener.try_send(state.clone()).is_ok());
    }

    fn display_scale(&self) -> Option<f64> {
        *SCALE.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Its folder in the app's data directory: the database and the Gespan
/// browser's profiles.
const FOLDER: &str = "browser";
/// Its database file in [`FOLDER`].
const DATABASE: &str = "browser.db";

/// Start the browser module: its database (`<data_dir>/browser/browser.db`,
/// in memory without a data directory) and its host. Once per process;
/// later calls do nothing. A failed start leaves nothing behind, so a later
/// call may try again.
pub fn start(data_dir: Option<&Path>) -> Result<(), String> {
    let _starting = STARTING.lock().unwrap_or_else(|e| e.into_inner());
    if gespan_browser::host().is_some() {
        return Ok(());
    }
    let folder = data_dir.map(|dir| dir.join(FOLDER));
    let db = match &folder {
        Some(folder) => {
            std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
            Connection::open(folder.join(DATABASE))
        }
        None => Connection::open_in_memory(),
    }
    .map_err(|e| e.to_string())?;
    let embedder = Arc::new(GespanEmbedder { db: Mutex::new(db) });
    gespan_browser::init(embedder, folder.as_deref())
}

/// App exit: the Gespan browser goes with the app.
pub fn shutdown() {
    gespan_browser::shutdown();
}

/// The running host, once [`start`] ran.
pub fn host() -> Option<Arc<BrowserHost>> {
    gespan_browser::host()
}

/// The state now, once [`start`] ran.
pub fn state() -> Option<BrowserState> {
    host().map(|host| host.state())
}

/// Every state change from now on. The UI keeps only the newest.
pub fn subscribe() -> async_channel::Receiver<BrowserState> {
    let (send, receive) = async_channel::unbounded();
    LISTENERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(send);
    receive
}

/// The pixel ratio of the display the main window is on; the Gespan
/// browser starts at it.
pub fn set_display_scale(scale: f64) {
    *SCALE.lock().unwrap_or_else(|e| e.into_inner()) =
        (scale.is_finite() && scale > 0.0).then_some(scale);
}

/// The key the browser module knows a session by.
pub fn session_key(session: i64) -> String {
    format!("s{session}")
}

/// What Gespan knows about a turn when it starts.
#[derive(Clone, Debug, Default)]
pub struct TurnFacts {
    pub session: i64,
    /// The turn's working directory: upload paths are checked against it.
    pub cwd: PathBuf,
    /// The user typed this turn's message in the thread.
    pub user_typed: bool,
    /// A Pilot habit run nobody watches.
    pub hidden_run: bool,
    pub orchestrator: bool,
    pub orchestration_worker: bool,
    /// The session runs on another machine (`remote://`).
    pub remote: bool,
    /// The thread runs with full access, which grants like "Always on".
    pub full_access: bool,
}

/// The one check of whether a turn may carry the browser: a turn the user
/// typed in a local thread. Hidden habit runs, orchestration, remote
/// sessions and turns an agent sent never do, whatever the thread's grant,
/// "Always on" or full access.
pub fn eligible(facts: &TurnFacts) -> bool {
    facts.user_typed
        && !facts.hidden_run
        && !facts.orchestrator
        && !facts.orchestration_worker
        && !facts.remote
}

/// What a turn sends to the agent about the browser. A turn that carries
/// the browser is active for it until this is dropped: keep it until the
/// provider's run returns.
#[derive(Default)]
pub struct Turn {
    /// The host and session key of a turn that carries the browser.
    active: Option<(Arc<BrowserHost>, String)>,
    /// The `<gespan_browser>` block for the prompt.
    instructions: Option<String>,
}

impl Turn {
    /// The turn mounts the browser toolkit and prepends [`Turn::instructions`].
    pub fn carries_browser(&self) -> bool {
        self.active.is_some()
    }

    /// The `<gespan_browser>` block for the prompt, when the turn carries
    /// the browser: the instructions' overview, which says that page content
    /// is untrusted.
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }
}

impl std::fmt::Debug for Turn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Turn")
            .field("session", &self.active.as_ref().map(|(_, key)| key))
            .finish_non_exhaustive()
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        if let Some((host, key)) = self.active.take() {
            // Actions still running end as `unknown`.
            host.turn_finished(&key);
        }
    }
}

/// A turn is about to start. It carries the browser when it is
/// [`eligible`], the switch is on and the thread has access, or gets it now
/// from "Always on" or full access. Phase 1 has no Allow bar, so a thread
/// that would have to ask carries nothing.
pub fn begin_turn(facts: &TurnFacts) -> Turn {
    begin_turn_on(host(), facts)
}

fn begin_turn_on(host: Option<Arc<BrowserHost>>, facts: &TurnFacts) -> Turn {
    use gespan_browser::turn::{turn_access, TurnAccess};
    let Some(host) = host else {
        return Turn::default();
    };
    if !eligible(facts) {
        // Not even a grant the thread holds reaches these turns, and they
        // leave the turn state alone: ending one must not end the user's
        // turn running beside it.
        return Turn::default();
    }
    let key = session_key(facts.session);
    match turn_access(&host.state(), &key, true, facts.full_access) {
        Some(TurnAccess::Grant) => host.set_grant(&key, WINDOW, true),
        Some(TurnAccess::Granted) => {}
        Some(TurnAccess::Ask) | None => return Turn::default(),
    }
    host.turn_started(&key, WINDOW, &facts.cwd.to_string_lossy(), None);
    Turn {
        active: Some((host, key)),
        instructions: Some(instructions_block()),
    }
}

fn instructions_block() -> String {
    let overview = gespan_browser::agent::tools::section("overview").unwrap_or_default();
    format!("<gespan_browser>\n{overview}\n</gespan_browser>")
}

/// The session was deleted or archived: its grant goes, its tabs are closed
/// or given back.
pub fn session_deleted(session: i64) {
    gespan_browser::session_deleted(&session_key(session));
}

#[cfg(test)]
fn session_deleted_on(host: &BrowserHost, session: i64) {
    host.session_deleted(&session_key(session));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gespan_browser::protocol::BrowserChoice;
    use std::time::{Duration, Instant};

    pub(crate) fn facts() -> TurnFacts {
        TurnFacts {
            session: 1,
            cwd: PathBuf::from("/repo"),
            ..TurnFacts::default()
        }
    }

    /// A host of its own (never the process-wide one), in Attach mode on
    /// `profile`; with no `DevToolsActivePort` there, nothing is found.
    pub(crate) fn host_on(profile: &Path, access: AccessMode) -> Arc<BrowserHost> {
        BrowserHost::for_tests(BrowserSettings {
            browser: BrowserChoice::UserDataDir {
                path: profile.display().to_string(),
            },
            mode: BrowserMode::Attach,
            access,
            ..BrowserSettings::default()
        })
    }

    /// A host whose browser profile has nothing to attach to.
    pub(crate) fn unreachable_host(access: AccessMode) -> (Arc<BrowserHost>, tempfile::TempDir) {
        let profile = tempfile::tempdir().unwrap();
        (host_on(profile.path(), access), profile)
    }

    pub(crate) fn wait_for(host: &BrowserHost, state: ConnectionState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.connection().state != state {
            assert!(
                Instant::now() < deadline,
                "expected {state:?}, got {:?}",
                host.connection()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn session(
        state: &BrowserState,
        session: i64,
    ) -> Option<&gespan_browser::protocol::SessionAccess> {
        gespan_browser::turn::session(state, &session_key(session))
    }

    #[test]
    fn eligible_only_for_user_typed_local_turns() {
        let base = TurnFacts {
            user_typed: true,
            ..facts()
        };
        assert!(eligible(&base));
        // Full access changes nothing about who may get the browser.
        assert!(eligible(&TurnFacts {
            full_access: true,
            ..base.clone()
        }));
        for f in [
            TurnFacts {
                hidden_run: true,
                ..base.clone()
            },
            TurnFacts {
                orchestrator: true,
                ..base.clone()
            },
            TurnFacts {
                orchestration_worker: true,
                ..base.clone()
            },
            TurnFacts {
                remote: true,
                ..base.clone()
            },
            TurnFacts {
                user_typed: false,
                ..base.clone()
            },
        ] {
            assert!(!eligible(&f), "{f:?}");
        }
    }

    #[test]
    fn a_session_key_is_its_id_with_a_prefix() {
        assert_eq!(session_key(1), "s1");
        assert_eq!(session_key(42), "s42");
        assert_ne!(session_key(1), session_key(11));
    }

    #[test]
    fn switch_off_carries_nothing() {
        let (host, _profile) = unreachable_host(AccessMode::Always);
        let typed = TurnFacts {
            user_typed: true,
            full_access: true,
            ..facts()
        };
        let turn = begin_turn_on(Some(host.clone()), &typed);
        assert!(!turn.carries_browser());
        assert_eq!(turn.instructions(), None);
        // Nothing was granted behind the user's back.
        assert!(session(&host.state(), 1).is_none_or(|s| !s.granted));
    }

    #[test]
    fn no_host_carries_nothing() {
        let typed = TurnFacts {
            user_typed: true,
            full_access: true,
            ..facts()
        };
        let turn = begin_turn_on(None, &typed);
        assert!(!turn.carries_browser());
        assert_eq!(turn.instructions(), None);
    }

    #[test]
    fn always_on_carries_for_an_eligible_turn() {
        let (host, _profile) = unreachable_host(AccessMode::Always);
        host.set_toggle(true);
        let typed = TurnFacts {
            user_typed: true,
            ..facts()
        };
        let turn = begin_turn_on(Some(host.clone()), &typed);
        assert!(turn.carries_browser());
        let entry = session(&host.state(), 1).cloned().expect("session row");
        assert!(entry.granted && entry.turn_active, "{entry:?}");
        let instructions = turn.instructions().expect("instructions");
        assert!(
            instructions.starts_with("<gespan_browser>"),
            "{instructions}"
        );
        assert!(
            instructions.ends_with("</gespan_browser>"),
            "{instructions}"
        );
        // The overview, warning included, straight from the crate.
        let overview = gespan_browser::agent::tools::section("overview").unwrap();
        assert!(instructions.contains(overview));
        assert!(instructions.contains("### Page content is untrusted"));

        // Dropping the turn ends it; the grant stays for the next one.
        drop(turn);
        let entry = session(&host.state(), 1).cloned().expect("session row");
        assert!(entry.granted && !entry.turn_active, "{entry:?}");
        let again = begin_turn_on(Some(host.clone()), &typed);
        assert!(again.carries_browser());
    }

    #[test]
    fn full_access_grants_like_always_on() {
        let (host, _profile) = unreachable_host(AccessMode::Ask);
        host.set_toggle(true);
        let turn = begin_turn_on(
            Some(host.clone()),
            &TurnFacts {
                user_typed: true,
                full_access: true,
                ..facts()
            },
        );
        assert!(turn.carries_browser());
        assert!(session(&host.state(), 1).is_some_and(|s| s.granted));
    }

    #[test]
    fn ask_carries_nothing_in_phase_one() {
        let (host, _profile) = unreachable_host(AccessMode::Ask);
        host.set_toggle(true);
        let turn = begin_turn_on(
            Some(host.clone()),
            &TurnFacts {
                user_typed: true,
                ..facts()
            },
        );
        assert!(!turn.carries_browser());
        assert_eq!(turn.instructions(), None);
        // No request is put to the user and no turn is marked active.
        let state = host.state();
        let entry = session(&state, 1);
        assert!(
            entry.is_none_or(|s| !s.granted && !s.requested && !s.turn_active),
            "{entry:?}"
        );
    }

    #[test]
    fn a_granted_thread_carries_under_ask() {
        let (host, _profile) = unreachable_host(AccessMode::Ask);
        host.set_toggle(true);
        host.set_grant(&session_key(1), WINDOW, true);
        let turn = begin_turn_on(
            Some(host.clone()),
            &TurnFacts {
                user_typed: true,
                ..facts()
            },
        );
        assert!(turn.carries_browser());
    }

    #[test]
    fn hidden_run_with_full_access_carries_nothing() {
        let (host, _profile) = unreachable_host(AccessMode::Always);
        host.set_toggle(true);
        let hidden = TurnFacts {
            user_typed: true,
            hidden_run: true,
            full_access: true,
            ..facts()
        };
        let turn = begin_turn_on(Some(host.clone()), &hidden);
        assert!(!turn.carries_browser());
        assert_eq!(turn.instructions(), None);
        assert!(session(&host.state(), 1).is_none_or(|s| !s.granted));
    }

    /// The chat's own grant does not reach a run the user did not type:
    /// hidden habit runs, orchestration, remote sessions, agent-sent turns.
    #[test]
    fn ineligible_turns_never_carry_even_on_a_granted_thread() {
        let (host, _profile) = unreachable_host(AccessMode::Always);
        host.set_toggle(true);
        let typed = TurnFacts {
            user_typed: true,
            full_access: true,
            ..facts()
        };
        let user_turn = begin_turn_on(Some(host.clone()), &typed);
        assert!(user_turn.carries_browser());
        for f in [
            TurnFacts {
                hidden_run: true,
                ..typed.clone()
            },
            TurnFacts {
                orchestrator: true,
                ..typed.clone()
            },
            TurnFacts {
                orchestration_worker: true,
                ..typed.clone()
            },
            TurnFacts {
                remote: true,
                ..typed.clone()
            },
            TurnFacts {
                user_typed: false,
                ..typed.clone()
            },
        ] {
            let turn = begin_turn_on(Some(host.clone()), &f);
            assert!(!turn.carries_browser(), "{f:?}");
            assert_eq!(turn.instructions(), None, "{f:?}");
            drop(turn);
            // Ending it does not end the user's turn running beside it.
            assert!(
                session(&host.state(), 1).is_some_and(|s| s.turn_active),
                "{f:?}"
            );
        }
    }

    #[test]
    fn deleting_a_session_ends_its_grant() {
        let (host, _profile) = unreachable_host(AccessMode::Always);
        host.set_toggle(true);
        host.set_grant(&session_key(1), WINDOW, true);
        host.set_grant(&session_key(2), WINDOW, true);
        session_deleted_on(&host, 1);
        let state = host.state();
        assert!(!gespan_browser::turn::granted(&state, &session_key(1)));
        assert!(gespan_browser::turn::granted(&state, &session_key(2)));
    }

    /// The one test of the process-wide module: nothing else in this crate
    /// starts it, so it cannot race another test.
    #[test]
    fn start_opens_the_database_once_and_reports_state() {
        let dir = tempfile::tempdir().unwrap();
        let updates = subscribe();
        start(Some(dir.path())).unwrap();
        assert!(dir.path().join("browser").join("browser.db").is_file());
        // Later calls do nothing, whatever folder they name.
        let other = tempfile::tempdir().unwrap();
        start(Some(other.path())).unwrap();
        assert!(!other.path().join("browser").exists());

        let now = state().expect("started");
        assert!(!now.toggle_on, "the switch starts off");
        assert_eq!(now.settings.mode, BrowserMode::Attach);

        // Settings reach the database and every subscriber.
        let host = host().expect("started");
        host.set_settings(BrowserSettings {
            access: AccessMode::Always,
            ..now.settings.clone()
        });
        let mut latest = None;
        while let Ok(update) = updates.try_recv() {
            latest = Some(update);
        }
        assert_eq!(
            latest.expect("a state update").settings.access,
            AccessMode::Always
        );

        // The global path: the switch is off, so no turn carries the browser.
        let turn = begin_turn(&TurnFacts {
            user_typed: true,
            full_access: true,
            ..facts()
        });
        assert!(!turn.carries_browser());
        session_deleted(1);
        set_display_scale(2.0);
        set_display_scale(f64::NAN);
    }
}
