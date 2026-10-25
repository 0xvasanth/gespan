# Desktop Dictation (Voice, desktop half of #50) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Push-to-talk dictation into the session composer of both Gespan Native (GPUI) and the Tauri app, using a local Parakeet model. Audio never leaves the machine.

**Architecture:** There are two new shared crates under `crates/`.
- `gespan-models` downloads and verifies large model files.
- `gespan-speech` holds the vocabulary rules, the JSON-lines helper protocol, the sherpa-onnx Parakeet engine, the microphone capture and a supervisor.

Each app runs its own binary again with a `speech-helper` subcommand. That child process owns the microphone and the model, so a crash in native speech code can't take the app down. The apps talk to it over stdio JSON lines. Live captions show in a bar above the composer, and the final punctuated text is inserted at the cursor when dictation stops.

**Tech Stack:** Rust (`sherpa-onnx` 2.0.0 static, `cpal` 0.18, `ureq` 2.12, `sha2` 0.11), Tauri 2 + React + Vitest, GPUI (`gpui-pre` 0.3.9).

**Spec:** GitHub epic #50 (body) plus sub-issues #121, #113, #115, #116, #117 and #118. The decisions below are mine, made because the owner said "plan and start"; they are flagged so they can be overridden.

## Decisions (my calls, flagged for review)

