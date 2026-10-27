//! Desktop dictation commands: speech model downloads and the supervised
//! speech helper process. The commands are thin wrappers over the plain
//! functions below so the tests need no Tauri app.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use gespan_models::{default_root, CancelToken, ModelSpec, ModelStatus, ModelStore, Progress};
use gespan_speech::catalog;
use gespan_speech::protocol::{ErrorKind, Event, Request};
use gespan_speech::{helper_command, local_branches, pick_terms, SpeechClient, VocabularySources};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

pub const DICTATION_EVENT: &str = "dictation-event";
pub const DICTATION_DOWNLOAD: &str = "dictation-download";
/// Sent to a window whose dictation another window took over.
pub const DICTATION_TAKEN_OVER: &str = "dictation-taken-over";

/// At most this often, except for the final 100% report.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

/// One window's dictation attempt. The webview numbers its own attempts.
#[derive(Debug, Clone, PartialEq)]
struct Attempt {
    window: String,
    id: u64,
}

struct Owner {
    attempt: Attempt,
    /// This attempt's `Listening` has arrived. Until then a partial or final
    /// is the tail of a take that was cancelled.
    heard: bool,
}

impl Attempt {
    fn of(window: &WebviewWindow, id: u64) -> Self {
        Self {
            window: window.label().to_string(),
            id,
        }
    }
}

pub struct State {
    client: Mutex<Option<SpeechClient>>,
    store: ModelStore,
    cancels: Mutex<HashMap<String, CancelToken>>,
    /// Model id last sent to the helper in a `Load`.
    loaded: Mutex<Option<String>>,
    /// The attempt the helper is listening for. The helper is shared by every
    /// window, so ownership is decided here. Held while sending, so requests
    /// reach the helper in the order ownership changed.
    owner: Mutex<Option<Owner>>,
    /// Tests record requests here instead of running the helper.
    #[cfg(test)]
    sent: Mutex<Option<Vec<Request>>>,
}

impl State {
    fn with_store(store: ModelStore) -> Self {
        Self {
            client: Mutex::new(None),
            store,
            cancels: Mutex::new(HashMap::new()),
            loaded: Mutex::new(None),
            owner: Mutex::new(None),
            #[cfg(test)]
            sent: Mutex::new(None),
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::with_store(ModelStore::new(default_root()))
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    id: String,
    label: String,
    size_bytes: u64,
    status: &'static str,
    downloaded: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged, rename_all_fields = "camelCase")]
enum DownloadEvent {
    Progress {
        model_id: String,
        downloaded: u64,
        total: u64,
    },
    Done {
        model_id: String,
        done: bool,
    },
    Failed {
        model_id: String,
        error: String,
    },
}

struct StartArgs {
    project_path: Option<String>,
    model_id: String,
    user_words: Vec<String>,
    known_names: Vec<String>,
}

struct ProgressThrottle {
    last: Instant,
}

impl ProgressThrottle {
    fn new(now: Instant) -> Self {
        Self { last: now }
    }

    fn allow(&mut self, now: Instant, downloaded: u64, total: u64) -> bool {
        if downloaded >= total || now.duration_since(self.last) >= PROGRESS_INTERVAL {
            self.last = now;
            return true;
        }
        false
    }
}

fn find_spec(model_id: &str) -> Result<&'static ModelSpec, String> {
    catalog::ALL
        .iter()
        .copied()
        .find(|spec| spec.id == model_id)
        .ok_or_else(|| "unknown-model".to_string())
}

fn models_impl(state: &State) -> Vec<ModelInfo> {
    catalog::ALL
        .iter()
        .map(|spec| {
            let (status, downloaded) = match state.store.status(spec) {
                ModelStatus::Missing => ("missing", 0),
                ModelStatus::Partial { downloaded, .. } => ("partial", downloaded),
                ModelStatus::Ready => ("ready", spec.total_size()),
            };
            ModelInfo {
                id: spec.id.into(),
                label: spec.label.into(),
                size_bytes: spec.total_size(),
                status,
                downloaded,
            }
        })
        .collect()
}

/// Registers a cancel token for `model_id`, refusing a second download of the
/// same model.
fn register_download(state: &State, model_id: &str) -> Result<CancelToken, String> {
    let mut cancels = lock(&state.cancels);
    if cancels.contains_key(model_id) {
        return Err("already-downloading".into());
    }
    let token = CancelToken::new();
    cancels.insert(model_id.to_string(), token.clone());
    Ok(token)
}

