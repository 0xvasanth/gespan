# Native architecture

Gespan is a Rust and GPUI app, and it is the only desktop app. The earlier
React/Tauri app was retired once Native had parity; its data formats live on
because Native opens the databases it left behind.

## Code layout

| Location | Responsibility |
| --- | --- |
| `crates/gespan-core/` | Provider transports, persistence, accounts, Git/files, remote connections and shared services |
| `crates/gespan-gpui/` | Native shell, composer, transcript, editor, settings and platform UI |
| `crates/gespan-gpui/Cargo.toml` | Independent native Cargo workspace, profiles and dependency patches |
| `scripts/dev.mjs` | Development build and manual restart runner |
| `scripts/build.mjs` | Optimized build and macOS packaging |
| `scripts/editor-bundle/`, `scripts/arcade-bundle/` | Generators for embedded JavaScript assets |
| `vendor/gpui-pre*/` | Pinned framework and platform patches |

`gespan-core` has no GPUI dependency. Keep provider and storage
behavior in core services, with UI work in GPUI views. Development uses isolated
data; release builds use the installed desktop database. See the
[desktop guide](README.md#data-and-configuration) for paths and commands.

Provider transports use Codex app-server, Claude stream-json, OpenCode HTTP/SSE,
ACP for Cursor/Grok/Antigravity/fx/Hermes/Devin, and Pi/omp RPC. They reuse the
providers' CLI authentication and conversation IDs.

Each provider also has a bundled TOML manifest in
`crates/gespan-core/src/provider/manifests/`. For now it mirrors that
provider's `Spec` row field for field, a test keeps the two in step, and
nothing reads the manifests yet. When you change a `Spec` row, change its
manifest and bump the manifest's `version`.

## Rendering and responsiveness

Keep provider I/O, parsing, file access and persistence off the UI thread.
Composer input should not rebuild the transcript. Transcript rows have stable
identities, bounded blocks and virtualized layout; streaming updates should
preserve the reader's position. Cancellation must remain responsive under load.

The editor embeds generated analysis and formatting bundles. After changing their
source services or dependencies, run `bun install --frozen-lockfile` and `bun run build` inside
`scripts/editor-bundle/`. Generated bundles and their license notices belong
in the repository.

## Framework patches

GPUI and its platform crate are pinned to `gpui-pre` 0.3.9. Local Apple renderer
patches cover alpha blending, font matching, backdrop blur and shadows. The
headless Metal regression is documented in [checks](verification.md#local-checks).

The vendored framework/platform crates remove the unused `zed-scap` dependency.
The native build rejects lockfiles that reintroduce it, and the `screen-capture`
feature remains disabled. Preserve these guards and the licenses in [NOTICE](../../NOTICE).

See [known limitations](parity.md) for known gaps.
