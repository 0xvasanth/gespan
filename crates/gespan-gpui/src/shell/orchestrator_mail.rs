//! Messages between sessions: `sessions_message`, `sessions_wait` and the
//! reports started sessions send their parents.
//!
//! Mail is written to the store before anything else happens, and delivered
//! as one turn the next time its receiver is idle: not working, not held, not
//! waiting on the user, with nothing typed or queued. Nothing interrupts a
//! running turn. A receiver whose chat isn't open is loaded off-screen; an
//! archived one keeps its mail until it is unarchived. Mail passed on
//! [`mailbox::MAX_HOPS`] times, or past [`mailbox::MAX_AGENT_TURNS_PER_HOUR`],
//! is held for the user.
use super::{Shell, orchestrator_tree::effective_access};
use crate::runtime::locked;
use gespan_core::orchestrator::{
    bridge::UNAVAILABLE,
    mailbox::{self, HOUR_MS, Hold, Kind, Mail, State},
    policy::{Code, Refusal},
    tree::Outcome,
};
use gpui::{App, Context};
use serde_json::{Value, json};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

type Reply = mpsc::Sender<Result<Value, String>>;

/// How much of a child's last reply its report quotes.
const REPORT_REPLY: usize = 2_000;

/// One `sessions_wait` call.
pub(super) struct Waiter {
    ids: Vec<i64>,
    reply: Reply,
    deadline: Instant,
}

fn now_ms() -> u64 {
    gespan_core::time::now_ms()
}

fn fresh_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("auto-{}-{}", now_ms(), NEXT.fetch_add(1, Ordering::Relaxed))
}

fn this_view(shell: &Shell, id: i64) -> Option<gpui::Entity<crate::conversation::Conversation>> {
    shell.sessions.get(&id).cloned()
}

impl Shell {
    fn hops_of(&self, id: i64) -> u8 {
        self.turn_hops.get(&id).copied().unwrap_or(0)
    }

    /// A turn the user typed starts the hop count again.
    pub(super) fn orchestrator_typed(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(view) = self.sessions.get(&id) else {
            return;
        };
        let typed = view.read(cx).typed_turns;
        if self.seen_typed.insert(id, typed).unwrap_or(0) != typed {
            self.turn_hops.remove(&id);
            // The user picked it up again: its mail flows once more.
            self.cancelled.remove(&id);
        }
    }

    /// A started session's report to its parent, one hop past the turn it
    /// reports on.
    pub(super) fn queue_report(
        &mut self,
        parent: i64,
        child: i64,
        outcome: Outcome,
        turn: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.sessions.get(&child) else {
            return;
        };
        let view = view.read(cx);
        let reply = match outcome {
            Outcome::WaitingOnUser => view
                .pending_input(cx)
                .map(|input| input.label)
                .unwrap_or_default(),
            _ => view.last_reply(cx),
        };
        let reply: String = reply.trim().chars().take(REPORT_REPLY).collect();
        let mut text = format!("It {}.", outcome.describe());
        if !reply.is_empty() {
            text.push_str("\n\n");
            text.push_str(&reply);
        }
        // Unique across runs of the app: turn counts start again at each
        // start, and mail an earlier run delivered keeps its id.
        let id = match outcome {
            Outcome::WaitingOnUser => format!("report:{child}:{turn}:waiting:{}", fresh_id()),
            _ => format!("report:{child}:{turn}:{}", fresh_id()),
        };
        let mail = Mail {
            id,
            to: parent,
            from: Some(child),
            from_title: view.summary.title.clone(),
            kind: Kind::Report,
            text,
            hops: self.hops_of(child).saturating_add(1),
            created_ms: now_ms(),
            state: State::Queued,
            held: None,
            released: false,
            sender_access: None,
        };
        self.enqueue(mail, None, cx);
    }