fn end_download(state: &State, model_id: &str) {
    lock(&state.cancels).remove(model_id);
}

/// Releases the download's cancel token when dropped, even on a panic.
struct DownloadGuard<'a> {
    state: &'a State,
    model_id: &'a str,
}

impl Drop for DownloadGuard<'_> {
    fn drop(&mut self) {
        end_download(self.state, self.model_id);
    }
}

fn cancel_download_impl(state: &State, model_id: &str) {
    if let Some(token) = lock(&state.cancels).get(model_id) {
        token.cancel();
    }
}

/// Blocking: downloads `model_id`, reporting through `emit`. Always ends with
/// a `Done` or `Failed` event and releases the cancel token.
fn download_impl(
    state: &State,
    model_id: &str,
    emit: impl Fn(DownloadEvent),
) -> Result<(), String> {
    download_spec(state, find_spec(model_id)?, emit)
}

fn download_spec(
    state: &State,
    spec: &ModelSpec,
    emit: impl Fn(DownloadEvent),
) -> Result<(), String> {
    let model_id = spec.id;
    let token = register_download(state, model_id)?;
    let guard = DownloadGuard { state, model_id };
    let mut throttle = ProgressThrottle::new(Instant::now());
    let result = state.store.download(spec, &token, |p: Progress| {
        if throttle.allow(Instant::now(), p.downloaded, p.total) {
            emit(DownloadEvent::Progress {
                model_id: model_id.to_string(),
                downloaded: p.downloaded,
                total: p.total,
            });
        }
    });
    // Release before the final event so a retry on `Done`/`Failed` is accepted.
    drop(guard);
    match result {
        Ok(()) => {
            emit(DownloadEvent::Done {
                model_id: model_id.to_string(),
                done: true,
            });
            Ok(())
        }
        Err(e) => {
            let error = e.to_string();
            emit(DownloadEvent::Failed {
                model_id: model_id.to_string(),
                error: error.clone(),
            });
            Err(error)
        }
    }
}

fn delete_model_impl(state: &State, model_id: &str) -> Result<(), String> {
    let spec = find_spec(model_id)?;
    state.store.delete(spec).map_err(|e| e.to_string())?;
    let mut loaded = lock(&state.loaded);
    if loaded.as_deref() == Some(model_id) {
        *loaded = None;
    }
    Ok(())
}

/// Blocking (reads git): spawns the helper on first use, loads the model if it
/// changed, then starts listening with the project's vocabulary.
fn start_impl(
    state: &State,
    sink: &EventSink,
    who: Attempt,
    args: StartArgs,
) -> Result<Option<Attempt>, String> {
    let spec = find_spec(&args.model_id)?;
    if state.store.status(spec) != ModelStatus::Ready {
        return Err("model-missing".into());
    }

    let project_root = args.project_path.as_deref().map(PathBuf::from);
    let branches = project_root
        .as_deref()
        .map(local_branches)
        .unwrap_or_default();
    let terms = pick_terms(&VocabularySources {
        project_root: project_root.as_deref(),
        branches: &branches,
        known_names: &args.known_names,
        user_words: &args.user_words,
    });

    let mut owner = lock(&state.owner);
    spawn_helper(state, sink)?;
    // A take still owned, possibly by another window, ends first. The same
    // attempt id only comes back from a reloaded webview.
    let previous = owner.take();
    if previous.is_some() {
        send_impl(state, Request::Cancel);
    }
    let replaced = previous.map(|o| o.attempt).filter(|a| *a != who);
    let mut loaded = lock(&state.loaded);
    if loaded.as_deref() != Some(args.model_id.as_str()) {
        send_impl(
            state,
            Request::Load {
                model_dir: state.store.dir(spec),
                model_id: args.model_id.clone(),
            },
        );
        *loaded = Some(args.model_id);
    }
    send_impl(state, Request::Start { terms });
    *owner = Some(Owner {
        attempt: who,
        heard: false,
    });
    Ok(replaced)
}

fn spawn_helper(state: &State, sink: &EventSink) -> Result<(), String> {
    #[cfg(test)]
    if lock(&state.sent).is_some() {
        return Ok(());
    }
    let mut client = lock(&state.client);
    if client.as_ref().is_some_and(|c| c.is_unavailable()) {
        return Err("speech-unavailable".into());
    }
    if client.is_none() {
        let (program, helper_args) = helper_command().map_err(|e| e.to_string())?;
        let sink = sink.clone();
        *client = Some(SpeechClient::spawn(program, helper_args, move |e| sink(e)));
    }
    Ok(())
}