1. **Scope:** desktop only. The iPhone tickets (#110, #111, #112, #114 and #119) need real devices and stay open. #113 ships only its desktop half here: term picking and post-processing. The `mobile-link` message waits for the phone work.
2. **Engine:** sherpa-onnx on every platform. `fluidaudio-rs` (Apple Neural Engine) is left for later behind the same `SpeechEngine` trait. It is at version 0.x and needs a Swift toolchain in CI.
3. **Models:**
   - The default is `parakeet-tdt-0.6b-v2` int8 (English, about 660 MB).
   - `-v3` int8 is the "Multilingual" choice.
   - Both are fetched as individual files from Hugging Face at a pinned revision. The mirror list starts empty because no mirror exists yet.
   - There is no Nemotron streaming model and no VAD model in v1.
4. **Live text:** the helper re-decodes the whole buffered audio once a second and emits `Partial`; `Final` comes from one decode on stop. Recording is capped at **120 s**, after which it auto-stops and produces a final. This keeps the download to a single model.
5. **Vocabulary:** post-processing always runs. Native sherpa hotwords are used only if the spike in Task 4 shows the NeMo TDT model accepts `modified_beam_search` with hotwords. Otherwise the post-processor alone does the job.
6. **Shared code location:** vocabulary code lives in `crates/gespan-speech`, not in `gespan-core` as #113 suggests. `src-tauri` cannot depend on `gespan-core`, and the helper (which both apps run) applies the post-processing.
7. **Model storage:** both apps use one shared folder, so the model is downloaded once. It is `<dirs::data_dir()>/dev.gespan.shared/models/<model-id>/`.
8. **Shortcut:** "Composer: Dictate" uses `cmd-shift-m` on macOS and `ctrl-shift-m` elsewhere, in the composer context. Both are unused today; `cmd-shift-space` is taken by the Quick Composer.
   - A press of under 400 ms toggles dictation: tap to start, tap again to stop.
   - Holding longer is push-to-talk: releasing stops it.
   - The mic button in the composer toggles on click.
9. **Insertion:** the final text goes in at the composer's cursor or selection *at stop time*, so text typed during dictation is kept. A single space is added before it when the character before the cursor is not whitespace and the text does not start with `.,;:!?)`.
10. **PRs:** one per sub-issue, in this order: A #121, B #113, C #115, D #116, E #117. Each branch is cut off the previous one until that one merges. #118's documents go in C; its attribution UI goes in D and E; owner sign-off is still needed before release.

## Global Constraints

- Product name is **Gespan** only (`src/test/legacyNames.test.ts` fails the build on old names). Crate names are `gespan-models` and `gespan-speech`.
- UI tooling uses Bun (`bun install`, `bun run`, `bunx`); tests run through Vitest, not `bun test`. Rust uses `cargo`.
- New crates go in the root `Cargo.toml` `[workspace] members` and are added as `path =` dependencies from `src-tauri`, `crates/gespan-core` (if needed) and `crates/gespan-gpui`. Lock files are updated by running cargo (`cargo update -p <crate>`), never by hand. The Native check uses `--locked`, so commit `crates/gespan-gpui/Cargo.lock`.
- `sherpa-onnx`'s build script downloads prebuilt static libraries from GitHub at build time. The CI jobs `check` (ci.yml) and `core` and `desktop` (native.yml) must pass on every OS they run.
- No audio or transcript is written to disk or logs. Tracing records only lengths, durations and error kinds.
- No model is downloaded without an explicit click. The download prompt states the size: "Download the speech model (about 660 MB)?"
- Hotspot files take only additive one-line registrations: `src-tauri/src/lib.rs` (`mod` and `invoke_handler`), `src/app/App.tsx`, `SettingsView.tsx`, `settings/model.rs` `Section`, and the `mod` list in `gespan-gpui/src/main.rs`.
- Before each PR, run `bun run check` (for PRs A–D) and/or `bun run native:check` (for A–C and E), and get all 9 required CI checks green. Commits carry no Claude or co-author attribution.

## Review Focus

1. **Microphone denied or no input device:** a clear error ("Gespan can't use the microphone. Allow it in System Settings → Privacy & Security → Microphone.") and the button goes back to idle. Tests: Task 5 `no_input_device_reports_error`, Task 7 `start_error_returns_to_idle`.
2. **User types or moves the caret while dictating:** the final text goes in at the caret at stop time, and the typed text is kept. Tests: Task 8 `final_inserts_at_caret_at_stop_time`, Task 9 `final_text_goes_to_current_cursor`.
3. **Pressing the mic before the model is ready:** this shows the download prompt instead of starting. An interrupted download resumes, and a corrupt file is fetched again. Tests: Task 1 `resumes_from_partial`, `corrupt_file_redownloads`; Task 7 `start_without_model_prompts_download`.
4. **Very long dictation:** at 120 s the helper sends `Final` with `capped: true` without being asked. Memory stays bounded at 120 s × 16 kHz. Test: Task 5 `caps_recording_at_limit`.
5. **Leaving the session or closing the composer mid-dictation:** dictation is cancelled and nothing is inserted into another session. Tests: Task 8 `unmount_cancels_without_insert`, Task 9 `switching_session_cancels_dictation`.

---

## PR A — #121 `gespan-models` (branch `voice/121-models`)

### Task 1: Model download manager

**Files:**
- Create: `crates/gespan-models/Cargo.toml`, `crates/gespan-models/src/lib.rs`, `crates/gespan-models/src/download.rs`, `crates/gespan-models/tests/download.rs`, `crates/gespan-models/tests/support/mod.rs` (a tiny `std::net::TcpListener` HTTP server that honours `Range` and can cut a body short)
- Modify: `Cargo.toml` (root `members`), `crates/gespan-gpui/Cargo.toml` (path dependency, for later), the lock files via cargo

**Interfaces:**
- Produces:
  - `pub struct ModelFile { pub name: &'static str, pub url_path: &'static str, pub size: u64, pub sha256: &'static str }`
  - `pub struct ModelSpec { pub id: &'static str, pub label: &'static str, pub base_urls: &'static [&'static str], pub files: &'static [ModelFile] }`. `base_urls[0]` is the primary source and the rest are mirrors, tried in order. The URL is `base + url_path`.
  - `pub fn default_root() -> PathBuf`, which returns `dirs::data_dir()/dev.gespan.shared/models`
  - `pub struct ModelStore { root: PathBuf }`, with:
    - `ModelStore::new(root)`
    - `status(&self, &ModelSpec) -> ModelStatus`
    - `dir(&self, &ModelSpec) -> PathBuf`
    - `delete(&self, &ModelSpec) -> io::Result<()>`
    - `download(&self, &ModelSpec, &CancelToken, impl FnMut(Progress)) -> Result<(), DownloadError>`
  - `pub enum ModelStatus { Missing, Partial { downloaded: u64, total: u64 }, Ready }`. `Ready` means every file exists with the expected size **and** a `.verified` marker that was written after its SHA-256 matched.
  - `pub struct Progress { pub downloaded: u64, pub total: u64 }`
  - `#[derive(Clone)] pub struct CancelToken(Arc<AtomicBool>)`, with `new()`, `cancel()` and `is_cancelled()`
  - `pub enum DownloadError { Cancelled, Network(String), Checksum { file: String }, Io(io::Error) }`, using thiserror
- Approach:
  - Each file streams into `<name>.part`, resuming with `Range: bytes=<len>-`.
  - It is hashed after completion, then renamed into place.
  - A checksum mismatch deletes the `.part` file and retries from zero once, then returns `Checksum`.
  - Cancelling deletes the `.part` file of the current file.
  - A network error moves on to the next base URL and keeps the `.part` file.

- [ ] **Step 1:** Write the failing tests in `tests/download.rs`:
  - `downloads_and_verifies`: two files; status goes `Missing` → `Ready`; progress is monotonic and ends at the total.
  - `resumes_from_partial`: the server cuts the first response at 40%; a second `download` sends `Range` (the server records the header) and ends `Ready`.
  - `corrupt_file_redownloads`: the server serves wrong bytes once and then correct ones; ends `Ready`, and the server saw 2 full requests.
  - `persistent_checksum_mismatch_errors`: always wrong; `Err(Checksum { file })` and no `.part` left.
  - `cancel_leaves_no_partial_files`: cancel from the progress callback after the first chunk; `Err(Cancelled)` and no `*.part` under `dir()`.
  - `falls_back_to_mirror`: the first base URL refuses the connection and the second serves; ends `Ready`.
  - `truncated_final_file_is_not_ready`: a file of the right name but wrong size gives `Partial`.
  - `delete_removes_model_dir`.
- [ ] **Step 2:** `cargo test -p gespan-models` fails (the crate is empty).
- [ ] **Step 3:** Implement `lib.rs` and `download.rs` as above. Use `ureq` 2.12 (`default-features = false, features = ["tls"]`), `sha2` 0.11, `dirs` and `thiserror`, with 1 MiB read chunks.
- [ ] **Step 4:** `cargo test -p gespan-models` passes, and `cargo clippy -p gespan-models --all-targets -- -D warnings` is clean.
- [ ] **Step 5:** Commit `Models: shared download manager with resume, SHA-256 checks and mirrors (#121)`. Open PR A with `Closes #121`.

---

## PR B — #113 desktop half (branch `voice/113-vocabulary`)

### Task 2: `gespan-speech` crate with vocabulary and post-processing

**Files:**
- Create: `crates/gespan-speech/Cargo.toml`, `src/lib.rs`, `src/vocabulary.rs`, `src/postprocess.rs`, `tests/identifiers.rs`, `tests/fixtures/identifier_phrases.json`
- Modify: root `Cargo.toml` members

**Interfaces:**
- Produces:
  - `pub const MAX_TERMS: usize = 230;` and `pub const MIN_TERM_LEN: usize = 4;`
  - `pub struct VocabularySources<'a> { pub project_root: Option<&'a Path>, pub branches: &'a [String], pub known_names: &'a [String], pub user_words: &'a [String] }`
  - `pub fn local_branches(project_root: &Path) -> Vec<String>`, from `git for-each-ref --format=%(refname:short) refs/heads`. It returns an empty list when git fails.
  - `pub fn pick_terms(src: &VocabularySources) -> Vec<String>`. It collects:
    - user words, which come first and are always kept;
    - the top-level folder names under `project_root`;
    - the names of up to 2,000 tracked files from `git ls-files` (the file stem, plus the extension as written);
    - the branch names, split on `/`;
    - the known provider and model names.

    It drops anything shorter than 4 characters or purely numeric, dedupes case-insensitively (keeping the first casing seen) and truncates to 230.
  - `pub fn postprocess(text: &str, terms: &[String]) -> String`, which applies the rules in order:
    1. The spoken-symbol table: "dot <ext>" becomes `.<ext>` when `<ext>` is a known extension or term suffix; "underscore" becomes `_`; "dash dash" becomes `--`; "slash" between words becomes `/`.
    2. Joining a run of 2–4 words whose lowercase concatenation equals the lowercase form of a term. The term's own casing is used: "use inbox unseen" with `useInboxUnseen` gives `useInboxUnseen`, and "ru sqlite" with `rusqlite` gives `rusqlite`.
    3. A word that equals a term case-insensitively takes the term's casing.

    Punctuation attached to words is kept.

- [ ] **Step 1:** Write the fixture file of at least 25 phrase → expected pairs plus their terms. Include the epic's examples (`useInboxUnseen`, `rusqlite`, `worktree`, "main dot rs" → `main.rs`) and negative cases: "the use of inbox" must not change, and a sentence-ending period is left alone. Write the tests:
  - `identifier_phrases_fixture`, which runs the fixture through `postprocess`;
  - `pick_terms_filters_short_and_dedupes`;
  - `pick_terms_caps_at_230_keeping_user_words`;
  - `pick_terms_reads_top_folders_and_git_files`, using a tempdir with `git init` and two committed files.
- [ ] **Step 2:** `cargo test -p gespan-speech` fails.
- [ ] **Step 3:** Implement `vocabulary.rs` and `postprocess.rs`. The crate has no sherpa dependency yet.
- [ ] **Step 4:** The tests pass and clippy is clean.
- [ ] **Step 5:** Commit `Voice: project vocabulary and spoken-identifier post-processing (#113, desktop)`. Open PR B with `Part of #113`; the link message is still to come.

---

## PR C — #115 engine and helper (branch `voice/115-speech-engine`)

### Task 3: Helper protocol

**Files:**
- Create: `crates/gespan-speech/src/protocol.rs`, `tests/protocol.rs`

**Interfaces:**
- Produces:
  - `#[serde(tag = "type", rename_all = "snake_case")] pub enum Request`:
    - `Load { model_dir: PathBuf, model_id: String }`
    - `Start { terms: Vec<String> }`
    - `Stop`
    - `Cancel`
    - `Shutdown`
  - `#[serde(tag = "type", rename_all = "snake_case")] pub enum Event`:
    - `Ready`
    - `Loaded`
    - `Listening`
    - `Level { rms: f32 }` (at most 20 per second)
    - `Partial { text: String }`
    - `Final { text: String, capped: bool }`
    - `Cancelled`
    - `Error { kind: ErrorKind, message: String }`
  - `pub enum ErrorKind { MicrophoneDenied, NoInputDevice, ModelMissing, ModelLoad, Engine, Busy }`
  - `pub fn write_line<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()>` and `pub fn read_line<T: DeserializeOwned>(r: &mut impl BufRead) -> io::Result<Option<T>>`. `Ok(None)` means end of file; a malformed line returns `InvalidData`.
- [ ] **Step 1:** Write the tests: `every_request_round_trips`, `every_event_round_trips`, `malformed_line_is_invalid_data`, `eof_is_none`.
- [ ] **Step 2:** They fail. **Step 3:** Implement. **Step 4:** They pass. **Step 5:** Commit `Voice: helper protocol`.

### Task 4: Parakeet engine (sherpa-onnx) and model catalog, with a spike

**Files:**
- Create: `crates/gespan-speech/src/engine.rs`, `src/parakeet.rs`, `src/catalog.rs`, `tests/wer.rs`, `tests/clips/README.md`, `docs/licenses/speech-models.md`
- Modify: `crates/gespan-speech/Cargo.toml` (adds `sherpa-onnx = "2.0.0"` and `gespan-models`)

**Interfaces:**
- Produces:
  - `pub trait SpeechEngine: Send { fn transcribe(&mut self, samples_16k: &[f32], terms: &[String]) -> Result<String, EngineError>; }`
  - `pub struct ParakeetEngine`, with `ParakeetEngine::load(model_dir: &Path) -> Result<Self, EngineError>`. It uses `OfflineRecognizer` with the transducer files `encoder.int8.onnx`, `decoder.int8.onnx`, `joiner.int8.onnx` and `tokens.txt`, `model_type = "nemo_transducer"`, and `num_threads = min(4, available_parallelism)`.
  - `catalog::PARAKEET_V2_EN: ModelSpec` and `catalog::PARAKEET_V3_MULTI: ModelSpec`. Their sources are `https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-{v2,v3}-int8/resolve/<pinned commit>/`. Pin the commit from the HF API (`sha` field; v2 = `1ab9323565ddb038682214b292f588070a538ce2`) and record each file's size and LFS `oid` (its SHA-256). `tokens.txt` is not stored in LFS, so hash it locally after downloading.
  - `catalog::ALL: &[&ModelSpec]`
- [ ] **Step 1 (spike, no tests):**
  1. Download v2 using `gespan-models` into a scratch folder.
  2. Transcribe the repo's `test_wavs/*.wav`, and time it on an M-series Mac.
  3. Try `decoding_method = "modified_beam_search"` with `create_stream_with_hotwords`.
  4. Write the outcome into the "Hotwords" section of `docs/licenses/speech-models.md`: whether hotwords are supported, and the speed in multiples of real time.

  If hotwords fail, `transcribe` ignores `terms`, which are applied later by `postprocess` (decision 5).
- [ ] **Step 2:** Write `tests/wer.rs::parakeet_clips_meet_wer`, marked `#[ignore]`. It runs when `GESPAN_SPEECH_MODEL_DIR` is set, reads `tests/clips/*.wav` with `*.txt` references and asserts that the mean WER is at most **0.10** after `postprocess`.
  - The clips are the model repo's test wavs (CC-BY-4.0, attributed in `tests/clips/README.md`).
  - The owner adds at least 8 self-recorded code-term clips: an owner-assisted step, noted in the PR.

  Also write `catalog_specs_are_consistent`, which checks unique ids and that every file has a 64-hex-character sha256 and a size above zero.
- [ ] **Step 3:** Implement the engine and catalog. Write `docs/licenses/speech-models.md`, covering:
  - v2 and v3 under CC-BY-4.0, with attribution text: "Speech recognition uses NVIDIA Parakeet TDT 0.6B (CC-BY-4.0), run with sherpa-onnx (Apache-2.0)";
  - EOU, Nemotron and fluidaudio as not shipped;
  - an owner sign-off checkbox for #118.
- [ ] **Step 4:** `cargo test -p gespan-speech` passes. `GESPAN_SPEECH_MODEL_DIR=… cargo test -p gespan-speech -- --ignored` passes locally, with the WER value written into the PR.
- [ ] **Step 5:** Commit `Voice: Parakeet engine on sherpa-onnx and model catalog (#115)`.

### Task 5: Helper runtime (microphone, buffering, partials and finals)

**Files:**
- Create: `crates/gespan-speech/src/helper.rs`, `src/capture.rs`, `tests/helper.rs`

**Interfaces:**
- Consumes: Task 3's `Request` and `Event`, Task 4's `SpeechEngine` and `ParakeetEngine::load`, Task 2's `postprocess`.
- Produces:
  - `pub trait AudioSource: Send { fn start(&mut self, sink: Box<dyn FnMut(&[f32]) + Send>) -> Result<(), CaptureError>; fn stop(&mut self); }`. Samples handed to the sink are already 16 kHz mono f32.
  - `pub struct MicSource`, which implements it with `cpal` 0.18: the default input device, downmixed to mono and linearly resampled to 16 kHz. It maps a missing device to `CaptureError::NoInputDevice` and a permission or stream-build failure to `CaptureError::Denied`.
  - `pub const MAX_RECORDING_SECS: u32 = 120;` and `pub const PARTIAL_INTERVAL_MS: u64 = 1000;`
  - `pub fn run_helper(input: impl BufRead, output: impl Write + Send + 'static, make_engine: impl Fn(&Path) -> Result<Box<dyn SpeechEngine>, EngineError>, source: Box<dyn AudioSource>) -> io::Result<()>`
  - `pub fn helper_main() -> !`, which runs `run_helper` on stdin and stdout with `ParakeetEngine` and `MicSource`, and exits 0 on `Shutdown` or end of input. This is what the apps' `speech-helper` subcommand calls.
- Behaviour:
  - `Ready` goes out at startup. `Load` gives `Loaded` or an `Error`, either `ModelMissing` or `ModelLoad`.
  - `Start` without a loaded model gives `Error ModelMissing`. `Start` while already listening gives `Error Busy`.
  - While listening, a worker thread re-decodes every 1,000 ms, but only when the buffer has grown, and sends `Partial` without post-processing.
  - `Stop` produces the `Final`: one decode, then `postprocess(text, terms)`, then trimmed. An empty result is still sent as a `Final` with empty text.
  - Hitting 120 s sends `Final { capped: true }` without being asked.
  - `Cancel` clears the buffer and sends `Cancelled`.
- [ ] **Step 1:** Write the tests, using a `FakeSource` that pushes scripted sample chunks and a `FakeEngine` that returns `"<n> samples"`, or `"use inbox unseen"` for the postprocess test. Pipe the requests and events through in-memory pipes.
  - `start_before_load_is_model_missing`
  - `stop_emits_postprocessed_final` (expects `useInboxUnseen` with terms `["useInboxUnseen"]`)
  - `emits_partials_while_listening`
  - `caps_recording_at_limit` (the limit is injectable through a `#[cfg(test)]` constructor param, or `run_helper_with(limits)`; the test uses 1 s of samples against a 0.5 s cap and expects `capped: true` and buffer length ≤ cap)
  - `cancel_discards_audio`
  - `second_start_is_busy`
  - `no_input_device_reports_error` (the `FakeSource` returns `NoInputDevice`, so the event is `Error { kind: NoInputDevice }`)
  - `eof_exits_cleanly`
- [ ] **Step 2:** They fail. **Step 3:** Implement `helper.rs` and `capture.rs`. **Step 4:** They pass.
- [ ] **Step 5:** Commit `Voice: speech helper with microphone capture, partials and capped finals (#115)`.

### Task 6: Supervisor client (used by both apps)

**Files:**
- Create: `crates/gespan-speech/src/client.rs`, `crates/gespan-speech/src/bin/fake-speech-helper.rs` (test-only behaviour, chosen by an env var: `echo`, `crash-after-start` or `hang`), `tests/client.rs`

**Interfaces:**
- Produces:
  - `pub struct SpeechClient`
  - `SpeechClient::spawn(program: PathBuf, args: Vec<String>, on_event: impl Fn(Event) + Send + Sync + 'static) -> SpeechClient`
  - `send(&self, Request)`: queued until the helper reports `Ready`, and a no-op after shutdown
  - `shutdown(self)`: sends `Shutdown`, waits 2 s, then kills
  - On an unexpected exit:
    - it emits `Event::Error { kind: Engine, message: "Dictation stopped unexpectedly." }` if it was listening;
    - it restarts with backoff of 0.5 s, 2 s and 5 s, at most 3 restarts in 60 s, after which it stops and emits `Error { kind: Engine, message: "Dictation is unavailable. Restart Gespan to try again." }`;
    - after a restart it re-sends the last `Load`.
  - `pub fn helper_command() -> (PathBuf, Vec<String>)`, which returns `(current_exe, ["speech-helper"])`
- [ ] **Step 1:** Write the tests, using `env!("CARGO_BIN_EXE_fake-speech-helper")`:
  - `forwards_events_in_order`
  - `restarts_after_crash_and_reloads_model` (the second process receives `Load`)
  - `gives_up_after_three_crashes_in_a_minute`
  - `crash_while_listening_reports_error`
  - `shutdown_kills_hung_helper_within_3s`
- [ ] **Step 2:** They fail. **Step 3:** Implement. **Step 4:** They pass, and `bun run check` plus `bun run native:check` are green with the crate wired into neither app yet.
- [ ] **Step 5:** Commit `Voice: supervised speech helper client with restart backoff (#115)`. Open PR C with `Closes #115`, and note the WER result plus the owner-assisted clip recording.

---

## PR D — #116 Tauri (branch `voice/116-tauri-dictation`)

### Task 7: Tauri backend commands

**Files:**
- Create: `src-tauri/src/speech.rs`
- Modify:
  - `src-tauri/src/main.rs`: a `speech-helper` arm that calls `gespan_speech::helper_main()`, next to `control`
  - `src-tauri/src/lib.rs`: `mod speech;`, `.manage(speech::State::default())`, and the commands added to `invoke_handler`
  - `src-tauri/Cargo.toml`: path dependencies on `gespan-speech` and `gespan-models`
  - `src-tauri/Info.plist`: `NSMicrophoneUsageDescription` = "Gespan uses the microphone when you dictate a message. Audio stays on this device."
  - `src-tauri/Entitlements.plist`: `com.apple.security.device.audio-input` = true. Native's bundle script reuses this file.

**Interfaces:**
- Produces Tauri commands, all `async` and returning `Result<_, String>`:
  - `dictation_models() -> Vec<ModelInfo { id, label, size_bytes, status: "missing" | "partial" | "ready", downloaded }>`
  - `dictation_download(model_id)`: progress arrives as `dictation-download` events `{ model_id, downloaded, total }` (throttled to 10 per second), ending in `{ model_id, done: true }` or `{ model_id, error }`
  - `dictation_cancel_download(model_id)` and `dictation_delete_model(model_id)`
  - `dictation_start(project_path: Option<String>, model_id, user_words: Vec<String>, known_names: Vec<String>)`: lazily spawns the client and sends `Load` if the model changed. It then computes `pick_terms` from `local_branches(project_path)`, the user words (the `dictation.words` setting) and the provider and model names the frontend already holds, and sends `Start`
  - `dictation_stop()` and `dictation_cancel()`
  - Every helper `Event` is forwarded as the `dictation-event` app event (serde form, as in Task 3)
  - `dictation_start` with a model that isn't `Ready` returns `Err("model-missing")`
- [ ] **Step 1:** Write Rust unit tests in `speech.rs`: `start_without_model_prompts_download` (returns `Err("model-missing")` against a temp `ModelStore`) and `model_info_reports_partial_progress`.
- [ ] **Step 2:** They fail. **Step 3:** Implement with `State { client: Mutex<Option<SpeechClient>>, store: ModelStore, cancels: Mutex<HashMap<String, CancelToken>> }`. **Step 4:** `cargo test -p gespan` (the src-tauri package) passes.
- [ ] **Step 5:** Commit `Voice (Tauri): speech commands, helper subcommand and microphone permission (#116)`.

### Task 8: Tauri composer UI, shortcut and settings

**Files:**
- Create:
  - `src/features/sessions/dictation/insertDictation.ts`
  - `src/features/sessions/dictation/useDictation.ts`
  - `src/features/sessions/ui/DictationBar.tsx`
  - `src/features/settings/ui/DictationSettings.tsx`
  - tests `src/features/sessions/dictation/insertDictation.test.ts` and `useDictation.test.ts`
- Modify:
  - `src/features/sessions/ui/Composer.tsx`: a mic `ToolButton` next to the plus button around :2175, the `DictationBar` rendered above the textarea, and the hook wired to the textarea ref
  - `src/features/settings/model/settings.ts`: a `"dictation"` id in `SettingsSectionId` and `SETTINGS_SECTIONS` (group "app", label "Dictation", keywords `voice, microphone, speech, parakeet`), and a `KEYBINDINGS` row `{ command: "Composer: Dictate", keys: \`${MOD}${SHIFT}M\`, when: "Composer focused" }`
  - `SettingsView.tsx`: one registration line

**Interfaces:**
- Consumes: Task 7's commands and events.
- Produces:
  - `insertDictation(field: HTMLTextAreaElement, text: string): void`. It inserts at the current `selectionStart`/`selectionEnd` with the spacing rule from decision 9, using `setRangeText(..., "end")`, then fires a bubbling `input` event (the same approach as `draftRestore.ts`).
  - `useDictation({ fieldRef, projectPath, sessionId }) -> { state: "idle" | "starting" | "listening" | "finishing" | "needs-model" | "error", partial: string, level: number, error?: string, toggle(), pressStart(), pressEnd(), cancel() }`. `pressStart` and `pressEnd` give the tap-or-hold behaviour, with a threshold of 400 ms.
- The UI:
  - `DictationBar` shows a level meter, the partial text in muted type, "Listening…" or "Finishing…", and Esc to cancel.
  - The `needs-model` state shows "Download the speech model (about 660 MB)?" with Download and Not now buttons, and progress.
  - `DictationSettings` covers: model choice (English or Multilingual), download, cancel and delete with sizes, custom words (one per line, stored in the `dictation.words` setting), the shortcut row, and the attribution text from Task 4.
- [ ] **Step 1:** Write the Vitest tests.
  - `insertDictation.test.ts`:
    - `inserts_at_caret_with_leading_space`
    - `no_space_after_whitespace_or_before_punctuation`
    - `replaces_selection`
    - `fires_input_event`
    - `final_inserts_at_caret_at_stop_time` (move the caret between start and final)
  - `useDictation.test.ts`, with `@tauri-apps/api` `invoke` and `listen` mocked:
    - `tap_toggles_and_hold_releases_stops` (fake timers at 399 ms and 401 ms)
    - `partial_then_final_inserts_once`
    - `start_error_returns_to_idle` (the event is `error` `microphone_denied` and the message shown is the Review Focus text)
    - `start_without_model_prompts_download` (`invoke` rejects with `"model-missing"`, so the state becomes `needs-model`)
    - `unmount_cancels_without_insert`
    - `session_change_cancels`
- [ ] **Step 2:** `bunx vitest run src/features/sessions/dictation` fails.
- [ ] **Step 3:** Implement the code and wire it into the composer and settings.
- [ ] **Step 4:** `bun run check` is green. Do a manual check on a Mac with `bun run tauri dev`:
  1. Download the model.
  2. Dictate "rename use inbox unseen in main dot rs" and get "Rename useInboxUnseen in main.rs".
  3. Deny the microphone in System Settings and see the error message.
- [ ] **Step 5:** Commit `Voice (Tauri): composer mic button, Cmd-Shift-M and Dictation settings (#116)`. Open PR D with `Closes #116`.

---

## PR E — #117 Native (branch `voice/117-native-dictation`)

### Task 9: Native dictation

**Files:**
- Create: `crates/gespan-gpui/src/conversation/dictation.rs`, `crates/gespan-gpui/src/settings/pages/dictation.rs`
- Modify:
  - `crates/gespan-gpui/src/main.rs`: a `speech-helper` arm next to `visual-mcp` (:218), plus a `mod` line if needed
  - `crates/gespan-gpui/src/input.rs`: `Dictate` added to `actions!(composer, …)` (:26), with the binding `KeyBinding::new("cmd-shift-m", Dictate, Some("Composer"))` and `ctrl-shift-m` on non-mac
  - `crates/gespan-gpui/src/conversation.rs`: a mic pill next to `"composer-add"` (around :2867) and a caption row above the input in `fn composer` (:2682)
  - `crates/gespan-gpui/src/settings/model.rs`: `Section::Dictation`, added to `GROUPS`, to `ALL` (13 → 14), and to the `id`, label, description and keyword arms
  - `settings/pages/mod.rs`: a dispatch arm
  - `scripts/bundle-native-macos.sh`: `NSMicrophoneUsageDescription` in the Info.plist heredoc, with the same text as Task 7
  - `crates/gespan-gpui/Cargo.toml`: path dependencies on `gespan-speech` and `gespan-models`
  - `crates/gespan-gpui/Cargo.lock`

**Interfaces:**
- Consumes: `SpeechClient`, `helper_command`, `catalog`, `ModelStore` and `pick_terms` from Tasks 1–6.
- Produces:
  - `pub(crate) struct Dictation { state: DictationState, partial: SharedString, level: f32, client: Option<SpeechClient>, … }`, as a GPUI entity owned by `Conversation`
  - `pub(crate) enum DictationState { Idle, Starting, Listening, Finishing, NeedsModel, Error(SharedString) }`
  - `fn toggle`, `fn press_start` and `fn press_end` (400 ms threshold), and `fn cancel`
  - Helper events come from the client callback into an `async_channel` and are drained on the foreground executor
  - The `Final` goes in through `input.update(cx, |i, cx| i.replace_text_in_range(None, &spaced, window, cx))`, using the spacing rule from decision 9 against `content[..selection.start]`
  - The settings page mirrors Task 8's `DictationSettings`, with the same copy
- [ ] **Step 1:** Write the GPUI tests in `conversation/dictation.rs` `#[cfg(test)]`, using `gpui::TestAppContext` and a client stub. Allow a `Dictation::with_sink(fn(Request))` test constructor, so events can be injected without a process.
  - `final_text_goes_to_current_cursor` (move the selection between `Listening` and `Final`)
  - `leading_space_rule_matches_tauri` (the same cases as Task 8)
  - `switching_session_cancels_dictation` (sends `Cancel`; a later `Final` inserts nothing)
  - `model_missing_shows_download_prompt`
  - `microphone_denied_returns_to_idle_with_message`
  - `cmd_shift_m_tap_toggles_and_hold_stops`, via `cx.simulate_keystrokes` and the test executor's timer
  - `settings_section_count_is_14` (update the existing `ALL` test)
- [ ] **Step 2:** `bun run native:test` fails.
- [ ] **Step 3:** Implement and wire it up.
- [ ] **Step 4:** `bun run native:check` is green. Do a manual check with `bun run native:dev` (from a terminal, so the terminal gets the microphone prompt) and with a `scripts/bundle-native-macos.sh` bundle (where Gespan itself gets it), using the same three checks as Task 8. Confirm one downloaded model serves both apps (decision 7).
- [ ] **Step 5:** Commit `Voice (Native): composer mic, Cmd-Shift-M and Dictation settings (#117)`. Open PR E with `Closes #117`. Comment on #118 linking `docs/licenses/speech-models.md` and asking for owner sign-off.
