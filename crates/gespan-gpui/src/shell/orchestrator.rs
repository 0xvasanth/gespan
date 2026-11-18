//! The app's side of the `sessions` tools: which other sessions work in the
//! caller's repository, in its main checkout or any of its worktrees.
//!
//! Git and the store are only read off the UI thread. What they tell about a
//! session (its goal, the files it changed, its repository and branch) is
//! kept as [`Facts`], so the roster built at the start of a turn reads
//! nothing slow.
use super::Shell;
use crate::runtime::locked;
use gespan_core::orchestrator::{
    bridge::{Action, Request, Scope, Server},
    repo,
    roster::{self, Member, Status, Workspace},
};
use gpui::{App, Context};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// What git and the transcript say about one session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Facts {
    /// The first line of the session's first message.
    pub goal: String,
    /// Files its own edits touched that are still uncommitted.
    pub files: Vec<String>,
    pub repo: Option<PathBuf>,
    /// The checked-out branch, for a session in a worktree.
    pub branch: Option<String>,
}

/// A live session's status. Hold outranks the rest; a turn blocked on an
/// approval or question is waiting on the user rather than working.
pub(super) fn live_status(held: bool, working: bool, waiting: bool) -> Status {
    if held {
        Status::Held
    } else if waiting {
        Status::WaitingOnUser
    } else if working {
        Status::Working
    } else {
        Status::Idle
    }
}

const UNREADABLE: &str = "That session isn't readable here.";

/// What a session's facts are computed from, captured on the UI thread.
struct Source {
    id: i64,
    cwd: String,
    in_worktree: bool,
}

