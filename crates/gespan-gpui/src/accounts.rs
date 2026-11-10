//! Shared provider accounts, identities and per-account subscription snapshots.
pub(crate) mod chat_signin;
mod masked;
pub(crate) mod resets;
mod signin;
pub(crate) mod ui;
use crate::{
    input::{self, Input},
    runtime::{Database, locked},
};
use gespan_core::{
    accounts::{self, Account, AuthAction, Book, Profile},
    provider::Kind,
    usage::{self, FetchError, Snapshot},
};
use gpui::{
    App, Context, Entity, EventEmitter, Focusable, Global, Subscription, Task, Window, prelude::*,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) struct Shared(pub Entity<Accounts>);
impl Global for Shared {}
pub(crate) fn get(cx: &App) -> Option<Entity<Accounts>> {
    cx.try_global::<Shared>().map(|s| s.0.clone())
}
pub(crate) fn install(store: Database, cx: &mut App) {
    if get(cx).is_some() {
        return;
    }
    let (root, book) = locked(&store)
        .map(|s| {
            (
                s.data_dir().map(Path::to_path_buf),
                s.provider_accounts().unwrap_or_default(),
            )
        })
        .unwrap_or_default();
    let root = root.unwrap_or_else(|| {
        std::env::temp_dir().join(format!(
            "gespan-test-accounts-{}",
            gespan_core::pilot::new_id()
        ))
    });
    let center = cx.new(|cx| Accounts::new(root, book, store, cx));
    cx.set_global(Shared(center));
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub kind: Kind,
    pub profile: Profile,
    pub billing: String,
}
impl Key {
    pub(crate) fn new(kind: Kind, profile: Profile, model: Option<&str>) -> Self {
        let billing = if kind == Kind::Pi {
            model
                .unwrap_or("")
                .trim_start_matches("pi:")
                .split('/')
                .next()
                .unwrap_or("")
                .into()
        } else {
            String::new()
        };
        Self {
            kind,
            profile,
            billing,
        }
    }
    fn model(&self) -> Option<String> {
        (!self.billing.is_empty()).then(|| format!("{}/", self.billing))
    }
}
#[derive(Clone, Default)]
pub(crate) struct Cached {
    pub snapshot: Option<Snapshot>,
    pub fetching: bool,
    pub error: Option<FetchError>,
    pub attempted: Option<Instant>,
    pub generation: u64,
    pub queued: bool,
    /// What the CLI's status says, asked only when usage could not be read.
    pub sign_in: Option<accounts::SignInState>,
}
impl Cached {
    /// A provider's own mid-turn reading is fresher than any probe, and it
    /// stands in for a probe that failed or has not answered yet.
    pub(crate) fn live(&mut self, windows: &[usage::Window], now: i64) {
        if windows.is_empty() {
            return;
        }
        self.snapshot
            .get_or_insert_with(Snapshot::default)
            .merge_live(windows, now);
        self.error = None;
    }
    /// Usage when there is some; otherwise the sign-in, so a provider that
    /// reports no usage, or a CLI that is missing or signed out, still says
    /// where it stands.
    pub(crate) fn status(&self, now: i64) -> Status {
        if let Some(snapshot) = &self.snapshot
            && let Some(headroom) = snapshot.headroom(now)
        {
            if headroom <= 0. {
                return Status {
                    tone: Tone::Exhausted,
                    label: "Exhausted".into(),
                    detail: snapshot
                        .exhausted_reset()
                        .filter(|at| *at > now)
                        .map(|at| format!("back in {}", usage::duration(at - now))),
                };
            }
            if headroom <= 20. {
                return Status {
                    tone: Tone::Low,
                    label: "Running low".into(),
                    detail: Some(format!("{headroom:.0}% left")),
                };
            }
            return Status::plain(Tone::Ready, "Ready");
        }
        if self.fetching || (self.snapshot.is_none() && self.error.is_none()) {
            return Status::checking();
        }
        let unavailable = self.error.as_ref().is_some_and(|e| e.unavailable);
        match self.sign_in {
            Some(accounts::SignInState::NotInstalled) => {
                return Status::plain(Tone::Unknown, "Not installed");
            }
            Some(accounts::SignInState::SignedOut) => {
                return Status::plain(Tone::SignedOut, "Signed out");
            }
            Some(accounts::SignInState::SignedIn) if unavailable => {
                return Status::plain(Tone::Ready, "Signed in");
            }
            _ => {}
        }
        Status {
            tone: Tone::Unknown,
            label: self
                .error
                .as_ref()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| "Usage unavailable".into()),
            detail: None,
        }
    }
    /// The CLI said it is signed out or not there, and no usage says otherwise.
    pub(crate) fn signed_out(&self) -> bool {
        self.snapshot.is_none()
            && matches!(
                self.sign_in,
                Some(accounts::SignInState::SignedOut | accounts::SignInState::NotInstalled)
            )
    }
    pub(crate) fn complete(&mut self, result: Result<Snapshot, FetchError>) {
        self.fetching = false;
        match result {
            Ok(snapshot) => {
                self.snapshot = Some(snapshot);
                self.error = None;
            }
            Err(error) => {
                if error.unavailable {
                    self.snapshot = None;
                }
                self.error = Some(error);
            }
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Tone {
    Ready,
    Low,
    Exhausted,
    Checking,
    SignedOut,
    Unknown,
}
pub(crate) struct Status {
    pub tone: Tone,
    pub label: String,
    pub detail: Option<String>,
}
impl Status {
    fn checking() -> Self {
        Self::plain(Tone::Checking, "Checking…")
    }
    fn plain(tone: Tone, label: &str) -> Self {
        Self {
            tone,
            label: label.into(),
            detail: None,
        }
    }
}
#[derive(Clone)]
pub(crate) enum AccountEvent {
    Updated,
    AuthStarted,
    PickerDismiss,
    Manage,
    Authenticated(Kind, String),
    Removing(Kind, String),
    Select {
        kind: Kind,
        id: String,
        project: String,
        session: Option<i64>,
    },
}
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Surface {
    Settings,
    InlineSettings,
    Footer,
    /// The sign-in card a chat shows when its provider signed out.
    Chat,
}
struct Editor {
    kind: Kind,
    id: Option<String>,
    input: Entity<Input>,
    surface: Surface,
    previous_focus: Option<gpui::FocusHandle>,
    _submit: Subscription,
    _changed: Subscription,
}
struct SignIn {
    kind: Kind,
    profile: Profile,
    action: AuthAction,
    surface: Surface,
    cancel: Arc<AtomicBool>,
    complete: bool,
    error: Option<String>,
    adding: Option<Account>,
    /// What the CLI asks of the user, for the chat card.
    prompt: accounts::LoginPrompt,
    /// Codes the user pastes back, for a CLI that waits for one.
    codes: Option<std::sync::mpsc::Sender<String>>,
    started: Instant,
}
struct Picker {
    kind: Kind,
    project: String,
    session: Option<i64>,
    index: usize,
}
pub(crate) struct Accounts {
    pub book: Book,
    pub root: PathBuf,
    pub usage: HashMap<Key, Cached>,
    pub reset_actions: HashMap<Key, resets::ResetAction>,
    pub identities: HashMap<(Kind, String), accounts::Identity>,
    identity_loading: std::collections::HashSet<(Kind, String)>,
    identity_generation: HashMap<(Kind, String), u64>,
    store: Database,
    editor: Option<Editor>,
    signin: Option<SignIn>,
    pub error: Option<String>,
    working: bool,
    pub mask_emails: bool,
    pub show_remaining: bool,
    revealed: std::collections::HashSet<(Kind, String)>,
    dialog_focus: gpui::FocusHandle,
    dialog_presented: bool,
    previous_focus: Option<gpui::FocusHandle>,
    picker_focus: gpui::FocusHandle,
    picker: Option<Picker>,
    _clock: Task<()>,
}
impl EventEmitter<AccountEvent> for Accounts {}
impl Accounts {
    fn new(root: PathBuf, book: Book, store: Database, cx: &mut Context<Self>) -> Self {
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        let keys: Vec<_> = this.usage.keys().cloned().collect();
                        for key in keys {
                            this.refresh(key, false, cx);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            book,
            root,
            store,
            usage: HashMap::new(),
            reset_actions: HashMap::new(),
            identities: HashMap::new(),
            identity_loading: Default::default(),
            identity_generation: Default::default(),
            editor: None,
            signin: None,
            error: None,
            working: false,
            mask_emails: true,
            show_remaining: false,
            revealed: Default::default(),
            dialog_focus: cx.focus_handle(),
            dialog_presented: false,
            previous_focus: None,
            picker_focus: cx.focus_handle(),
            picker: None,
            _clock: clock,
        }
    }
    pub(crate) fn profile(&self, id: &str) -> Profile {
        Profile::new(id, &self.root)
    }
    pub(crate) fn hide_emails(&mut self) {
        self.revealed.clear();
    }
    pub(crate) fn open_picker(
        &mut self,
        kind: Kind,
        selected: &str,
        project: &str,
        session: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picker = Some(Picker {
            kind,
            project: project.into(),
            session,
            index: self
                .book
                .list(kind)
                .iter()
                .position(|a| a.id == selected)
                .unwrap_or_default(),
        });
        window.focus(&self.picker_focus, cx);
        cx.notify();
    }
    fn picker_key(&mut self, event: &gpui::KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.picker else {
            return;
        };
        let rows = self.book.list(picker.kind);
        if rows.is_empty() {
            return;
        }
        match event.keystroke.key.as_str() {
            "down" => picker.index = (picker.index + 1) % rows.len(),
            "up" => picker.index = (picker.index + rows.len() - 1) % rows.len(),
            "home" => picker.index = 0,
            "end" => picker.index = rows.len() - 1,
            "enter" | "space" => {
                let (kind, project, session) =
                    (picker.kind, picker.project.clone(), picker.session);
                let id = rows[picker.index.min(rows.len() - 1)].id.clone();
                self.choose(kind, id, project, session, cx);
            }
            "escape" | "tab" => cx.emit(AccountEvent::PickerDismiss),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
    pub(crate) fn suggestion(&self, kind: Kind, selected: &str) -> Option<Account> {
        let now = usage::now_ms();
        let room = |id: &str| {
            self.usage
                .get(&Key::new(kind, self.profile(id), None))?
                .snapshot
                .as_ref()?
                .headroom(now)
        };
        if room(selected)? > 20. {
            return None;
        }
        self.book
            .list(kind)
            .into_iter()
            .filter(|a| a.id != selected)
            .filter_map(|a| {
                let n = room(&a.id)?;
                (n > 20.).then_some((a, n))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(a, _)| a)
    }
    pub(crate) fn selected(&self, kind: Kind, project: &str) -> Profile {
        self.profile(&self.book.selected(kind, project))
    }
    /// Who the account is signed in as, unless its CLI says it is signed out
    /// or missing: a config file can outlive its sign-in.
    pub(crate) fn identity_shown(&self, kind: Kind, id: &str) -> Option<&accounts::Identity> {
        let gone = self
            .usage
            .get(&Key::new(kind, self.profile(id), None))
            .is_some_and(Cached::signed_out);
        self.identities
            .get(&(kind, id.to_owned()))
            .filter(|_| !gone)
    }
    /// The plan the account's usage report names, for its badge when the
    /// sign-in itself does not say.
    pub(crate) fn usage_plan(&self, kind: Kind, id: &str) -> Option<String> {
        self.usage
            .get(&Key::new(kind, self.profile(id), None))?
            .snapshot
            .as_ref()?
            .plan
            .clone()
    }
    /// React's `accountStatus`: a tone, a word, and an optional detail.
    pub(crate) fn status(&self, kind: Kind, id: &str) -> Status {
        self.usage
            .get(&Key::new(kind, self.profile(id), None))
            .map(|state| state.status(usage::now_ms()))
            .unwrap_or_else(Status::checking)
    }
    pub(crate) fn refresh(&mut self, key: Key, force: bool, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let state = self.usage.entry(key.clone()).or_default();
        if state.fetching {
            state.queued |= force;
            return;
        }
        let reset = state.snapshot.as_ref().is_some_and(|s| {
            s.windows.iter().any(|w| {
                w.resets_at
                    .is_some_and(|at| at > s.updated_at && at <= usage::now_ms())
            })
        });
        if !force
            && state
                .attempted
                .is_some_and(|at| at.elapsed() < Duration::from_secs(if reset { 300 } else { 900 }))
        {
            return;
        }
        state.fetching = true;
        state.attempted = Some(Instant::now());
        let generation = state.generation;
        let request = key.clone();
        let work = cx.background_executor().spawn(async move {
            let result =
                usage::fetch_for(request.kind, &request.profile, request.model().as_deref());
            // Without usage the row falls back to the CLI's own sign-in status.
            let sign_in = result
                .is_err()
                .then(|| accounts::sign_in_state(request.kind, &request.profile));
            (result, sign_in)
        });
        cx.spawn(async move |this, cx| {
            let (result, sign_in) = work.await;
            let _ = this.update(cx, |this, cx| {
                if key.profile.id != accounts::DEFAULT
                    && !this.book.exists(key.kind, &key.profile.id)
                {
                    return;
                }
                let state = this.usage.entry(key.clone()).or_default();
                if state.generation != generation {
                    return;
                }
                let queued = std::mem::take(&mut state.queued);
                match &result {
                    Ok(snapshot) => tracing::info!(
                        provider = key.kind.label(),
                        account = %key.profile.id,
                        windows = snapshot.windows.len(),
                        "usage refreshed"
                    ),
                    Err(error) => tracing::warn!(
                        provider = key.kind.label(),
                        account = %key.profile.id,
                        unavailable = error.unavailable,
                        message = %error.message,
                        "usage refresh failed"
                    ),
                }
                let refreshed = result.is_ok();
                state.complete(result);
                state.sign_in = sign_in;
                if refreshed
                    && let Some(action) = this.reset_actions.get_mut(&key)
                    && !action.applying()
                {
                    action.row = None;
                }
                if let Some(identity) = state.snapshot.as_ref().and_then(|s| s.identity.clone()) {
                    this.identities
                        .insert((key.kind, key.profile.id.clone()), identity);
                }
                if queued {
                    this.refresh(key, true, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn live_usage(
        &mut self,
        key: Key,
        windows: &[usage::Window],
        cx: &mut Context<Self>,
    ) {
        self.usage
            .entry(key)
            .or_default()
            .live(windows, usage::now_ms());
        cx.notify();
    }
    pub(crate) fn load_provider(&mut self, kind: Kind, force: bool, cx: &mut Context<Self>) {
        for account in self.book.list(kind) {
            self.refresh(Key::new(kind, self.profile(&account.id), None), force, cx);
            if force {
                self.identities.remove(&(kind, account.id.clone()));
                self.identity_loading.remove(&(kind, account.id.clone()));
                *self
                    .identity_generation
                    .entry((kind, account.id.clone()))
                    .or_default() += 1;
            }
            self.identity(kind, &account.id, cx);
        }
    }
    fn identity(&mut self, kind: Kind, id: &str, cx: &mut Context<Self>) {
        let key = (kind, id.to_owned());
        if cfg!(test)
            || self.identities.contains_key(&key)
            || !self.identity_loading.insert(key.clone())
        {
            return;
        }
        let profile = self.profile(id);
        let generation = *self.identity_generation.entry(key.clone()).or_default();
        let work = cx
            .background_executor()
            .spawn(async move { profile.identity(kind) });
        cx.spawn(async move |this, cx| {
            let identity = work.await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .identity_generation
                    .get(&key)
                    .copied()
                    .unwrap_or_default()
                    != generation
                    || !this.book.exists(key.0, &key.1)
                {
                    this.identity_loading.remove(&key);
                    return;
                }
                if let Some(identity) = identity {
                    this.identities.insert(key, identity);
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn persist(&mut self, book: Book, cx: &mut Context<Self>) {
        self.working = true;
        if let Some(editor) = &self.editor {
            editor
                .input
                .update(cx, |input, _| input.set_read_only(true));
        }
        self.error = None;
        let store = self.store.clone();
        let saved = book.clone();
        let work = cx
            .background_executor()
            .spawn(async move { locked(&store)?.save_provider_accounts(&saved) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.working = false;
                if let Some(editor) = &this.editor {
                    editor
                        .input
                        .update(cx, |input, _| input.set_read_only(false));
                }
                match result {
                    Ok(()) => {
                        let added: Vec<_> = book
                            .accounts
                            .iter()
                            .filter(|a| !this.book.exists(Kind::from_id(&a.provider), &a.id))
                            .cloned()
                            .collect();
                        this.book = book;
                        this.editor = None;
                        for account in added {
                            this.invalidate(Kind::from_id(&account.provider), &account.id, cx);
                        }
                        cx.emit(AccountEvent::Updated);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn choose(
        &mut self,
        kind: Kind,
        id: String,
        project: String,
        session: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        if self.working || !self.book.exists(kind, &id) {
            return;
        }
        let mut book = self.book.clone();
        if let Err(error) = book.select(kind, &project, &id) {
            self.error = Some(error);
            return;
        }
        self.persist(book, cx);
        cx.emit(AccountEvent::Select {
            kind,
            id,
            project,
            session,
        });
    }
    pub(crate) fn start_edit(
        &mut self,
        kind: Kind,
        id: Option<String>,
        surface: Surface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.working {
            return;
        }
        let value = id
            .as_ref()
            .and_then(|id| self.book.list(kind).into_iter().find(|a| a.id == *id))
            .map(|a| a.label)
            .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut input = Input::field(
                format!(
                    "{} {} account",
                    if id.is_some() { "Rename" } else { "New" },
                    kind.label()
                ),
                "Work or Personal",
                crate::theme::content(0.25),
                cx,
            );
            input.set_font_size(12.);
            input.set_max_chars(48);
            input
        });
        input.update(cx, |input, cx| input.set_text(value, cx));
        let submit = cx.subscribe_in(&input, window, |this, _, _: &input::Submitted, _, cx| {
            this.submit_editor(cx)
        });
        let changed = cx.subscribe(&input, |_, _, _: &input::Changed, cx| cx.notify());
        let previous_focus = window.focused(cx);
        window.focus(&input.read(cx).focus_handle(cx), cx);
        self.editor = Some(Editor {
            kind,
            id,
            input,
            surface,
            previous_focus,
            _submit: submit,
            _changed: changed,
        });
        self.error = None;
        cx.notify();
    }
    fn submit_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else {
            return;
        };
        if self.working {
            return;
        }
        let label = editor.input.read(cx).content.to_string();
        if label.trim().is_empty() {
            return;
        }
        let kind = editor.kind;
        let id = editor.id.clone();
        let mut book = self.book.clone();
        if let Some(id) = id {
            match book.rename(kind, &id, &label) {
                Ok(()) => self.persist(book, cx),
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                }
            }
        } else {
            match book.add(kind, &label) {
                Ok(account) => self.begin_auth(
                    kind,
                    &account.id.clone(),
                    AuthAction::SignIn,
                    Some(account),
                    cx,
                ),
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                }
            }
        }
    }
    pub(crate) fn begin_auth(
        &mut self,
        kind: Kind,
        id: &str,
        action: AuthAction,
        adding: Option<Account>,
        cx: &mut Context<Self>,
    ) {
        let surface = self
            .editor
            .as_ref()
            .filter(|_| adding.is_some())
            .map_or(Surface::Settings, |editor| editor.surface);
        self.start_auth(kind, id, action, adding, surface, cx);
    }
    /// Whether a sign-in is still running, not finished or failed.
    pub(crate) fn auth_in_progress(&self) -> bool {
        self.signin
            .as_ref()
            .is_some_and(|s| !s.complete && s.error.is_none())
    }
    pub(crate) fn begin_footer_auth(&mut self, kind: Kind, id: &str, cx: &mut Context<Self>) {
        self.start_auth(kind, id, AuthAction::SignIn, None, Surface::Footer, cx);
    }
    fn start_auth(
        &mut self,
        kind: Kind,
        id: &str,
        action: AuthAction,
        adding: Option<Account>,
        surface: Surface,
        cx: &mut Context<Self>,
    ) {
        if self.working
            || self
                .signin
                .as_ref()
                .is_some_and(|s| !s.complete && s.error.is_none())
        {
            return;
        }
        let surface = if surface == Surface::Settings && adding.is_some() {
            Surface::InlineSettings
        } else {
            surface
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let request = cancel.clone();
        if !matches!(surface, Surface::Footer | Surface::Chat) || adding.is_some() {
            self.working = true;
        }
        let interactive = surface == Surface::Chat && action == AuthAction::SignIn;
        let (codes, code_rx) = std::sync::mpsc::channel();
        let (prompt_tx, prompt_rx) = async_channel::unbounded();
        self.error = None;
        let profile = self.profile(id);
        if self.signin.is_none() {
            self.dialog_presented = false;
        }
        if action == AuthAction::SignOut {
            cx.emit(AccountEvent::Removing(kind, id.into()));
        }
        self.signin = Some(SignIn {
            kind,
            profile: profile.clone(),
            action,
            surface,
            cancel: cancel.clone(),
            complete: false,
            error: None,
            adding,
            prompt: Default::default(),
            codes: interactive.then_some(codes),
            started: Instant::now(),
        });
        if self.has_modal_auth() {
            cx.emit(AccountEvent::AuthStarted);
        }
        if let Some(editor) = &self.editor {
            editor
                .input
                .update(cx, |input, _| input.set_read_only(true));
        }
        let work = cx.background_executor().spawn(async move {
            if action == AuthAction::SignOut {
                accounts::stop_processes(kind, &profile);
            }
            let interaction = interactive.then(|| accounts::Interaction {
                prompt: Box::new(move |prompt| {
                    let _ = prompt_tx.try_send(prompt);
                }),
                codes: code_rx,
            });
            let result = accounts::authenticate_with(kind, &profile, action, cancel, interaction);
            gespan_core::catalog::invalidate(kind, &profile);
            result
        });
        if interactive {
            let request = request.clone();
            cx.spawn(async move |this, cx| {
                while let Ok(prompt) = prompt_rx.recv().await {
                    let live = this.update(cx, |this, cx| {
                        let Some(signin) = this
                            .signin
                            .as_mut()
                            .filter(|s| Arc::ptr_eq(&s.cancel, &request))
                        else {
                            return false;
                        };
                        signin.prompt = prompt;
                        cx.notify();
                        true
                    });
                    if !matches!(live, Ok(true)) {
                        break;
                    }
                }
            })
            .detach();
        }
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                let Some(signin) = &mut this.signin else {
                    return;
                };
                let kind = signin.kind;
                let id = signin.profile.id.clone();
                if !Arc::ptr_eq(&signin.cancel, &request) || signin.cancel.load(Ordering::Relaxed) {
                    return;
                }
                if !matches!(signin.surface, Surface::Footer | Surface::Chat)
                    || signin.adding.is_some()
                {
                    this.working = false;
                }
                signin.codes = None;
                if let Some(editor) = &this.editor {
                    editor
                        .input
                        .update(cx, |input, _| input.set_read_only(false));
                }
                match result {
                    Err(error) => {
                        signin.error = Some(error.clone());
                        if signin.adding.is_some() {
                            this.error = Some(error);
                        }
                    }
                    Ok(()) => {
                        signin.complete = true;
                        let adding = signin.adding.take();
                        if let Some(account) = adding {
                            let mut book = this.book.clone();
                            book.accounts.push(account);
                            this.persist(book, cx);
                        }
                        this.invalidate(kind, &id, cx);
                        cx.emit(AccountEvent::Authenticated(kind, id));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn invalidate(&mut self, kind: Kind, id: &str, cx: &mut Context<Self>) {
        self.reset_actions
            .retain(|key, _| key.kind != kind || key.profile.id != id);
        self.identities.remove(&(kind, id.to_owned()));
        self.identity_loading.remove(&(kind, id.to_owned()));
        *self
            .identity_generation
            .entry((kind, id.to_owned()))
            .or_default() += 1;
        for (key, state) in &mut self.usage {
            if key.kind == kind && key.profile.id == id {
                state.generation += 1;
                state.fetching = false;
                state.snapshot = None;
                state.attempted = None;
            }
        }
        self.refresh(Key::new(kind, self.profile(id), None), true, cx);
        self.identity(kind, id, cx);
    }
    fn remove(&mut self, account: Account, window: &mut Window, cx: &mut Context<Self>) {
        if self.working || account.is_default() {
            return;
        }
        let answer=window.prompt(gpui::PromptLevel::Warning,&format!("Remove “{}”?",account.label),Some("Its stored credentials will be deleted and any running turns for this account will stop. Existing conversations stay in history, but cannot continue until you switch accounts."),&["Remove account","Cancel"],cx);
        cx.spawn_in(window, async move |this, cx| {
            if !matches!(answer.await, Ok(0)) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let kind = Kind::from_id(&account.provider);
                let profile = this.profile(&account.id);
                let mut book = this.book.clone();
                if book.remove(kind, &account.id).is_err() {
                    return;
                }
                this.working = true;
                cx.emit(AccountEvent::Removing(kind, account.id.clone()));
                let store = this.store.clone();
                let saved = book.clone();
                let work = cx.background_executor().spawn(async move {
                    accounts::stop_processes(kind, &profile);
                    profile.remove_credentials(kind)?;
                    locked(&store)?.save_provider_accounts(&saved)
                });
                cx.spawn(async move |this, cx| {
                    let result = work.await;
                    let _ = this.update(cx, |this, cx| {
                        this.working = false;
                        match result {
                            Ok(()) => {
                                this.book = book;
                                this.usage.retain(|key, _| {
                                    key.kind != kind || key.profile.id != account.id
                                });
                                this.reset_actions.retain(|key, _| {
                                    key.kind != kind || key.profile.id != account.id
                                });
                                this.identities.remove(&(kind, account.id));
                                cx.emit(AccountEvent::Updated);
                            }
                            Err(error) => this.error = Some(error),
                        }
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            });
        })
        .detach();
    }
}
impl Accounts {
    /// Gespan came back to the front while a browser sign-in is still
    /// waiting. A finished login reaches the CLI within a second or two, so if
    /// it is still waiting after a short grace the browser was closed or
    /// abandoned: reset to Sign in instead of waiting out the CLI's 10 minutes.
    /// A chat sign-in that waits for a pasted code is left alone, because the
    /// user comes back from the browser to paste it.
    pub(crate) fn app_reactivated(&mut self, cx: &mut Context<Self>) {
        let Some(signin) = &self.signin else {
            return;
        };
        if signin.complete
            || signin.error.is_some()
            || signin.action != AuthAction::SignIn
            || signin.codes.is_some()
            || signin.started.elapsed() < Duration::from_secs(2)
        {
            return;
        }
        let attempt = signin.cancel.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = this.update(cx, |this, cx| {
                let waiting = this.signin.as_ref().is_some_and(|s| {
                    Arc::ptr_eq(&s.cancel, &attempt) && !s.complete && s.error.is_none()
                });
                if waiting {
                    this.cancel_auth(cx);
                }
            });
        })
        .detach();
    }
}
impl Drop for Accounts {
    fn drop(&mut self) {
        if let Some(signin) = &self.signin {
            signin.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::accounts::SignInState;

    fn probed(result: Result<Snapshot, FetchError>, sign_in: Option<SignInState>) -> Cached {
        let mut state = Cached::default();
        state.complete(result);
        state.sign_in = sign_in;
        state
    }
    fn label(state: &Cached) -> (String, bool, bool) {
        let status = state.status(usage::now_ms());
        (
            status.label,
            status.tone == Tone::Ready,
            status.tone == Tone::SignedOut,
        )
    }
    fn no_usage() -> FetchError {
        FetchError {
            message: "No subscription usage is available for Cursor.".into(),
            unavailable: true,
        }
    }

    #[test]
    fn a_provider_without_usage_shows_its_sign_in() {
        let signed_in = probed(Err(no_usage()), Some(SignInState::SignedIn));
        assert_eq!(label(&signed_in), ("Signed in".into(), true, false));
        let signed_out = probed(Err(no_usage()), Some(SignInState::SignedOut));
        assert_eq!(label(&signed_out), ("Signed out".into(), false, true));
        let missing = probed(Err(no_usage()), Some(SignInState::NotInstalled));
        assert_eq!(label(&missing), ("Not installed".into(), false, false));
        // A CLI that does not say keeps the usage reader's own words.
        let silent = probed(Err(no_usage()), Some(SignInState::Unknown));
        assert_eq!(
            label(&silent).0,
            "No subscription usage is available for Cursor."
        );
    }

    #[test]
    fn usage_wins_over_sign_in_and_a_failed_read_is_not_called_signed_in() {
        let snapshot = Snapshot {
            windows: vec![usage::Window::new(40., 300, None)],
            ..Default::default()
        };
        assert_eq!(
            label(&probed(Ok(snapshot), None)),
            ("Ready".into(), true, false)
        );
        // Signed in, but the usage read itself failed: say so, not "Signed in".
        let offline = FetchError {
            message: "Kiro usage report was unexpected.".into(),
            unavailable: false,
        };
        assert_eq!(
            label(&probed(Err(offline), Some(SignInState::SignedIn))).0,
            "Kiro usage report was unexpected."
        );
        // Signed out beats the reader's sign-in hint, whatever it says.
        let hint = FetchError {
            message: "Sign in with kiro-cli login, then refresh usage.".into(),
            unavailable: true,
        };
        assert_eq!(
            label(&probed(Err(hint), Some(SignInState::SignedOut))).0,
            "Signed out"
        );
        // A config left behind by a sign-out does not name the account.
        assert!(probed(Err(no_usage()), Some(SignInState::SignedOut)).signed_out());
        assert!(probed(Err(no_usage()), Some(SignInState::NotInstalled)).signed_out());
        assert!(!probed(Err(no_usage()), Some(SignInState::SignedIn)).signed_out());
        assert!(!probed(Err(no_usage()), None).signed_out());
        let mut checking = probed(Err(no_usage()), Some(SignInState::SignedIn));
        checking.fetching = true;
        assert_eq!(label(&checking).0, "Checking…");
        assert_eq!(label(&Cached::default()).0, "Checking…");
    }
    #[test]
    fn usage_cache_preserves_stale_data_on_errors_and_separates_accounts_and_pi_billing() {
        let snapshot = Snapshot {
            windows: vec![usage::Window::new(40., 300, None)],
            ..Default::default()
        };
        let mut state = Cached::default();
        state.complete(Ok(snapshot));
        state.complete(Err(FetchError {
            message: "Offline".into(),
            unavailable: false,
        }));
        assert_eq!(
            state.snapshot.as_ref().unwrap().windows[0].used_percent,
            40.
        );
        state.complete(Err(FetchError {
            message: "Not signed in".into(),
            unavailable: true,
        }));
        assert!(state.snapshot.is_none());
        assert_ne!(
            Key::new(Kind::Codex, Profile::new("a", Path::new("/fixture")), None),
            Key::new(Kind::Codex, Profile::new("b", Path::new("/fixture")), None)
        );
        assert_eq!(
            Key::new(Kind::Pi, Profile::default(), Some("anthropic/model-a")),
            Key::new(Kind::Pi, Profile::default(), Some("pi:anthropic/model-b"))
        );
        assert_ne!(
            Key::new(Kind::Pi, Profile::default(), Some("anthropic/model")),
            Key::new(Kind::Pi, Profile::default(), Some("openai-codex/model"))
        );
    }
}
