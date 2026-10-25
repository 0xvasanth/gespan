# Redaction fixtures

`redaction.json` is the shared contract for scrubbing diagnostics. Both apps
run these cases in their tests, so the Tauri app and Gespan Native scrub
the same input in the same way.

- `contexts` describes one machine per platform: its `home`, its `projects`
  (the first is `<project-1>`) and any known `secrets`.
- Each case names a `context` and a `kind`. A `text` case's `input` is a string
  for `Redactor::redact_text`. A `json` case's `input` is any JSON for
  `Redactor::redact_value`.
- `expected` is the exact output. Scrubbing `expected` again must not change it.

To run the cases from another crate, add a dev-dependency with the `fixtures`
feature and use `gespan_redact::fixtures::load()`, as
`crates/gespan-redact/tests/fixtures.rs` does.

When you add a rule, add a case here with a real-looking example. Never paste
a live secret: use made-up values that only have the right shape.