impl Shell {
    pub(super) fn init_orchestrator_bridge(&mut self, cx: &mut Context<Self>) {
        if cx.is_test() {
            return;
        }
        let (server, requests) = Server::start();
        self.orchestrator_bridge = Some(server);
        cx.spawn(async move |this, cx| {
            while let Ok(request) = requests.recv().await {
                if this
                    .update(cx, |this, cx| this.answer_sessions(request, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Lets `id`'s agent reach this window through its `sessions` tools.
    pub(super) fn serve_sessions(&self, id: i64) {
        if let Some(bridge) = &self.orchestrator_bridge {
            bridge.serve(id);
        }
    }

    /// Answers one `sessions_list` or `sessions_read` call. The reply is
    /// sent once, from a background task, so git never runs on the UI thread.
    pub(super) fn answer_sessions(&mut self, request: Request, cx: &mut Context<Self>) {
        let Request {
            session,
            action,
            reply,
        } = request;
        match action {
            Action::List { scope } => {
                let sources: Vec<Source> = self
                    .roster_candidates(cx)
                    .into_iter()
                    .filter(|id| *id == session || !self.session_facts.contains_key(id))
                    .filter_map(|id| self.facts_source(id, cx))
                    .collect();
                let store = self.store.clone();
                let work = cx
                    .background_executor()
                    .spawn(async move { collect_facts(&store, sources) });
                cx.spawn(async move |this, cx| {
                    let found = work.await;
                    let answer = this.update(cx, |this, cx| {
                        this.session_facts.extend(found);
                        this.list_answer(session, scope, cx)
                    });
                    let _ = reply.send(answer.unwrap_or_else(|_| {
                        Err(gespan_core::orchestrator::bridge::UNAVAILABLE.into())
                    }));
                })
                .detach();
            }
            Action::Read { id, before } => {
                let readable =
                    self.session_summaries.contains_key(&id) || self.sessions.contains_key(&id);
                let readable = readable && {
                    let metadata = self.session_metadata(id, cx);
                    !metadata.summary.archived && !metadata.summary.cwd.starts_with("remote://")
                };
                if !readable || id == session {
                    let _ = reply.send(Err(UNREADABLE.into()));
                    return;
                }
                let (Some((caller_cwd, _)), Some((target_cwd, _))) =
                    (self.session_cwd(session, cx), self.session_cwd(id, cx))
                else {
                    let _ = reply.send(Err(UNREADABLE.into()));
                    return;
                };
                let busy = self.session_metadata(id, cx).busy;
                let draft = self
                    .sessions
                    .get(&id)
                    .is_some_and(|v| v.read(cx).has_saved_draft())
                    || self.draft_sessions.contains(&id);
                let input = match before {
                    Some(before) => json!({ "before": before }),
                    None => json!({}),
                };
                let store = self.store.clone();
                cx.background_executor()
                    .spawn(async move {
                        // Only a sibling's transcript: one in the caller's repository.
                        let caller = repo::repo_key(Path::new(&caller_cwd));
                        if caller.is_none() || repo::repo_key(Path::new(&target_cwd)) != caller {
                            let _ = reply.send(Err(UNREADABLE.into()));
                            return;
                        }
                        let page = locked(&store)
                            .and_then(|store| store.app_conversation(id, &input, busy, draft));
                        let _ = reply.send(page);
                    })
                    .detach();
            }
        }
    }

    /// Re-reads `id`'s facts in the background, as when its turn ends.
    pub(super) fn refresh_session_facts(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(source) = self.facts_source(id, cx) else {
            return;
        };
        let store = self.store.clone();
        let work = cx
            .background_executor()
            .spawn(async move { collect_facts(&store, vec![source]) });
        cx.spawn(async move |this, cx| {
            let found = work.await;
            let _ = this.update(cx, |this, _| this.session_facts.extend(found));
        })
        .detach();
    }

    fn list_answer(&self, session: i64, scope: Scope, cx: &App) -> Result<Value, String> {
        let caller = self
            .roster_member(session, cx)
            .ok_or(gespan_core::orchestrator::bridge::UNAVAILABLE)?;
        let members: Vec<Member> = self
            .order
            .iter()
            .filter_map(|id| self.roster_member(*id, cx))
            .collect();
        let roster = roster::build(&caller, &members);
        let mut answer = json!({ "sessions": roster.entries, "omitted": roster.omitted });
        if scope == Scope::All {
            let listed: Vec<i64> = roster.entries.iter().map(|e| e.member.id).collect();
            let elsewhere: Vec<Value> = members
                .iter()
                .filter(|m| m.id != session && !m.archived && !m.remote && !listed.contains(&m.id))
                .filter(|m| caller.repo.is_none() || m.repo != caller.repo)
                .map(|m| {
                    json!({
                        "id": m.id,
                        "title": m.title,
                        "harness": m.harness,
                        "status": m.status,
                        // The folder's name only: other projects' paths stay private.
                        "project": Path::new(&m.cwd)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned()),
                    })
                })
                .collect();
            answer["elsewhere"] = json!(elsewhere);
        }
        Ok(answer)
    }

    /// Every local, unarchived session a roster could name.
    fn roster_candidates(&self, cx: &App) -> Vec<i64> {
        self.order
            .iter()
            .copied()
            .filter(|id| self.session_summaries.contains_key(id) || self.sessions.contains_key(id))
            .filter(|id| {
                let summary = self.session_metadata(*id, cx).summary;
                !summary.archived && !summary.cwd.starts_with("remote://")
            })
            .collect()
    }

    /// The folder `id` works in: its worktree when it has one.
    fn session_cwd(&self, id: i64, cx: &App) -> Option<(String, bool)> {
        let metadata = self.session_metadata(id, cx);
        let worktree = match self.sessions.get(&id).map(|view| view.read(cx)) {
            Some(live) => live
                .settings
                .worktree_cwd
                .clone()
                .filter(|_| !live.settings.worktree_removed),
            None => self.session_worktrees.get(&id).cloned().flatten(),
        };
        match worktree {
            Some(worktree) => Some((worktree, true)),
            None => Some((metadata.summary.cwd.clone(), false)),
        }
    }

    fn facts_source(&self, id: i64, cx: &App) -> Option<Source> {
        if !self.session_summaries.contains_key(&id) && !self.sessions.contains_key(&id) {
            return None;
        }
        let (cwd, in_worktree) = self.session_cwd(id, cx)?;
        (!cwd.starts_with("remote://")).then_some(Source {
            id,
            cwd,
            in_worktree,
        })
    }

    fn roster_member(&self, id: i64, cx: &App) -> Option<Member> {
        if !self.session_summaries.contains_key(&id) && !self.sessions.contains_key(&id) {
            return None;
        }
        let metadata = self.session_metadata(id, cx);
        let summary = metadata.summary;
        let status = match self.sessions.get(&id).map(|view| view.read(cx)) {
            Some(live) => live_status(
                self.held_mark(id, Some(live), cx),
                live.responding(),
                live.needs_approval(),
            ),
            None => live_status(self.held_mark(id, None, cx), metadata.busy, false),
        };
        let (cwd, in_worktree) = self.session_cwd(id, cx)?;
        let facts = self.session_facts.get(&id).cloned().unwrap_or_default();
        let workspace = match (in_worktree, facts.branch) {
            (true, Some(branch)) => Workspace::Worktree { branch },
            (true, None) => Workspace::Worktree {
                branch: Path::new(&cwd)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            },
            (false, _) => Workspace::Main,
        };
        Some(Member {
            id,
            title: summary.title.clone(),
            harness: summary.provider.clone(),
            status,
            workspace,
            goal: facts.goal,
            files: facts.files,
            repo: facts.repo,
            remote: summary.cwd.starts_with("remote://"),
            archived: summary.archived,
            cwd,
            last_active_ms: summary.updated_at,
        })
    }
}

/// Reads each session's transcript and asks git about its folder.
fn collect_facts(store: &crate::runtime::Database, sources: Vec<Source>) -> Vec<(i64, Facts)> {
    sources
        .into_iter()
        .map(|source| {
            let rows = locked(store)
                .and_then(|store| store.load(source.id))
                .map(|transcript| transcript.rows)
                .unwrap_or_default();
            let goal = gespan_core::response::request(&rows)
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            let cwd = Path::new(&source.cwd);
            let edited = gespan_core::response::edited_files(&rows);
            let files = repo::checkout_root(cwd)
                .map(|root| repo::session_files(&edited, &root))
                .unwrap_or_default();
            let branch = source
                .in_worktree
                .then(|| repo::current_branch(cwd))
                .flatten();
            let facts = Facts {
                goal,
                files,
                repo: repo::repo_key(cwd),
                branch,
            };
            (source.id, facts)
        })
        .collect()
}
