# Native Kiro end-to-end check

`run_kiro_e2e.py` drives the real Gespan Native app on macOS through a Kiro
conversation and fails (non-zero exit) if the integration breaks. No model is
involved: Gespan's Kiro provider is pointed at `fake-kiro-cli`, a small ACP
agent that always answers `GESPAN KIRO STUB REPLY: <your prompt>`.

What one run checks:

1. The app starts on a throwaway data directory and project, with Kiro as the
   provider for new sessions.
2. Cmd+N, a prompt, Return; then a follow-up in the same conversation.
3. The stub saw Gespan launch `kiro-cli acp --agent-engine=v3`, then
   `initialize`, one `session/new` in the project, and both prompts.
4. The follow-up reused the same Kiro process (no second start, no
   `session/load`); a cold restart per turn costs several seconds with real Kiro.
5. Gespan saved a `kiro` session with the stub's session id and both replies.
6. With `--ocr`, the reply is also read back from a screenshot of the window.

Every other provider is pointed at `/usr/bin/false`, so a run can never reach a
real model, even if Kiro is broken.

## Requirements

- macOS with Python 3 (`/usr/bin/python3` is fine) and the Xcode command line
  tools (`swiftc`, only for `--ocr`).
- A built app bundle:

  ```bash
  cargo build --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-gpui --bin gespan-gpui --profile native-dev
  ```

  ```bash
  bash scripts/bundle-native-macos.sh native-dev
  ```

- **Accessibility** permission for the app that runs the script (Terminal,
  iTerm, your IDE, or Claude), under System Settings > Privacy & Security >
  Accessibility. The script types with System Events.
- **Screen Recording** permission for the same app, only with `--ocr`.
- Kiro CLI does not need to be installed or signed in.

The script brings the test app to the front before every keystroke, so leave
the machine alone while it runs (about 15 seconds).

## Running

```bash
python3 tests/native-kiro/run_kiro_e2e.py
```

Options: `--ocr` (also check the window text), `--keep` (keep the temp
directory with `app.log` and the stub's `kiro-stub.jsonl` wire trace),
`--app PATH` (another bundle), `--kiro-binary PATH` (another `kiro-cli`).

Exit codes: `0` pass, `1` an assertion failed, `2` the environment is not
ready (missing bundle or permissions).

To see it catch a broken integration, point Kiro at a missing binary; the run
fails because no Kiro reply is ever saved:

```bash
python3 tests/native-kiro/run_kiro_e2e.py --kiro-binary /nonexistent/kiro-cli --timeout 20
```

## Related automated tests

The same Kiro behaviours are covered without a UI by Rust tests that run in CI:

- `provider::parked::tests::kiro_follow_ups_stay_warm_and_replacements_do_not_wait_for_its_helpers`
- `provider::acp::tests::kiro_*` (launch flags, model selection, late model list)
- `catalog::tests::kiro_model_picker_offers_the_configured_model`
- `pickers::tests::kiro_default_entry_names_the_model_kiro_picks`
