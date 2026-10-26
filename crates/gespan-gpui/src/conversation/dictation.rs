//! Port of useDictation.ts, insertDictation.ts and DictationBar.tsx: the
//! composer's mic, its shortcut and the caption row above the field.
//!
//! The speech helper is app-wide. [`Speech`] owns its one client, the shared
//! model store and the downloads; every composer has its own [`Dictation`],
//! and the helper's events go to whichever composer last started. Requests
//! are only sent from the foreground thread, through the client's single
//! queue, so a Cancel always reaches the helper before a later Start.
use crate::{input::Input, theme::*};
use gespan_models::{
    CancelToken, DownloadError, ModelSpec, ModelStatus, ModelStore, Progress, default_root,
};
use gespan_speech::{
    CRASHED, SpeechClient, UNAVAILABLE, VocabularySources, catalog, helper_command, local_branches,
    pick_terms,
    protocol::{ErrorKind, Event, Request},
};
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EntityId, Focusable, Global, MouseButton, Role,
    SharedString, Subscription, Task, WeakEntity, div, prelude::*, px, relative,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const ALREADY_DOWNLOADING: &str = "The speech model is already downloading.";
pub(crate) const DOWNLOAD_PROMPT: &str = "Download the speech model (about 660 MB)?";
pub(crate) const DICTATE_COMMAND: &str = "Composer: Dictate";
/// A shortcut press at least this long is push-to-talk: releasing stops.
pub(crate) const HOLD_TO_TALK: Duration = Duration::from_millis(400);
/// Download progress is reported at most this often, except at 100%.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// The model a saved choice names; anything unknown is the English default.
pub(crate) fn model_spec(id: &str) -> &'static ModelSpec {
    catalog::ALL
        .iter()
        .copied()
        .find(|spec| spec.id == id)
        .unwrap_or(&catalog::PARAKEET_V2_EN)
}

