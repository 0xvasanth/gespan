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
    roster::{self, Member, Roster, Status, Workspace},
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
    /// Whether the transcript and git status were read. A session in a
    /// repository no open chat works in only has its repository looked up.
    pub full: bool,
    /// Its folder no longer exists, as when its worktree was removed.
    pub gone: bool,
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
        // Tests run it too: the end-to-end tests call these tools for real.
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

    /// Lets `id`'s agent reach this window through its `sessions` tools,
    /// and learns what its roster needs.
    pub(super) fn serve_sessions(&mut self, id: i64, cx: &mut Context<Self>) {
        if let Some(bridge) = &self.orchestrator_bridge {
            bridge.serve(id);
        }
        self.learn_facts(vec![id], cx);
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
                let mut ids = self.missing_facts(cx);
                if !ids.contains(&session) {
                    ids.push(session);
                }
                let sources: Vec<Source> = ids
                    .into_iter()
                    .filter_map(|id| self.facts_source(id, cx))
                    .collect();
                let open = self.open_cwds(cx);
                let store = self.store.clone();
                let work = cx
                    .background_executor()
                    .spawn(async move { collect_facts(&store, sources, &open) });
                cx.spawn(async move |this, cx| {
                    let found = work.await;
                    let answer = this.update(cx, |this, cx| {
                        this.session_facts.extend(found);
                        this.push_rosters(cx);
                        this.list_answer(session, scope, cx)
                    });
                    let _ = reply.send(answer.unwrap_or_else(|_| {
                        Err(gespan_core::orchestrator::bridge::UNAVAILABLE.into())
                    }));
                })
                .detach();
            }
            Action::Start { input } => self.answer_start(session, *input, reply, cx),
            Action::Cancel { id } => self.answer_cancel(session, id, reply, cx),
            Action::Message {
                id,
                text,
                request_id,
            } => self.answer_message(session, id, text, request_id, reply, cx),
            Action::Wait {
                ids,
                timeout_seconds,
            } => self.answer_wait(
                session,
                ids,
                std::time::Duration::from_secs(timeout_seconds),
                reply,
                cx,
            ),
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
        self.learn_facts(vec![id], cx);
    }

    /// Reads these sessions' facts in one background task, then hands every
    /// open chat its roster once. Sessions already being read are skipped.
    fn learn_facts(&mut self, ids: Vec<i64>, cx: &mut Context<Self>) {
        let sources: Vec<Source> = ids
            .into_iter()
            .filter(|id| !self.facts_pending.contains(id))
            .filter_map(|id| self.facts_source(id, cx))
            .collect();
        if sources.is_empty() {
            return;
        }
        let batch: Vec<i64> = sources.iter().map(|s| s.id).collect();
        self.facts_pending.extend(batch.iter().copied());
        let open = self.open_cwds(cx);
        let store = self.store.clone();
        let work = cx
            .background_executor()
            .spawn(async move { collect_facts(&store, sources, &open) });
        cx.spawn(async move |this, cx| {
            let found = work.await;
            let _ = this.update(cx, |this, cx| {
                for id in &batch {
                    this.facts_pending.remove(id);
                }
                this.session_facts.extend(found);
                this.push_rosters(cx);
            });
        })
        .detach();
    }

    /// Sessions a roster could name whose facts are unknown, or only partly
    /// known though their repository now has an open chat.
    fn missing_facts(&self, cx: &App) -> Vec<i64> {
        let open: Vec<&PathBuf> = self
            .sessions
            .keys()
            .filter_map(|id| self.session_facts.get(id)?.repo.as_ref())
            .collect();
        self.roster_candidates(cx)
            .into_iter()
            .filter(|id| !self.untouched(*id, cx) || self.sessions.contains_key(id))
            .filter(|id| match self.session_facts.get(id) {
                None => true,
                Some(facts) => {
                    !facts.full && facts.repo.as_ref().is_some_and(|repo| open.contains(&repo))
                }
            })
            .collect()
    }

    /// The folders open chats work in: their repositories get full facts.
    fn open_cwds(&self, cx: &App) -> Vec<String> {
        self.sessions
            .keys()
            .filter_map(|id| Some(self.session_cwd(*id, cx)?.0))
            .filter(|cwd| !cwd.starts_with("remote://"))
            .collect()
    }

    /// `id`'s siblings from what is already known, for the start of a turn.
    fn roster_for(&self, id: i64, cx: &App) -> Option<Roster> {
        let caller = self.roster_member(id, cx)?;
        let others: Vec<Member> = self
            .order
            .iter()
            .filter(|other| !self.untouched(**other, cx))
            .filter_map(|other| self.roster_member(*other, cx))
            .collect();
        Some(roster::build(&caller, &others))
    }

    /// Hands each open chat the roster its next turn would carry, so the
    /// send path reads it without touching git or the store. Sessions not
    /// yet known are learned in one batch, which hands the rosters out again.
    pub(super) fn push_rosters(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<i64> = self.sessions.keys().copied().collect();
        for id in ids {
            let text = self.roster_for(id, cx).and_then(|r| roster::render(&r));
            if let Some(view) = self.sessions.get(&id) {
                view.update(cx, |view, _| view.set_roster(text));
            }
        }
        let missing = self.missing_facts(cx);
        if !missing.is_empty() {
            self.learn_facts(missing, cx);
        }
    }

    fn list_answer(&self, session: i64, scope: Scope, cx: &App) -> Result<Value, String> {
        let caller = self
            .roster_member(session, cx)
            .ok_or(gespan_core::orchestrator::bridge::UNAVAILABLE)?;
        let members: Vec<Member> = self
            .order
            .iter()
            .filter(|id| !self.untouched(**id, cx))
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

    /// A new chat nothing was ever sent from: no one's sibling yet.
    fn untouched(&self, id: i64, cx: &App) -> bool {
        if !self.session_summaries.contains_key(&id) && !self.sessions.contains_key(&id) {
            return true;
        }
        let metadata = self.session_metadata(id, cx);
        !metadata.summary.has_messages && !metadata.busy
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
            // Its worktree is gone: it can't run, and isn't in the main checkout.
            Some(live) if live.settings.worktree_removed => return None,
            Some(live) => live.settings.worktree_cwd.clone(),
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
        if facts.gone {
            return None;
        }
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
            parent: self
                .session_tree
                .parent(id)
                .map(|parent| self.session_metadata_title(parent, cx)),
        })
    }

    /// A session's title, whether or not its chat is loaded.
    pub(super) fn session_metadata_title(&self, id: i64, cx: &App) -> String {
        if let Some(view) = self.sessions.get(&id) {
            return view.read(cx).summary.title.clone();
        }
        self.session_summaries
            .get(&id)
            .map(|summary| summary.title.clone())
            .unwrap_or_else(|| format!("session {id}"))
    }
}