/// Hands a helper event to the window that owns the take, if any.
fn on_helper_event(state: &State, event: Event, out: &dyn Fn(&str, Event)) {
    // The model is gone or failed to load, so the next start loads again.
    if let Event::Error {
        kind: ErrorKind::ModelMissing | ErrorKind::ModelLoad,
        ..
    } = &event
    {
        *lock(&state.loaded) = None;
    }
    let mut owner = lock(&state.owner);
    let Some(current) = owner.as_mut() else {
        // Nobody can stop this take, and it would hold the mic until the cap.
        if event == Event::Listening {
            send_impl(state, Request::Cancel);
        }
        return;
    };
    match event {
        Event::Listening => current.heard = true,
        Event::Level { .. } | Event::Partial { .. } | Event::Final { .. } if !current.heard => {
            return
        }
        _ => {}
    }
    let window = current.attempt.window.clone();
    if matches!(event, Event::Final { .. } | Event::Error { .. }) {
        *owner = None;
    }
    drop(owner);
    out(&window, event);
}

fn owns(owner: &Option<Owner>, who: &Attempt) -> bool {
    owner.as_ref().is_some_and(|o| o.attempt == *who)
}

fn send_impl(state: &State, req: Request) {
    #[cfg(test)]
    if let Some(sent) = lock(&state.sent).as_mut() {
        sent.push(req);
        return;
    }
    if let Some(client) = lock(&state.client).as_ref() {
        client.send(req);
    }
}

/// Only the owner may stop or cancel the take; it may be another window's now.
fn stop_impl(state: &State, who: &Attempt) {
    let owner = lock(&state.owner);
    if owns(&owner, who) {
        send_impl(state, Request::Stop);
    }
}

fn cancel_impl(state: &State, who: &Attempt) {
    let mut owner = lock(&state.owner);
    if owns(&owner, who) {
        *owner = None;
        send_impl(state, Request::Cancel);
    }
}

/// Stops the helper on app exit.
pub fn shutdown(state: &State) {
    let client = lock(&state.client).take();
    if let Some(client) = client {
        client.shutdown();
    }
}

fn blocking_failed(e: tauri::Error) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn dictation_models(state: tauri::State<'_, State>) -> Result<Vec<ModelInfo>, String> {
    Ok(models_impl(&state))
}

#[tauri::command]
pub async fn dictation_download(app: AppHandle, model_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        download_impl(&state, &model_id, |event| {
            let _ = app.emit(DICTATION_DOWNLOAD, event);
        })
    })
    .await
    .map_err(blocking_failed)?
}

#[tauri::command]
pub async fn dictation_cancel_download(
    state: tauri::State<'_, State>,
    model_id: String,
) -> Result<(), String> {
    cancel_download_impl(&state, &model_id);
    Ok(())
}

#[tauri::command]
pub async fn dictation_delete_model(
    state: tauri::State<'_, State>,
    model_id: String,
) -> Result<(), String> {
    delete_model_impl(&state, &model_id)
}

#[tauri::command]
pub async fn dictation_start(
    app: AppHandle,
    window: WebviewWindow,
    attempt: u64,
    project_path: Option<String>,
    model_id: String,
    user_words: Vec<String>,
    known_names: Vec<String>,
) -> Result<(), String> {
    let who = Attempt::of(&window, attempt);
    tauri::async_runtime::spawn_blocking(move || {
        let sink: EventSink = {
            let app = app.clone();
            Arc::new(move |event| {
                on_helper_event(&app.state::<State>(), event, &|window, event| {
                    let _ = app.emit_to(window, DICTATION_EVENT, event);
                });
            })
        };
        let state = app.state::<State>();
        let replaced = start_impl(
            &state,
            &sink,
            who,
            StartArgs {
                project_path,
                model_id,
                user_words,
                known_names,
            },
        )?;
        if let Some(old) = replaced {
            let _ = app.emit_to(old.window.as_str(), DICTATION_TAKEN_OVER, old.id);
        }
        Ok(())
    })
    .await
    .map_err(blocking_failed)?
}