/// One word or phrase per line; blank lines dropped.
pub(crate) fn parse_words(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// insertDictation's spacing: the trimmed text, with one space in front when
/// the character before the caret is not whitespace and the text does not
/// attach to the word before it. `None` when nothing was said.
pub(crate) fn spaced(before: &str, text: &str) -> Option<String> {
    let spoken = text.trim();
    if spoken.is_empty() {
        return None;
    }
    let attaches = spoken.starts_with(['.', ',', ';', ':', '!', '?', ')']);
    let after_word = before
        .chars()
        .next_back()
        .is_some_and(|c| !c.is_whitespace());
    Some(if after_word && !attaches {
        format!(" {spoken}")
    } else {
        spoken.to_owned()
    })
}

fn saved_choice(cx: &App) -> (&'static ModelSpec, Vec<String>) {
    match cx.try_global::<crate::settings::Preferences>() {
        Some(p) => (
            model_spec(&p.dictation_model),
            parse_words(&p.dictation_words),
        ),
        None => (&catalog::PARAKEET_V2_EN, Vec::new()),
    }
}

enum Transport {
    /// The helper process, spawned on the first start.
    Helper(Option<SpeechClient>),
    #[cfg(test)]
    Sink {
        send: std::rc::Rc<dyn Fn(Request)>,
        unavailable: bool,
    },
}

/// The app-wide speech helper, model store and downloads.
pub(crate) struct Speech {
    transport: Transport,
    store: Arc<ModelStore>,
    /// Model id last sent in a `Load`.
    loaded: Option<String>,
    /// The composer the helper's events belong to.
    owner: Option<WeakEntity<Dictation>>,
    /// Every composer's dictation, so leaving a session can reset its prompt.
    members: Vec<WeakEntity<Dictation>>,
    /// Cancels sent that the helper has not answered yet. Until it has, an
    /// error before `Listening` belongs to the abandoned take.
    unanswered_cancels: std::cell::Cell<usize>,
    events: async_channel::Sender<Event>,
    downloads: HashMap<&'static str, (CancelToken, Progress)>,
    /// Each catalog model's status, once read.
    models: Option<Vec<(&'static ModelSpec, ModelStatus)>>,
    _models: Option<Task<()>>,
    _drain: Task<()>,
}

struct SpeechGlobal(Entity<Speech>);
impl Global for SpeechGlobal {}

/// The app's speech hub, made on first use. The helper itself starts on the
/// first dictation and is shut down when the app quits.
pub(crate) fn speech(cx: &mut App) -> Entity<Speech> {
    if let Some(hub) = cx.try_global::<SpeechGlobal>() {
        return hub.0.clone();
    }
    let hub =
        cx.new(|cx| Speech::new(Transport::Helper(None), ModelStore::new(default_root()), cx));
    let weak = hub.downgrade();
    cx.on_app_quit(move |cx| {
        let client = weak.update(cx, |s, _| s.take_client()).ok().flatten();
        let done = client.map(|c| cx.background_executor().spawn(async move { c.shutdown() }));
        async move {
            if let Some(done) = done {
                done.await;
            }
        }
    })
    .detach();
    cx.set_global(SpeechGlobal(hub.clone()));
    hub
}

/// The client's own messages for a helper that died or was given up on.
fn supervisor_error(message: &str) -> bool {
    message == UNAVAILABLE || message == CRASHED
}

/// Leaving for another session: dictation running anywhere but `keep` is
/// cancelled and inserts nothing, and a download prompt left behind closes,
/// so its download finishing later does not open the mic there.
pub(crate) fn switched_to(keep: &Entity<Dictation>, cx: &mut App) {
    let Some(hub) = cx.try_global::<SpeechGlobal>().map(|g| g.0.clone()) else {
        return;
    };
    let others: Vec<_> = hub.update(cx, |s, _| {
        s.members.retain(|m| m.upgrade().is_some());
        s.members
            .iter()
            .filter(|m| m.entity_id() != keep.entity_id())
            .cloned()
            .collect()
    });
    for other in others {
        let _ = other.update(cx, |d, cx| d.leave(cx));
    }
}

impl Speech {
    fn new(transport: Transport, store: ModelStore, cx: &mut Context<Self>) -> Self {
        let (events, rx) = async_channel::unbounded::<Event>();
        // Supervisor-thread events are handled here, on the foreground.
        let drain = cx.spawn(async move |this, cx| {
            while let Ok(event) = rx.recv().await {
                let Ok(owner) = this.read_with(cx, |s, _| {
                    s.answered(&event);
                    s.owner.clone()
                }) else {
                    break;
                };
                if let Some(owner) = owner {
                    let _ = owner.update(cx, |d, cx| d.on_event(event, cx));
                }
            }
        });
        Self {
            transport,
            store: Arc::new(store),
            loaded: None,
            owner: None,
            members: Vec::new(),
            unanswered_cancels: Default::default(),
            events,
            downloads: HashMap::new(),
            models: None,
            _models: None,
            _drain: drain,
        }
    }

    fn take_client(&mut self) -> Option<SpeechClient> {
        match &mut self.transport {
            Transport::Helper(client) => client.take(),
            #[cfg(test)]
            Transport::Sink { .. } => None,
        }
    }

    fn unavailable(&self) -> bool {
        match &self.transport {
            Transport::Helper(client) => client.as_ref().is_some_and(|c| c.is_unavailable()),
            #[cfg(test)]
            Transport::Sink { unavailable, .. } => *unavailable,
        }
    }

    /// Makes `me` the owner and returns the composer it replaces.
    fn claim(&mut self, me: WeakEntity<Dictation>) -> Option<WeakEntity<Dictation>> {
        let previous = self.owner.replace(me.clone());
        previous.filter(|p| p.entity_id() != me.entity_id())
    }

    fn release(&mut self, me: EntityId) {
        if self.owner.as_ref().is_some_and(|o| o.entity_id() == me) {
            self.owner = None;
        }
    }

    /// Spawns the helper if needed, loads the model when it changed, then
    /// starts listening.
    fn start(&mut self, spec: &'static ModelSpec, terms: Vec<String>) -> Result<(), String> {
        if self.unavailable() {
            return Err(UNAVAILABLE.into());
        }
        if let Transport::Helper(client @ None) = &mut self.transport {
            let (program, args) = helper_command().map_err(|e| e.to_string())?;
            let events = self.events.clone();
            *client = Some(SpeechClient::spawn(program, args, move |event| {
                let _ = events.try_send(event);
            }));
        }
        if self.loaded.as_deref() != Some(spec.id) {
            self.send(Request::Load {
                model_dir: self.store.dir(spec),
                model_id: spec.id.into(),
            });
            self.loaded = Some(spec.id.into());
        }
        self.send(Request::Start { terms });
        Ok(())
    }

    fn send(&self, request: Request) {
        if request == Request::Cancel {
            self.unanswered_cancels
                .set(self.unanswered_cancels.get() + 1);
        }
        match &self.transport {
            Transport::Helper(client) => {
                if let Some(client) = client {
                    client.send(request);
                }
            }
            #[cfg(test)]
            Transport::Sink { send, .. } => send(request),
        }
    }

    /// Keeps count of the helper's replies to Cancel. `Listening` means it has
    /// read everything sent before the latest Start, and the supervisor's own
    /// errors mean the process went away with whatever was unread.
    fn answered(&self, event: &Event) {
        let n = &self.unanswered_cancels;
        match event {
            Event::Cancelled => n.set(n.get().saturating_sub(1)),
            Event::Listening => n.set(0),
            Event::Error { message, .. } if supervisor_error(message) => n.set(0),
            _ => {}
        }
    }

    /// An error now may be the tail of a take that was cancelled.
    fn stale_errors(&self) -> bool {
        self.unanswered_cancels.get() > 0
    }

    /// The model is gone or failed to load, so the next start loads again.
    fn forget_loaded(&mut self) {
        self.loaded = None;
    }

    pub(crate) fn progress(&self, model_id: &str) -> Option<Progress> {
        self.downloads.get(model_id).map(|(_, p)| *p)
    }

    pub(crate) fn models(&self) -> Option<&[(&'static ModelSpec, ModelStatus)]> {
        self.models.as_deref()
    }

    /// Reads every model's status off the main thread.
    pub(crate) fn refresh_models(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            catalog::ALL
                .iter()
                .map(|&spec| (spec, store.status(spec)))
                .collect::<Vec<_>>()
        });
        self._models = Some(cx.spawn(async move |this, cx| {
            let models = work.await;
            let _ = this.update(cx, |this, cx| {
                this.models = Some(models);
                cx.notify();
            });
        }));
    }

    /// Starts downloading `spec`, or `None` when it already is.
    pub(crate) fn download(
        &mut self,
        spec: &'static ModelSpec,
        cx: &mut Context<Self>,
    ) -> Option<Task<Result<(), DownloadError>>> {
        if self.downloads.contains_key(spec.id) {
            return None;
        }
        let token = CancelToken::new();
        let start = Progress {
            downloaded: 0,
            total: 0,
        };
        self.downloads.insert(spec.id, (token.clone(), start));
        let (progress, updates) = async_channel::unbounded::<Progress>();
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let mut last: Option<Instant> = None;
            store.download(spec, &token, |p| {
                let now = Instant::now();
                if p.downloaded >= p.total
                    || last.is_none_or(|t| now.duration_since(t) >= PROGRESS_INTERVAL)
                {
                    last = Some(now);
                    let _ = progress.try_send(p);
                }
            })
        });
        cx.notify();
        Some(cx.spawn(async move |this, cx| {
            while let Ok(p) = updates.recv().await {
                let _ = this.update(cx, |this, cx| {
                    if let Some(entry) = this.downloads.get_mut(spec.id) {
                        entry.1 = p;
                    }
                    cx.notify();
                });
            }
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.downloads.remove(spec.id);
                this.refresh_models(cx);
                cx.notify();
            });
            result
        }))
    }

    pub(crate) fn cancel_download(&self, model_id: &str) {
        if let Some((token, _)) = self.downloads.get(model_id) {
            token.cancel();
        }
    }

    pub(crate) fn delete(
        &mut self,
        spec: &'static ModelSpec,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let store = self.store.clone();
        let work = cx
            .background_executor()
            .spawn(async move { store.delete(spec).map_err(|e| e.to_string()) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.loaded.as_deref() == Some(spec.id) {
                    this.loaded = None;
                }
                this.refresh_models(cx);
            });
            result
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DictationState {
    Idle,
    Starting,
    Listening,
    Finishing,
    NeedsModel,
    Error(SharedString),
}

enum StartError {
    ModelMissing,
}

/// One composer's dictation.
pub(crate) struct Dictation {
    state: DictationState,
    /// Live caption of what has been heard so far; never inserted.
    partial: SharedString,
    level: f32,
    error: Option<SharedString>,
    /// The model this composer's download prompt is fetching.
    downloading: Option<&'static str>,
    input: WeakEntity<Input>,
    project: Option<PathBuf>,
    known_names: Vec<String>,
    speech: Entity<Speech>,
    /// Bumped by every start and cancel; a stale start result is dropped.
    run: u64,
    /// This composer's attempt owns the helper's events.
    active: bool,
    /// The helper said `Listening` for this attempt. Anything else before it
    /// belongs to an earlier take, possibly another composer's.
    heard: bool,
    start_pending: bool,
    /// `Start` reached the helper, so leaving must send `Cancel`.
    sent: bool,
    stop_after_start: bool,
    press: Option<Instant>,
    /// The shortcut is down; key repeats are ignored until it is released.
    chord_held: bool,
    _start: Option<Task<()>>,
    _download: Option<Task<()>>,
    _speech: Subscription,
}

impl Dictation {
    pub(crate) fn new(input: WeakEntity<Input>, cx: &mut Context<Self>) -> Self {
        let speech = speech(cx);
        let me = cx.weak_entity();
        speech.update(cx, |s, _| s.members.push(me));
        let observer = cx.observe(&speech, |_, _, cx| cx.notify());
        let me = cx.entity_id();
        // Closing the composer mid-dictation inserts nothing anywhere.
        cx.on_release(move |this, cx| {
            if this.active {
                let sent = this.sent;
                this.speech.update(cx, |s, _| {
                    s.release(me);
                    if sent {
                        s.send(Request::Cancel);
                    }
                });
            }
        })
        .detach();
        Self {
            state: DictationState::Idle,
            partial: SharedString::default(),
            level: 0.,
            error: None,
            downloading: None,
            input,
            project: None,
            known_names: Vec::new(),
            speech,
            run: 0,
            active: false,
            heard: false,
            start_pending: false,
            sent: false,
            stop_after_start: false,
            press: None,
            chord_held: false,
            _start: None,
            _download: None,
            _speech: observer,
        }
    }

    /// The project vocabulary comes from, and the provider and model names to
    /// bias identifier joining towards.
    pub(crate) fn set_context(&mut self, project: Option<PathBuf>, known_names: Vec<String>) {
        self.project = project;
        self.known_names = known_names;
    }

    #[cfg(test)]
    pub(crate) fn state(&self) -> &DictationState {
        &self.state
    }

    #[cfg(test)]
    pub(crate) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn partial(&self) -> &SharedString {
        &self.partial
    }

    pub(crate) fn dictating(&self) -> bool {
        matches!(
            self.state,
            DictationState::Starting | DictationState::Listening | DictationState::Finishing
        )
    }

    /// Escape and the caption row's Cancel apply.
    pub(crate) fn cancellable(&self) -> bool {
        self.dictating() || self.state == DictationState::NeedsModel
    }

    fn disabled(&self, cx: &App) -> bool {
        self.input
            .upgrade()
            .is_none_or(|input| input.read(cx).read_only())
    }

    fn now(cx: &App) -> Instant {
        cx.background_executor().now()
    }

    fn set_state(&mut self, state: DictationState, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }

    /// The mic button.
    pub(crate) fn toggle(&mut self, cx: &mut Context<Self>) {
        match self.state {
            DictationState::Idle | DictationState::Error(_) => self.start(cx),
            DictationState::Starting | DictationState::Listening => self.stop(cx),
            _ => {}
        }
    }

    pub(crate) fn press_start(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.state,
            DictationState::Starting | DictationState::Listening
        ) {
            self.press = None;
            self.stop(cx);
            return;
        }
        if !matches!(self.state, DictationState::Idle | DictationState::Error(_)) {
            return;
        }
        self.press = Some(Self::now(cx));
        self.start(cx);
    }

    pub(crate) fn press_end(&mut self, cx: &mut Context<Self>) {
        if let Some(at) = self.press.take()
            && Self::now(cx).duration_since(at) >= HOLD_TO_TALK
        {
            self.stop(cx);
        }
    }

    /// The Dictate shortcut went down. Repeats while it is held are ignored.
    pub(crate) fn shortcut_down(&mut self, cx: &mut Context<Self>) {
        if self.chord_held {
            return;
        }
        self.chord_held = true;
        self.press_start(cx);
    }

    /// The shortcut's key or modifiers were released, or the field lost focus.
    pub(crate) fn shortcut_up(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.chord_held) {
            self.press_end(cx);
        }
    }

    pub(crate) fn chord_held(&self) -> bool {
        self.chord_held
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        // A disabled composer can stop and cancel, but not start.
        if self.disabled(cx) {
            return;
        }
        let me = cx.weak_entity();
        if let Some(previous) = self.speech.update(cx, |s, _| s.claim(me)) {
            let _ = previous.update(cx, |d, cx| d.cancel(cx));
        }
        self.run += 1;
        let run = self.run;
        self.active = true;
        self.heard = false;
        self.sent = false;
        self.start_pending = true;
        self.stop_after_start = false;
        self.error = None;
        self.partial = SharedString::default();
        self.level = 0.;
        self.set_state(DictationState::Starting, cx);

        let (spec, user_words) = saved_choice(cx);
        let store = self.speech.read(cx).store.clone();
        let project = self.project.clone();
        let known_names = self.known_names.clone();
        // Git and the project walk stay off the main thread.
        let work = cx.background_executor().spawn(async move {
            if store.status(spec) != ModelStatus::Ready {
                return Err(StartError::ModelMissing);
            }
            let branches = project.as_deref().map(local_branches).unwrap_or_default();
            Ok(pick_terms(&VocabularySources {
                project_root: project.as_deref(),
                branches: &branches,
                known_names: &known_names,
                user_words: &user_words,
            }))
        });
        self._start = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| this.started(run, spec, result, cx));
        }));
    }

    fn started(
        &mut self,
        run: u64,
        spec: &'static ModelSpec,
        result: Result<Vec<String>, StartError>,
        cx: &mut Context<Self>,
    ) {
        if run != self.run || !self.active {
            return;
        }
        self.start_pending = false;
        let terms = match result {
            Ok(terms) => terms,
            Err(StartError::ModelMissing) => {
                self.release(cx);
                self.set_state(DictationState::NeedsModel, cx);
                return;
            }
        };
        match self.speech.update(cx, |s, _| s.start(spec, terms)) {
            Ok(()) => {
                self.sent = true;
                if std::mem::take(&mut self.stop_after_start) {
                    self.speech.read(cx).send(Request::Stop);
                }
            }
            Err(message) => {
                self.release(cx);
                if message == UNAVAILABLE {
                    self.set_state(DictationState::Error(message.into()), cx);
                } else {
                    self.error = Some(message.into());
                    self.set_state(DictationState::Idle, cx);
                }
            }
        }
    }

    /// Stop listening and insert what was heard.
    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.active || self.state == DictationState::Finishing {
            return;
        }
        self.set_state(DictationState::Finishing, cx);
        if self.start_pending {
            self.stop_after_start = true;
        } else {
            self.speech.read(cx).send(Request::Stop);
        }
    }

    /// Drop the attempt without inserting anything.
    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        self.press = None;
        if self.active {
            self.run += 1;
            let sent = self.sent;
            self.release(cx);
            if sent {
                self.speech.read(cx).send(Request::Cancel);
            }
        }
        self.error = None;
        self.set_state(DictationState::Idle, cx);
    }

    /// Its session closed or another took over: anything running or prompting
    /// stops, and nothing is inserted.
    pub(crate) fn leave(&mut self, cx: &mut Context<Self>) {
        if self.cancellable() {
            self.cancel(cx);
        }
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        self.active = false;
        self.sent = false;
        self.start_pending = false;
        self.stop_after_start = false;
        self.partial = SharedString::default();
        self.level = 0.;
        let me = cx.entity_id();
        self.speech.update(cx, |s, _| s.release(me));
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        if !self.active {
            return;
        }
        match event {
            Event::Listening => {
                self.heard = true;
                if self.state == DictationState::Starting {
                    self.set_state(DictationState::Listening, cx);
                }
            }
            Event::Level { rms } if self.heard => {
                self.level = rms;
                cx.notify();
            }
            Event::Partial { text } if self.heard => {
                self.partial = text.into();
                cx.notify();
            }
            Event::Final { text, .. } if self.heard => {
                self.release(cx);
                self.insert(&text, cx);
                self.set_state(DictationState::Idle, cx);
            }
            Event::Error { message, .. }
                if !self.heard
                    && !supervisor_error(&message)
                    && self.speech.read(cx).stale_errors() => {}
            Event::Error { kind, message } => {
                self.release(cx);
                if matches!(kind, ErrorKind::ModelMissing | ErrorKind::ModelLoad) {
                    self.speech.update(cx, |s, _| s.forget_loaded());
                }
                if kind == ErrorKind::ModelMissing {
                    self.set_state(DictationState::NeedsModel, cx);
                } else {
                    self.error = Some(message.into());
                    self.set_state(DictationState::Idle, cx);
                }
            }
            // The helper answers every Cancel, and only this composer's own
            // (already handled) or an earlier attempt's can arrive here.
            _ => {}
        }
    }

    /// The final goes in at the caret or selection as they are now, so text
    /// typed meanwhile is kept.
    fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(input) = self.input.upgrade() else {
            return;
        };
        input.update(cx, |input, cx| {
            let before = &input.content[..input.selection().start];
            if let Some(text) = spaced(before, text) {
                // What was said still lands if the field turned read-only
                // meanwhile, as it would in the desktop app.
                let read_only = input.read_only();
                input.set_read_only(false);
                input.replace_text(None, &text, cx);
                input.set_read_only(read_only);
            }
        });
    }

    pub(crate) fn download(&mut self, cx: &mut Context<Self>) {
        if self.downloading.is_some() {
            return;
        }
        let (spec, _) = saved_choice(cx);
        self.error = None;
        let Some(task) = self.speech.update(cx, |s, cx| s.download(spec, cx)) else {
            self.error = Some(ALREADY_DOWNLOADING.into());
            cx.notify();
            return;
        };
        self.downloading = Some(spec.id);
        self._download = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| this.downloaded(result, cx));
        }));
        cx.notify();
    }

    fn downloaded(&mut self, result: Result<(), DownloadError>, cx: &mut Context<Self>) {
        self.downloading = None;
        match result {
            Ok(()) => {
                if self.state == DictationState::NeedsModel {
                    self.state = DictationState::Idle;
                    self.start(cx);
                }
            }
            // The user cancelled it; nothing to report.
            Err(DownloadError::Cancelled) => {}
            Err(error) => self.error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    pub(crate) fn cancel_download(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.downloading {
            self.speech.read(cx).cancel_download(id);
        }
    }

    fn download_progress(&self, cx: &App) -> Option<Progress> {
        self.downloading.map(|id| {
            self.speech.read(cx).progress(id).unwrap_or(Progress {
                downloaded: 0,
                total: 0,
            })
        })
    }
}

impl super::Conversation {
    /// Where dictation reads its vocabulary: the working copy (none for a
    /// remote session) and the provider and model names.
    pub(super) fn dictation_context(&self, cx: &mut Context<super::Conversation>) {
        let project = (!self.is_remote()).then(|| PathBuf::from(self.execution_cwd()));
        let mut names: Vec<String> = gespan_core::provider::Kind::ALL
            .iter()
            .map(|kind| kind.label().to_owned())
            .collect();
        names.push(
            self.model
                .clone()
                .unwrap_or_else(|| self.model_label().to_string()),
        );
        self.dictation
            .update(cx, |d, _| d.set_context(project, names));
    }

    /// The composer's dictation keys: the shortcut (tap toggles, hold talks),
    /// its release, and Escape to cancel. Shared by the session composer and
    /// the Pilot chat.
    pub(super) fn dictation_keys(
        &self,
        el: gpui::Stateful<gpui::Div>,
        cx: &mut Context<super::Conversation>,
    ) -> gpui::Stateful<gpui::Div> {
        el.on_action(cx.listener(|this, _: &crate::input::Dictate, _, cx| {
            this.dictation.update(cx, |d, cx| d.shortcut_down(cx));
        }))
        .on_key_up(cx.listener(|this, _: &gpui::KeyUpEvent, _, cx| {
            this.dictation.update(cx, |d, cx| d.shortcut_up(cx));
        }))
        .on_modifiers_changed(
            cx.listener(|this, event: &gpui::ModifiersChangedEvent, _, cx| {
                // macOS sends no key-up while Command is down, so
                // letting go of the modifiers ends a hold too.
                let held = event.modifiers.secondary() && event.modifiers.shift;
                if !held && this.dictation.read(cx).chord_held() {
                    this.dictation.update(cx, |d, cx| d.shortcut_up(cx));
                }
            }),
        )
        // Escape cancels dictation before the field or a menu sees
        // it, and still works while the composer is disabled. The
        // shell binds it to DismissMenu; unbound, it is a key.
        .capture_action(cx.listener(|this, _: &crate::shell::DismissMenu, _, cx| {
            if this.dictation.read(cx).cancellable() {
                this.dictation.update(cx, |d, cx| d.cancel(cx));
                cx.stop_propagation();
            }
        }))
        .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape" && this.dictation.read(cx).cancellable() {
                this.dictation.update(cx, |d, cx| d.cancel(cx));
                cx.stop_propagation();
            }
        }))
    }
}

