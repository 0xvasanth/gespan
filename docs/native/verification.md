# Native checks

The commands to check Gespan Native locally before you open a pull request.

## Local checks

Run the complete native check from the repository root:

```sh
bun run native:check
```

Core tests run in parallel. GPUI tests run serially to isolate rendering state
and avoid CPU contention with the interpreter's wall-clock limits. CI gets the
same isolation by running each GPUI test in its own process with
[cargo-nextest](https://nexte.st), which lets them run in parallel:

```sh
cargo nextest run --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-gpui --cargo-profile native-dev --profile ci
```

`gespan-first-launch` launches the binary `cargo build` made, so build it first.

For focused work, run the relevant part:

```sh
bun run native:test:dev
cargo fmt --manifest-path crates/gespan-gpui/Cargo.toml --all --check
cargo clippy --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-core -p gespan-gpui -p gespan-first-launch --all-targets --profile native-dev -- -D warnings
bun run native:test
```

On macOS with a Metal device, the renderer regression opens no window:

```sh
cargo test --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-gpui --profile native-dev overlapping_shell_pane -- --ignored
```

## Optional runtime checks

The provider smoke example uses an existing CLI login. `--models` only discovers
the catalog; without it, the example makes two real provider requests and checks
streaming and resumed context:

```sh
cargo run --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-core --profile native-dev --example provider_smoke -- cursor --models
cargo run --manifest-path crates/gespan-gpui/Cargo.toml --locked -p gespan-core --profile native-dev --example provider_smoke -- cursor
```

For long-history interaction, use the isolated fixture from the
[native guide](README.md#checks-and-fixtures). Check typing and scrolling while a
separate real session streams, then switch tabs and verify draft/viewport state,
Stop, and saved history after relaunch. Synthetic history is not sent to providers.