    /// Saves `mail`, held when it has gone too many hops, answers the caller
    /// and tries to deliver.
    fn enqueue(&mut self, mut mail: Mail, reply: Option<Reply>, cx: &mut Context<Self>) {
        if mail.held.is_none()
            && let Err(why) = mailbox::admit(mail.hops)
        {
            mail.state = State::Held;
            mail.held = Some(why);
        }
        let (to, id, held) = (mail.to, mail.id.clone(), mail.held);
        let store = self.store.clone();
        let work = cx
            .background_executor()
            .spawn(async move { locked(&store)?.enqueue_mail(&mail) });
        cx.spawn(async move |this, cx| {
            let saved = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(true) = saved
                    && let Some(why) = held
                {
                    *this.held_mail.entry(to).or_default() += 1;
                    this.notice_held(to, why, cx);
                }
                if let Some(reply) = reply {
                    let state = if held.is_some() { "held" } else { "queued" };
                    let _ = reply.send(saved.map(|_| json!({"id": id, "to": to, "state": state})));
                }
                this.deliver_mail(to, cx);
            });
        })
        .detach();
    }

    /// Tells the user mail for `to` is held, and how to let it through.
    fn notice_held(&mut self, to: i64, why: Hold, cx: &mut Context<Self>) {
        let Some(summary) = self.session_summaries.get(&to).cloned() else {
            return;
        };
        let body = match why {
            Hold::Hops => format!(
                "Messages to this session were held: sessions passed them back and forth {} times. Deliver them from its sidebar menu.",
                mailbox::MAX_HOPS
            ),
            Hold::Access => "A message to this session was held: it came from a session with narrower permissions, so it waits for you. Deliver it from its sidebar menu.".to_owned(),
            Hold::Rate => format!(
                "Messages to this session were held: other sessions started {} turns here in the last hour. Deliver them from its sidebar menu.",
                mailbox::MAX_AGENT_TURNS_PER_HOUR
            ),
        };
        self.deliver_session_notice(
            &summary,
            "agentInput",
            &body,
            Some(crate::sound::Cue::Input),
            cx,
        );
    }

    /// Delivers `target`'s queued mail as one turn, if `target` is idle.
    pub(super) fn deliver_mail(&mut self, target: i64, cx: &mut Context<Self>) {
        if self.mail_flushing.contains(&target)
            || self.cancelled.contains(&target)
            || self
                .session_summaries
                .get(&target)
                .is_some_and(|summary| summary.archived)
        {
            return;
        }
        if !self.mail_idle(target, cx) {
            return;
        }
        self.mail_flushing.insert(target);
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let store = locked(&store)?;
            let queued = store.mail_for(target, State::Queued)?;
            let turns = store.agent_turns_since(target, now_ms().saturating_sub(HOUR_MS))?;
            let refs: Vec<&Mail> = queued.iter().collect();
            let (now, wait) = mailbox::deliverable(&refs, turns);
            let wait: Vec<String> = wait.iter().map(|mail| mail.id.clone()).collect();
            if !wait.is_empty() {
                store.hold_mail(&wait, Hold::Rate)?;
            }
            let now: Vec<Mail> = now.into_iter().cloned().collect();
            // Claimed for this turn now, so no other pass can send it too;
            // whatever doesn't go out is put back below.
            let ids: Vec<String> = now.iter().map(|mail| mail.id.clone()).collect();
            store.mark_mail_delivered(&ids, now_ms())?;
            Ok::<_, String>((now, wait.len()))
        });
        cx.spawn(async move |this, cx| {
            let found = work.await;
            let _ = this.update(cx, |this, cx| {
                this.mail_flushing.remove(&target);
                let Ok((now, held)) = found else {
                    return;
                };
                if held > 0 {
                    *this.held_mail.entry(target).or_default() += held;
                    this.notice_held(target, Hold::Rate, cx);
                }
                if now.is_empty() {
                    return;
                }
                this.send_claimed(target, now, cx);
            });
        })
        .detach();
    }

    /// Sends claimed mail as `target`'s turn if it can still take one and
    /// each piece may still pass; puts the rest back in the queue (or holds
    /// what may no longer pass).
    fn send_claimed(&mut self, target: i64, claimed: Vec<Mail>, cx: &mut Context<Self>) {
        let requeue = |this: &Self, mails: &[Mail], cx: &mut Context<Self>| {
            let ids: Vec<String> = mails.iter().map(|mail| mail.id.clone()).collect();
            let store = this.store.clone();
            cx.background_executor()
                .spawn(async move {
                    if let Ok(store) = locked(&store) {
                        let _ = store.requeue_mail(&ids);
                    }
                })
                .detach();
        };
        // The receiver may have started a turn meanwhile, or not have read
        // its settings yet: the mail waits for its next idle point.
        let view = match this_view(self, target) {
            Some(view) if self.mail_idle(target, cx) && view.read(cx).settings_known() => view,
            _ => {
                requeue(self, &claimed, cx);
                return;
            }
        };
        // The receiver's mode may have widened since a message was queued:
        // check again, and hold what may no longer pass.
        let receiver = Some(view.read(cx).settings.access);
        let (now, held): (Vec<Mail>, Vec<Mail>) = claimed.into_iter().partition(|mail| {
            mail.kind == Kind::Report
                || mail.released
                || mailbox::admit_known_access(mail.sender_access, receiver).is_ok()
        });
        if !held.is_empty() {
            let ids: Vec<String> = held.iter().map(|mail| mail.id.clone()).collect();
            *self.held_mail.entry(target).or_default() += ids.len();
            self.notice_held(target, Hold::Access, cx);
            let store = self.store.clone();
            cx.background_executor()
                .spawn(async move {
                    if let Ok(store) = locked(&store) {
                        let _ = store.hold_mail(&ids, Hold::Access);
                    }
                })
                .detach();
        }
        if now.is_empty() {
            return;
        }
        let refs: Vec<&Mail> = now.iter().collect();
        let text = mailbox::turn_text(&refs);
        let hops = now.iter().map(|mail| mail.hops).max().unwrap_or(0);
        self.turn_hops.insert(target, hops);
        view.update(cx, |view, cx| view.send_worker_turn(text, cx));
    }

    /// Whether `id` can take a delivered turn now. Loads a closed chat
    /// off-screen first.
    fn mail_idle(&mut self, id: i64, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.ensure_session(id, cx) else {
            return false;
        };
        let live = view.read(cx);
        live.settings_known()
            && !live.summary.archived
            && !live.needs_approval()
            && !self.session_held(id, Some(live), cx)
            && live.available_for_managed_run(cx)
    }

    /// At start: mail queued before the app last closed is delivered, and
    /// held mail is counted for the sidebar.
    pub(super) fn resume_mail(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let store = locked(&store)?;
            Ok::<_, String>((store.queued_targets()?, store.held_counts()?))
        });
        cx.spawn(async move |this, cx| {
            let Ok((targets, held)) = work.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.held_mail.extend(held);
                for target in targets {
                    this.deliver_mail(target, cx);
                }
            });
        })
        .detach();
    }

    /// The user's "Deliver held messages": they go in the next idle turn,
    /// past the limits.
    pub(super) fn release_held_mail(&mut self, target: i64, cx: &mut Context<Self>) {
        self.held_mail.remove(&target);
        let store = self.store.clone();
        let work = cx
            .background_executor()
            .spawn(async move { locked(&store)?.release_mail(target) });
        cx.spawn(async move |this, cx| {
            if work.await.is_ok() {
                let _ = this.update(cx, |this, cx| this.deliver_mail(target, cx));
            }
        })
        .detach();
    }

    /// Whether `target` is one `caller` may message or wait on: a local
    /// session in the same project, other than itself.
    fn mail_scope(&self, caller: i64, target: i64, cx: &App) -> Result<(), Refusal> {
        if caller == target {
            return Err(Refusal::new(
                Code::SelfTarget,
                "a session can't message or wait on itself",
            ));
        }
        if !self.session_summaries.contains_key(&target) && !self.sessions.contains_key(&target) {
            return Err(Refusal::new(
                Code::NotFound,
                format!("there is no session {target}"),
            ));
        }
        let project = |id| self.session_metadata(id, cx).summary.cwd.clone();
        let target_project = project(target);
        if target_project != project(caller) || target_project.starts_with("remote://") {
            return Err(Refusal::new(
                Code::OutOfScope,
                format!("session {target} works in another project"),
            ));
        }
        Ok(())
    }

    pub(super) fn answer_message(
        &mut self,
        caller: i64,
        target: i64,
        text: String,
        request: Option<String>,
        reply: Reply,
        cx: &mut Context<Self>,
    ) {
        if target < 0 {
            match self.remote_peer_for(caller, target, cx) {
                Ok(peer) => self.answer_remote_message(caller, peer, text, reply, cx),
                Err(refusal) => {
                    let _ = reply.send(Err(refusal.into()));
                }
            }
            return;
        }
        if let Err(refusal) = self.mail_scope(caller, target, cx) {
            let _ = reply.send(Err(refusal.into()));
            return;
        }
        // A session can't get one that may do more to act for it: the user
        // decides.
        let sender = self
            .sessions
            .get(&caller)
            .map(|view| effective_access(view.read(cx)));
        // Until the receiver's settings are read its mode is a default; the
        // check then waits for delivery, which waits for the real mode.
        let receiver = self.ensure_session(target, cx).map(|view| {
            let view = view.read(cx);
            (view.settings_known(), view.settings.access)
        });
        let held = match receiver {
            Some((false, _)) => None,
            Some((true, mode)) => mailbox::admit_known_access(sender, Some(mode)).err(),
            None => mailbox::admit_known_access(sender, None).err(),
        };
        let mail = Mail {
            id: format!("msg:{caller}:{}", request.unwrap_or_else(fresh_id)),
            to: target,
            from: Some(caller),
            from_title: self.session_metadata_title(caller, cx),
            kind: Kind::Message,
            text,
            hops: self.hops_of(caller).saturating_add(1),
            created_ms: now_ms(),
            state: if held.is_some() {
                State::Held
            } else {
                State::Queued
            },
            held,
            released: false,
            sender_access: sender,
        };
        self.enqueue(mail, Some(reply), cx);
    }

    pub(super) fn answer_wait(
        &mut self,
        caller: i64,
        ids: Vec<i64>,
        timeout: Duration,
        reply: Reply,
        cx: &mut Context<Self>,
    ) {
        if let [id] = ids[..]
            && id < 0
        {
            match self.remote_peer_for(caller, id, cx) {
                Ok(peer) => self.answer_remote_wait(peer, id, timeout, reply),
                Err(refusal) => {
                    let _ = reply.send(Err(refusal.into()));
                }
            }
            return;
        }
        if ids.iter().any(|id| *id < 0) {
            let _ = reply.send(Err(Refusal::invalid(
                "wait on a remote agent on its own, not with other sessions",
            )
            .into()));
            return;
        }
        if let Some(refusal) = ids
            .iter()
            .find_map(|id| self.mail_scope(caller, *id, cx).err())
        {
            let _ = reply.send(Err(refusal.into()));
            return;
        }
        self.mail_waiters.push(Waiter {
            ids,
            reply,
            deadline: Instant::now() + timeout,
        });
        self.resolve_waits(cx);
        // Wall-clock time, on a thread of its own: the wait is bounded by the
        // agent's clock, not by how busy the app is.
        let (done, expired) = async_channel::bounded::<()>(1);
        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            let _ = done.send_blocking(());
        });
        cx.spawn(async move |this, cx| {
            let _ = expired.recv().await;
            let _ = this.update(cx, |this, cx| this.resolve_waits(cx));
        })
        .detach();
    }

    /// What a waited-on session is doing, once it is no longer working.
    fn wait_status(&self, id: i64, cx: &App) -> Option<&'static str> {
        if !self.session_summaries.contains_key(&id) && !self.sessions.contains_key(&id) {
            return Some("deleted");
        }
        let metadata = self.session_metadata(id, cx);
        if metadata.summary.archived {
            return Some("archived");
        }
        if metadata.needs_approval() {
            return Some("waiting-on-user");
        }
        (!metadata.busy && !self.launching.contains(&id)).then_some("idle")
    }

    /// Answers every wait whose sessions have stopped or that ran out of time.
    pub(super) fn resolve_waits(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let waiters = std::mem::take(&mut self.mail_waiters);
        for waiter in waiters {
            let ready: Vec<Value> = waiter
                .ids
                .iter()
                .filter_map(|id| {
                    self.wait_status(*id, cx)
                        .map(|status| json!({"id": id, "status": status}))
                })
                .collect();
            if !ready.is_empty() {
                let _ = waiter
                    .reply
                    .send(Ok(json!({"ready": ready, "timedOut": false})));
            } else if now >= waiter.deadline {
                let _ = waiter
                    .reply
                    .send(Ok(json!({"ready": [], "timedOut": true})));
            } else {
                self.mail_waiters.push(waiter);
            }
        }
    }
}

/// When the window closes with calls still waiting, they hear so.
impl Drop for Waiter {
    fn drop(&mut self) {
        let _ = self.reply.send(Err(UNAVAILABLE.into()));
    }
}