fn percent(p: Progress) -> u64 {
    (p.downloaded * 100).checked_div(p.total).unwrap_or(0)
}

fn bar_button(
    id: &'static str,
    label: &'static str,
    text: &'static str,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(label)
        .flex_shrink_0()
        .rounded_md()
        .px_2()
        .py(px(2.))
        .text_size(px(12.))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(text)
}

fn primary(id: &'static str, label: &'static str, text: &'static str) -> gpui::Stateful<gpui::Div> {
    bar_button(id, label, text)
        .bg(content(0.1))
        .text_color(content(1.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .hover(|s| s.bg(content(0.15)))
}

fn quiet(id: &'static str, label: &'static str, text: &'static str) -> gpui::Stateful<gpui::Div> {
    bar_button(id, label, text)
        .text_color(content(0.5))
        .hover(|s| s.text_color(content(1.)))
}

fn red() -> gpui::Hsla {
    gpui::rgb(0xf87171).into()
}

/// DictationBar: the level meter and live caption, the download prompt, or
/// the last error. `None` while there is nothing to show.
pub(crate) fn bar(
    dictation: &Entity<Dictation>,
    input: &Entity<Input>,
    cx: &App,
) -> Option<AnyElement> {
    let d = dictation.read(cx);
    let act = |f: fn(&mut Dictation, &mut Context<Dictation>)| {
        let dictation = dictation.clone();
        let input = input.clone();
        move |_: &gpui::ClickEvent, window: &mut gpui::Window, cx: &mut App| {
            dictation.update(cx, f);
            window.focus(&input.focus_handle(cx), cx);
        }
    };
    let row = || {
        div()
            .debug_selector(|| "dictation-bar".into())
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .pt_2()
            .text_size(px(12.))
    };
    let error = d.error.clone().or(match &d.state {
        DictationState::Error(message) => Some(message.clone()),
        _ => None,
    });
    match &d.state {
        DictationState::NeedsModel => {
            let prompt = match d.download_progress(cx) {
                Some(progress) => {
                    let percent = percent(progress);
                    row()
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .text_color(content(0.7))
                                .child(format!("Downloading the speech model… {percent}%")),
                        )
                        .child(
                            div()
                                .h_1()
                                .w_24()
                                .flex_shrink_0()
                                .overflow_hidden()
                                .rounded_full()
                                .bg(content(0.1))
                                .child(
                                    div()
                                        .h_full()
                                        .rounded_full()
                                        .bg(accent_color())
                                        .w(relative(percent as f32 / 100.)),
                                ),
                        )
                        .child(
                            quiet("dictation-cancel-download", "Cancel", "Cancel")
                                .on_click(act(Dictation::cancel_download)),
                        )
                }
                None => row()
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .text_color(content(0.7))
                            .child(DOWNLOAD_PROMPT),
                    )
                    .child(
                        primary("dictation-download", "Download", "Download")
                            .on_click(act(Dictation::download)),
                    )
                    .child(
                        quiet("dictation-not-now", "Not now", "Not now")
                            .on_click(act(Dictation::cancel)),
                    ),
            };
            Some(
                div()
                    .flex()
                    .flex_col()
                    .child(prompt)
                    .when_some(error, |d, error| {
                        d.child(
                            div()
                                .id("dictation-alert")
                                .role(Role::Alert)
                                .px_3()
                                .pt_1()
                                .text_size(px(12.))
                                .text_color(red())
                                .child(error),
                        )
                    })
                    .into_any_element(),
            )
        }
        DictationState::Idle | DictationState::Error(_) => {
            let error = error?;
            Some(
                row()
                    .id("dictation-alert")
                    .items_start()
                    .role(Role::Alert)
                    .text_color(red())
                    .child(
                        div()
                            .debug_selector(|| "dictation-error".into())
                            .min_w_0()
                            .flex_1()
                            .child(error),
                    )
                    .child(
                        div()
                            .id("dictation-dismiss")
                            .debug_selector(|| "dictation-dismiss".into())
                            .role(Role::Button)
                            .aria_label("Dismiss")
                            .flex_shrink_0()
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(icon("x", 14.).text_color(content(0.4)))
                            .on_click(act(Dictation::cancel)),
                    )
                    .into_any_element(),
            )
        }
        state => {
            let status = match state {
                DictationState::Finishing => "Finishing…",
                DictationState::Starting => "Starting…",
                _ => "Listening…",
            };
            // Speech RMS sits well under 0.3, so scale it up to fill the meter.
            let meter = (d.level.max(0.).sqrt() * 2.).min(1.);
            Some(
                row()
                    .id("dictation-status")
                    .role(Role::Status)
                    .child(
                        div()
                            .relative()
                            .h_3()
                            .w_1()
                            .flex_shrink_0()
                            .overflow_hidden()
                            .rounded_full()
                            .bg(content(0.1))
                            .child(
                                div()
                                    .absolute()
                                    .bottom_0()
                                    .left_0()
                                    .w_full()
                                    .rounded_full()
                                    .bg(accent_color())
                                    .h(relative(meter)),
                            ),
                    )
                    .child(div().flex_shrink_0().text_color(content(0.7)).child(status))
                    .child(
                        div()
                            .debug_selector(|| "dictation-partial".into())
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_color(content(0.4))
                            .child(d.partial.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(content(0.35))
                            .child("Esc to cancel"),
                    )
                    // Reachable when focus has left the field or it is disabled.
                    .when(*state != DictationState::Finishing, |row| {
                        row.child(
                            primary("dictation-stop", "Stop dictation", "Stop")
                                .on_click(act(Dictation::stop)),
                        )
                    })
                    .child(
                        quiet("dictation-cancel", "Cancel dictation", "Cancel")
                            .on_click(act(Dictation::cancel)),
                    )
                    .into_any_element(),
            )
        }
    }
}

/// The composer's mic button: toggles on click.
pub(crate) fn mic_button(
    dictation: &Entity<Dictation>,
    input: &Entity<Input>,
    cx: &App,
) -> AnyElement {
    let d = dictation.read(cx);
    let dictating = d.dictating();
    let disabled = d.disabled(cx) && !dictating;
    let shortcut = cx.try_global::<crate::settings::Preferences>().map_or_else(
        || crate::settings::command_shortcut(&Default::default(), DICTATE_COMMAND),
        |p| crate::settings::command_shortcut(p, DICTATE_COMMAND),
    );
    let label: SharedString = if dictating {
        "Stop dictation".into()
    } else if let Some(shortcut) = shortcut {
        format!("Dictate ({shortcut})").into()
    } else {
        "Dictate".into()
    };
    let dictation = dictation.clone();
    let input = input.clone();
    super::tool_pill("composer-mic", "Dictate")
        .debug_selector(|| "composer-mic".into())
        .aria_label(label)
        .w(px(26.))
        .justify_center()
        .when(dictating, |d| d.bg(accent_color().opacity(0.25)))
        .when(disabled, |d| d.opacity(0.4).cursor_default())
        .child(
            icon("mic-01", 14.)
                .text_color(if dictating { content(1.) } else { content(0.5) })
                .group_hover("composer-mic", |s| s.text_color(content(1.))),
        )
        .on_click(move |_, window, cx| {
            if disabled {
                return;
            }
            dictation.update(cx, |d, cx| d.toggle(cx));
            window.focus(&input.focus_handle(cx), cx);
        })
        .into_any_element()
}

/// A hub that records requests instead of running the helper, installed as
/// the app's; tests feed it helper events through the real event channel.
#[cfg(test)]
pub(crate) fn with_sink(
    store: ModelStore,
    send: impl Fn(Request) + 'static,
    cx: &mut App,
) -> Entity<Speech> {
    let send: std::rc::Rc<dyn Fn(Request)> = std::rc::Rc::new(send);
    let hub = cx.new(|cx| {
        Speech::new(
            Transport::Sink {
                send,
                unavailable: false,
            },
            store,
            cx,
        )
    });
    cx.set_global(SpeechGlobal(hub.clone()));
    hub
}

#[cfg(test)]
impl Speech {
    pub(crate) fn emit(&self, event: Event) {
        self.events.try_send(event).unwrap();
    }
    pub(crate) fn give_up(&mut self) {
        if let Transport::Sink { unavailable, .. } = &mut self.transport {
            *unavailable = true;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