/// Asks git which repository each session's folder belongs to, once per
/// folder. Only sessions in a repository one of the `open` folders shares
/// get their transcript read and their checkout's status asked.
fn collect_facts(
    store: &crate::runtime::Database,
    sources: Vec<Source>,
    open: &[String],
) -> Vec<(i64, Facts)> {
    let mut repos: std::collections::HashMap<String, Option<PathBuf>> = Default::default();
    let mut repo_of = |cwd: &str| {
        repos
            .entry(cwd.to_owned())
            .or_insert_with(|| repo::repo_key(Path::new(cwd)))
            .clone()
    };
    let relevant: Vec<PathBuf> = open.iter().filter_map(|cwd| repo_of(cwd)).collect();
    sources
        .into_iter()
        .map(|source| {
            if !Path::new(&source.cwd).is_dir() {
                let facts = Facts {
                    gone: true,
                    ..Default::default()
                };
                return (source.id, facts);
            }
            let repo = repo_of(&source.cwd);
            if !repo.as_ref().is_some_and(|repo| relevant.contains(repo)) {
                let facts = Facts {
                    repo,
                    ..Default::default()
                };
                return (source.id, facts);
            }
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
                repo,
                branch,
                full: true,
                gone: false,
            };
            (source.id, facts)
        })
        .collect()
}
