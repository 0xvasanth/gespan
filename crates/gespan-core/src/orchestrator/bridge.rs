//! How a session's `sessions` tool calls reach the app. The app holds a
//! [`Server`] and serves each local session it has loaded; a call from a
//! session with no route fails at once with [`UNAVAILABLE`].
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{mpsc, Mutex, OnceLock},
    time::Duration,
};

pub const UNAVAILABLE: &str =
    "Gespan isn't serving this session right now; retry after the app finishes loading it.";
const TIMED_OUT: &str = "Gespan didn't answer in time; retry.";
/// How long a call waits on the app, as the app bridge does.
const TIMEOUT: Duration = Duration::from_secs(60);

/// What a session asks the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The caller's sibling sessions.
    List { scope: Scope },
    /// One session's recent exchanges, older than the `before` cursor.
    Read { id: i64, before: Option<String> },
}

/// Which sessions a list covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Sessions in the caller's repository and its worktrees.
    Repo,
    /// Those, plus sessions in other repositories.
    All,
}

/// One call, as the app receives it. `session` is the caller, proven by its
/// hub credential.
#[derive(Debug)]
pub struct Request {
    pub session: i64,
    pub action: Action,
    pub reply: mpsc::Sender<Result<Value, String>>,
}

/// Which dispatcher serves each session.
type Routes = HashMap<i64, (u64, async_channel::Sender<Request>)>;

fn routes() -> &'static Mutex<Routes> {
    static ROUTES: OnceLock<Mutex<Routes>> = OnceLock::new();
    ROUTES.get_or_init(Default::default)
}

/// One dispatcher of `sessions` calls. Dropping it ends what it served.
pub struct Server {
    id: u64,
    requests: async_channel::Sender<Request>,
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
    pub fn start() -> (Self, async_channel::Receiver<Request>) {
        let (requests, rx) = async_channel::bounded(32);
        (
            Self {
                id: rand::random(),
                requests,
            },
            rx,
        )
    }

    /// Routes this session's calls to this dispatcher. Calling it again is
    /// harmless.
    pub fn serve(&self, session: i64) {
        routes()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session, (self.id, self.requests.clone()));
    }
}

/// Sends one call to the app serving `session` and waits for its answer.
pub fn dispatch(session: i64, action: Action) -> Result<Value, String> {
    let requests = routes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&session)
        .map(|(_, requests)| requests.clone())
        .ok_or(UNAVAILABLE)?;
    let (reply, answer) = mpsc::channel();
    requests
        .send_blocking(Request {
            session,
            action,
            reply,
        })
        .map_err(|_| UNAVAILABLE)?;
    match answer.recv_timeout(TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(TIMED_OUT.into()),
        // The app dropped the request without answering.
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(UNAVAILABLE.into()),
    }
}