#[tauri::command]
pub async fn dictation_stop(
    state: tauri::State<'_, State>,
    window: WebviewWindow,
    attempt: u64,
) -> Result<(), String> {
    stop_impl(&state, &Attempt::of(&window, attempt));
    Ok(())
}

#[tauri::command]
pub async fn dictation_cancel(
    state: tauri::State<'_, State>,
    window: WebviewWindow,
    attempt: u64,
) -> Result<(), String> {
    cancel_impl(&state, &Attempt::of(&window, attempt));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_models::ModelFile;
    use gespan_models::ModelStore;
    use gespan_speech::catalog::{PARAKEET_V2_EN, PARAKEET_V3_MULTI};
    use gespan_speech::RestartPolicy;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn state() -> (tempfile::TempDir, State) {
        let dir = tempfile::tempdir().unwrap();
        let state = State::with_store(ModelStore::new(dir.path().to_path_buf()));
        (dir, state)
    }

    fn start_args(model_id: &str) -> StartArgs {
        StartArgs {
            project_path: None,
            model_id: model_id.into(),
            user_words: vec![],
            known_names: vec![],
        }
    }

    fn noop_sink() -> EventSink {
        Arc::new(|_| {})
    }

    fn who(window: &str, id: u64) -> Attempt {
        Attempt {
            window: window.into(),
            id,
        }
    }

    #[test]
    fn start_without_model_prompts_download() {
        let (_dir, state) = state();
        let err = start_impl(
            &state,
            &noop_sink(),
            who("a", 1),
            start_args(PARAKEET_V2_EN.id),
        )
        .unwrap_err();
        assert_eq!(err, "model-missing");
        // The helper is never spawned for a model that cannot load.
        assert!(state.client.lock().unwrap().is_none());
    }

    #[test]
    fn start_with_unknown_model_is_rejected() {
        let (_dir, state) = state();
        let err = start_impl(&state, &noop_sink(), who("a", 1), start_args("nope")).unwrap_err();
        assert_eq!(err, "unknown-model");
    }

    #[test]
    fn model_info_reports_partial_progress() {
        let (dir, state) = state();
        let first = &PARAKEET_V2_EN.files[0];
        let model_dir = dir.path().join(PARAKEET_V2_EN.id);
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(
            model_dir.join(format!("{}.part", first.name)),
            vec![0u8; 1234],
        )
        .unwrap();

        let info = models_impl(&state);
        let v2 = info.iter().find(|m| m.id == PARAKEET_V2_EN.id).unwrap();
        assert_eq!(v2.status, "partial");
        assert_eq!(v2.downloaded, 1234);
        assert_eq!(v2.size_bytes, PARAKEET_V2_EN.total_size());
        let v3 = info.iter().find(|m| m.id != PARAKEET_V2_EN.id).unwrap();
        assert_eq!((v3.status, v3.downloaded), ("missing", 0));
    }

    #[test]
    fn model_info_serialises_camel_case() {
        let (_dir, state) = state();
        let json = serde_json::to_value(models_impl(&state)).unwrap();
        let first = &json[0];
        assert!(first.get("sizeBytes").is_some());
        assert_eq!(first["status"], "missing");
    }

    #[test]
    fn second_download_of_same_model_is_refused_until_the_first_ends() {
        let (_dir, state) = state();
        let token = register_download(&state, PARAKEET_V2_EN.id).unwrap();
        assert_eq!(
            register_download(&state, PARAKEET_V2_EN.id).err().unwrap(),
            "already-downloading"
        );
        // A different model is independent.
        assert!(register_download(&state, PARAKEET_V3_MULTI.id).is_ok());

        // Cancel reaches the running download's token.
        cancel_download_impl(&state, PARAKEET_V2_EN.id);
        assert!(token.is_cancelled());

        end_download(&state, PARAKEET_V2_EN.id);
        assert!(register_download(&state, PARAKEET_V2_EN.id).is_ok());
    }

    #[test]
    fn download_of_unknown_model_is_rejected() {
        let (_dir, state) = state();
        assert_eq!(
            download_impl(&state, "nope", |_| {}).unwrap_err(),
            "unknown-model"
        );
    }

    #[test]
    fn delete_forgets_the_loaded_model_only_when_it_matches() {
        let (dir, state) = state();
        *state.loaded.lock().unwrap() = Some(PARAKEET_V2_EN.id.into());
        std::fs::create_dir_all(dir.path().join(PARAKEET_V2_EN.id)).unwrap();

        delete_model_impl(&state, PARAKEET_V3_MULTI.id).unwrap();
        assert_eq!(
            state.loaded.lock().unwrap().as_deref(),
            Some(PARAKEET_V2_EN.id)
        );

        delete_model_impl(&state, PARAKEET_V2_EN.id).unwrap();
        assert!(state.loaded.lock().unwrap().is_none());
        assert!(!dir.path().join(PARAKEET_V2_EN.id).exists());
        assert_eq!(
            delete_model_impl(&state, "nope").unwrap_err(),
            "unknown-model"
        );
    }

    #[test]
    fn throttle_limits_progress_but_never_drops_the_final_one() {
        let start = Instant::now();
        let mut t = ProgressThrottle::new(start);
        assert!(!t.allow(start + Duration::from_millis(10), 10, 100));
        assert!(t.allow(start + Duration::from_millis(100), 20, 100));
        assert!(!t.allow(start + Duration::from_millis(150), 30, 100));
        assert!(t.allow(start + Duration::from_millis(210), 40, 100));
        // Inside the window, but complete.
        assert!(t.allow(start + Duration::from_millis(211), 100, 100));
    }

    #[test]
    fn download_events_use_camel_case_keys() {
        let progress = serde_json::to_value(DownloadEvent::Progress {
            model_id: "m".into(),
            downloaded: 1,
            total: 2,
        })
        .unwrap();
        assert_eq!(
            progress,
            serde_json::json!({"modelId":"m","downloaded":1,"total":2})
        );
        let done = serde_json::to_value(DownloadEvent::Done {
            model_id: "m".into(),
            done: true,
        })
        .unwrap();
        assert_eq!(done, serde_json::json!({"modelId":"m","done":true}));
        let err = serde_json::to_value(DownloadEvent::Failed {
            model_id: "m".into(),
            error: "x".into(),
        })
        .unwrap();
        assert_eq!(err, serde_json::json!({"modelId":"m","error":"x"}));
    }

    static UNREACHABLE: ModelSpec = ModelSpec {
        id: "unreachable-test-model",
        label: "Unreachable",
        base_urls: &["http://127.0.0.1:1/"],
        files: &[ModelFile {
            name: "model.bin",
            url_path: "model.bin",
            size: 10,
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
        }],
    };

    #[test]
    fn failed_download_reports_error_and_releases_the_token() {
        let (_dir, state) = state();
        let events = std::cell::RefCell::new(Vec::new());
        let result = download_spec(&state, &UNREACHABLE, |e| events.borrow_mut().push(e));
        let error = result.unwrap_err();

        let events = events.into_inner();
        let last = serde_json::to_value(events.last().unwrap()).unwrap();
        assert_eq!(
            last,
            serde_json::json!({"modelId": UNREACHABLE.id, "error": error})
        );
        assert!(lock(&state.cancels).is_empty());
        // A retry is accepted rather than refused as already-downloading.
        let retry = download_spec(&state, &UNREACHABLE, |_| {}).unwrap_err();
        assert_ne!(retry, "already-downloading");
    }

    #[test]
    fn download_guard_releases_the_token_on_panic() {
        let (_dir, state) = state();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_download(&state, PARAKEET_V2_EN.id).ok().unwrap();
            let _guard = DownloadGuard {
                state: &state,
                model_id: PARAKEET_V2_EN.id,
            };
            panic!("download thread died");
        }));
        assert!(outcome.is_err());
        assert!(register_download(&state, PARAKEET_V2_EN.id).is_ok());
    }

    /// Marks the model as fully downloaded without writing its real bytes.
    fn make_ready(dir: &std::path::Path, spec: &ModelSpec) {
        let model_dir = dir.join(spec.id);
        std::fs::create_dir_all(&model_dir).unwrap();
        for file in spec.files {
            let f = std::fs::File::create(model_dir.join(file.name)).unwrap();
            f.set_len(file.size).unwrap();
            std::fs::File::create(model_dir.join(format!("{}.verified", file.name))).unwrap();
        }
    }

    /// A state whose helper requests are recorded, with `spec` downloaded.
    fn fake_helper(spec: &ModelSpec) -> (tempfile::TempDir, State) {
        let (dir, state) = state();
        make_ready(dir.path(), spec);
        *lock(&state.sent) = Some(Vec::new());
        (dir, state)
    }

    fn sent(state: &State) -> Vec<Request> {
        lock(&state.sent).clone().unwrap()
    }

    fn is_load(req: &Request) -> bool {
        matches!(req, Request::Load { .. })
    }

    fn error(kind: ErrorKind) -> Event {
        Event::Error {
            kind,
            message: "x".into(),
        }
    }

    #[test]
    fn a_failed_load_makes_the_next_start_load_again() {
        for kind in [ErrorKind::ModelLoad, ErrorKind::ModelMissing] {
            let (_dir, state) = fake_helper(&PARAKEET_V2_EN);
            start_impl(
                &state,
                &noop_sink(),
                who("a", 1),
                start_args(PARAKEET_V2_EN.id),
            )
            .unwrap();
            on_helper_event(&state, error(kind.clone()), &|_, _| {});
            start_impl(
                &state,
                &noop_sink(),
                who("a", 1),
                start_args(PARAKEET_V2_EN.id),
            )
            .unwrap();
            let sent = sent(&state);
            assert_eq!(sent.len(), 4, "{kind:?}: {sent:?}");
            assert!(is_load(&sent[0]) && is_load(&sent[2]), "{kind:?}: {sent:?}");
            assert!(matches!(sent[3], Request::Start { .. }));
        }
    }

    #[test]
    fn other_errors_keep_the_loaded_model() {
        let (_dir, state) = fake_helper(&PARAKEET_V2_EN);
        start_impl(
            &state,
            &noop_sink(),
            who("a", 1),
            start_args(PARAKEET_V2_EN.id),
        )
        .unwrap();
        on_helper_event(&state, error(ErrorKind::NoInputDevice), &|_, _| {});
        start_impl(
            &state,
            &noop_sink(),
            who("a", 1),
            start_args(PARAKEET_V2_EN.id),
        )
        .unwrap();
        assert_eq!(sent(&state).iter().filter(|r| is_load(r)).count(), 1);
    }

    /// Records what each window is sent.
    #[derive(Default)]
    struct Windows(std::cell::RefCell<Vec<(String, Event)>>);

    impl Windows {
        fn route(&self, state: &State, event: Event) {
            on_helper_event(state, event, &|window, event| {
                self.0.borrow_mut().push((window.into(), event))
            });
        }

        fn take(&self) -> Vec<(String, Event)> {
            std::mem::take(&mut *self.0.borrow_mut())
        }
    }

    fn partial(text: &str) -> Event {
        Event::Partial { text: text.into() }
    }

    fn final_text(text: &str) -> Event {
        Event::Final {
            text: text.into(),
            capped: false,
        }
    }

    fn start(state: &State, window: &str, id: u64) -> Option<Attempt> {
        start_impl(
            state,
            &noop_sink(),
            who(window, id),
            start_args(PARAKEET_V2_EN.id),
        )
        .unwrap()
    }

    #[test]
    fn a_start_from_another_window_cancels_the_owner_first() {
        let (_dir, state) = fake_helper(&PARAKEET_V2_EN);
        let windows = Windows::default();
        assert_eq!(start(&state, "a", 1), None);
        windows.route(&state, Event::Listening);

        assert_eq!(start(&state, "b", 1), Some(who("a", 1)));
        let sent = sent(&state);
        assert!(is_load(&sent[0]), "{sent:?}");
        assert!(
            matches!(
                sent[1..],
                [
                    Request::Start { .. },
                    Request::Cancel,
                    Request::Start { .. }
                ]
            ),
            "{sent:?}"
        );
        // The model was already loaded for the first take.
        assert_eq!(sent.iter().filter(|r| is_load(r)).count(), 1);
    }

    #[test]
    fn a_reloaded_window_reusing_its_attempt_id_still_cancels_first() {
        let (_dir, state) = fake_helper(&PARAKEET_V2_EN);
        start(&state, "a", 1);
        // Its old take is not reported as taken over: it is the same attempt.
        assert_eq!(start(&state, "a", 1), None);
        assert!(matches!(
            sent(&state)[1..],
            [
                Request::Start { .. },
                Request::Cancel,
                Request::Start { .. }
            ]
        ));
    }

    #[test]
    fn helper_events_reach_only_the_owning_window() {
        let (_dir, state) = fake_helper(&PARAKEET_V2_EN);
        let windows = Windows::default();
        start(&state, "a", 1);
        windows.route(&state, Event::Listening);
        assert_eq!(windows.take(), vec![("a".into(), Event::Listening)]);

        start(&state, "b", 7);
        for event in [Event::Listening, partial("hel"), final_text("hello")] {
            windows.route(&state, event);
        }
        assert_eq!(
            windows.take(),
            vec![
                ("b".into(), Event::Listening),
                ("b".into(), partial("hel")),
                ("b".into(), final_text("hello")),
            ]
        );
        // The final ended the take, so nothing owns what follows.
        windows.route(&state, partial("late"));
        assert!(windows.take().is_empty());
    }

    #[test]
    fn transcripts_wait_for_this_attempts_listening() {
        let (_dir, state) = fake_helper(&PARAKEET_V2_EN);
        let windows = Windows::default();
        start(&state, "a", 1);
        windows.route(&state, Event::Listening);
        cancel_impl(&state, &who("a", 1));
        start(&state, "a", 2);
        // Still in flight from the cancelled take.
        windows.route(&state, partial("old"));
        windows.route(&state, final_text("old"));
        assert_eq!(windows.take(), vec![("a".into(), Event::Listening)]);

        windows.route(&state, Event::Listening);
        windows.route(&state, final_text("new"));
        assert_eq!(
            windows.take(),
            vec![
                ("a".into(), Event::Listening),
                ("a".into(), final_text("new"))
            ]
        );
    }

    #[test]
    fn an_error_before_listening_reaches_the_owner_and_ends_its_take() {
        let (_dir, state) = fake_helper(&PARAKEET_V2_EN);
        let windows = Windows::default();
        start(&state, "a", 1);
        windows.route(&state, error(ErrorKind::NoInputDevice));
        assert_eq!(
            windows.take(),
            vec![("a".into(), error(ErrorKind::NoInputDevice))]
        );
        windows.route(&state, error(ErrorKind::Engine));
        assert!(windows.take().is_empty());
    }

    #[test]
    fn an_unowned_listening_is_cancelled() {
        let (_dir, state) = fake_helper(&PARAKEET_V2_EN);
        let windows = Windows::default();
        windows.route(&state, Event::Listening);
        assert_eq!(sent(&state), vec![Request::Cancel]);
        assert!(windows.take().is_empty());

        // Also once the owner has cancelled and its Listening comes late.
        start(&state, "a", 1);
        cancel_impl(&state, &who("a", 1));
        let before = sent(&state).len();
        windows.route(&state, Event::Listening);
        assert_eq!(sent(&state)[before..], [Request::Cancel]);
        assert!(windows.take().is_empty());
    }

    #[test]
    fn stop_and_cancel_from_an_attempt_that_lost_ownership_do_nothing() {
        let (_dir, state) = fake_helper(&PARAKEET_V2_EN);
        start(&state, "a", 1);
        start(&state, "b", 1);
        let before = sent(&state).len();
        stop_impl(&state, &who("a", 1));
        cancel_impl(&state, &who("a", 1));
        assert_eq!(sent(&state).len(), before, "{:?}", sent(&state));

        stop_impl(&state, &who("b", 1));
        cancel_impl(&state, &who("b", 1));
        assert_eq!(sent(&state)[before..], [Request::Stop, Request::Cancel]);
    }

    #[test]
    fn start_against_a_given_up_client_reports_speech_unavailable() {
        let (dir, state) = state();
        make_ready(dir.path(), &PARAKEET_V2_EN);
        // A helper that cannot be launched exhausts its restarts quickly.
        let client = SpeechClient::spawn_with(
            dir.path().join("no-such-helper"),
            Vec::new(),
            Vec::new(),
            RestartPolicy {
                delays: vec![Duration::from_millis(5); 3],
                window: Duration::from_secs(60),
                max_restarts: 3,
            },
            |_| {},
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !client.is_unavailable() {
            assert!(Instant::now() < deadline, "client never gave up");
            std::thread::sleep(Duration::from_millis(5));
        }
        *lock(&state.client) = Some(client);

        let err = start_impl(
            &state,
            &noop_sink(),
            who("a", 1),
            start_args(PARAKEET_V2_EN.id),
        )
        .unwrap_err();
        assert_eq!(err, "speech-unavailable");
        // Nothing was sent, so the model is not recorded as loaded.
        assert!(lock(&state.loaded).is_none());
        shutdown(&state);
    }
}
