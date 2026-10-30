# Gespan Native preview

Gespan Native is the experimental Rust + GPUI app. The production app still
uses React/Tauri. Native builds have their own Cargo workspace and lockfile.

## Requirements

- Node.js 22+ and a current stable Rust toolchain.
- On macOS, Xcode with macOS components.
- Linux core checks need D-Bus headers and pkg-config (`libdbus-1-dev` and
  `pkg-config` on Debian/Ubuntu).
- At least one installed, signed-in agent CLI. See [provider setup](../providers.md).

The native commands do not require `bun install` or a frontend build.
macOS is the primary preview target. A Linux ARM64 source build has been
smoke-tested in a VM; Linux x64 packages and Windows GUI interaction still need
runtime testing. See [preview limitations](parity.md).

Fenced `mermaid` blocks render as native diagrams in Markdown previews and
conversations. See [Mermaid diagrams](mermaid.md) for examples and limitations.

## Develop

From the repository root:

```sh
bun run native:dev
```

Development uses a separate database. To open a particular project:

```sh
bun run native:dev --cwd /path/to/project
```

Type `r` then Enter in the terminal to rebuild and restart. The current app stays
open until the new build succeeds. Ctrl+C stops the app and runner. There is no
hot reload or automatic rebuild on save.

On macOS, the command packages and launches `Gespan Native.app`. Other
platforms launch the executable directly.

## Build

```sh
bun run native:build
```

This creates an optimized build without launching it. **The release app uses the
installed Gespan database, and changes save there.** No extra flags are needed.

On macOS, open:

```sh
open "crates/gespan-gpui/target/release/Gespan Native.app"
```

The app has a separate name and application ID (`dev.gespan.native`) so it can be
installed alongside Gespan. Local source builds are signed locally, without
Apple notarization or configured automatic updates. Linux and Windows builds
produce a standalone executable
under `crates/gespan-gpui/target/release/`. `CARGO_TARGET_DIR` and
`CARGO_BUILD_TARGET` can change these output paths.

The **Gespan Native Preview** GitHub Actions workflow runs checks and build
verification.

## Releases and the update feed

**Native preview release** runs only when started by hand, so pushing a tag
never publishes anything. To cut a preview:

1. Tag the commit as `v<version>` (for example `v0.0.4-beta`) and push the
   tag.
2. In the Actions tab, open **Native preview release**, choose **Run workflow**,
   set **Use workflow from** to that tag, and run it. Dispatching from a branch or
   any tag that is not a `vX.Y.Z` version fails at once in the first job.

The workflow builds and signs every platform package, then publishes to GitHub
Releases:

- The packages and their `.sig` files go on the version's one GitHub release,
  `v<version>` (a prerelease, never marked latest), which the Gespan (Tauri)
  release shares. Whichever workflow runs first creates it, titled
  `Gespan <version>` with that version's CHANGELOG section as its notes; the
  other adds its files. The signed feed is kept there as `native-latest.json`.
  Published files are never replaced; use a new tag to ship a fix. Run the two
  workflows one after the other, not at the same time.
- A fixed `native-latest` release holds only `latest.json`. The workflow creates
  it if missing and overwrites the asset on every native release. The signed
  feed's package URLs point at the versioned release's assets.

If a run fails after the versioned release was published but before the feed was
updated, run the workflow again from the same tag. Nothing is signed and no
package is re-uploaded on a rerun. When `v<version>` already has all of the
native packages, `.sig` files and `native-latest.json`, the run downloads the
original signed feed and the `.sig` files, checks that they match, and
republishes that same feed to `native-latest`. If only some of those assets are
there, the run fails and names the missing ones; use a new tag then.

Keep the signing secrets in a GitHub Environment with required reviewers, so a
release run needs an approval before it can use the signing key. If you move
the secrets into an Environment, also add `environment: <name>` to the `publish`
job in `native-release.yml`, and to the `build` and `rpm` jobs, which read
`TAURI_UPDATER_PUBKEY`. Without it those jobs cannot see the secrets, so the
release breaks silently.

The app's default endpoint is therefore
`https://github.com/0xvasanth/gespan/releases/download/native-latest/latest.json`.
GitHub's `releases/latest` serves the Tauri app's stable `v*` releases, so the
native feed deliberately uses its own tag and never competes with it. To
override the default (HTTPS only), set the `NATIVE_UPDATER_ENDPOINT` repository
variable (Settings, Secrets and variables, Actions, Variables) for CI releases,
which the workflow passes to `scripts/release-channel.cjs native-config`. For a
local build, set `GESPAN_NATIVE_UPDATER_ENDPOINT` yourself instead.

Required repository secrets: `TAURI_UPDATER_PUBKEY` (embedded in the app),
`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, plus the
Apple signing secrets. Cloudflare R2 is optional: when all of `R2_PUBLIC_BASE_URL`,
`R2_ACCOUNT_ID`, `R2_BUCKET_NAME`, `R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY`
are set, packages and `latest.json` are also mirrored under `native-preview/`;
otherwise that step is skipped. The channel logic lives in
`scripts/release-channel.cjs` and is tested in `scripts/release-channel.test.cjs`.

The macOS app icon is `crates/gespan-gpui/packaging/icon.icns`.

## Running alongside Gespan

Gespan Native and the Tauri Gespan app share one database,
`~/Library/Application Support/dev.gespan.desktop/gespan.db` on macOS. Before
you use Gespan Native with your real data:

- Back up `gespan.db` with both apps closed.
- Do not have the same conversation open in both apps at the same time. Tauri
  Gespan saves whole conversations, so it can overwrite turns that were added in
  Gespan Native.

Opening different conversations in each app is fine. A code safeguard for this is
planned but not built yet.
### AppImage desktop integration

Move the AppImage to a permanent location and make it executable, then opt in
to a per-user application menu entry and Wayland icon registration:

```sh
./GespanNative.AppImage --install-desktop
```

This installs `dev.gespan.native.desktop` and the app icon under
`$XDG_DATA_HOME` (default: `~/.local/share`). The launcher points to the AppImage,
not its temporary mount. Normal launches do not install anything. Run the
command again after moving or renaming the AppImage.

## Data and configuration

Development stores `gespan.db` under:

| Platform | Development directory |
| --- | --- |
| macOS | `~/Library/Application Support/Gespan Development/` |
| Linux | `$XDG_DATA_HOME/Gespan Development/`, or `~/.local/share/Gespan Development/` |
| Windows | `%LOCALAPPDATA%\Gespan Development\` |

`GESPAN_NATIVE_DATA_DIR` overrides this development directory.

Release builds open `dev.gespan.desktop/gespan.db` under macOS Application
Support, Linux's XDG data directory, or Windows' `%APPDATA%`. The app uses the
existing desktop schema and session IDs. GPUI preferences are saved beside the
database in `gespan.db.gpui.sqlite3`.

Failure counters (how often worktree creation, agent starts, Git, updates and
similar operations failed each day, by a short fixed error code) are kept in
`gespan-native.db`, also beside the database. Only Gespan Native opens that
file; nothing is added to `gespan.db`. The counts stay on this machine.

Supported desktop preferences and workspace state are read from the installed
app's webview localStorage on macOS and Linux. The Windows WebView2 reader is
implemented and covered by synthetic fixtures on Linux; reading a real Windows
profile remains unverified. Native preference edits are stored separately.
Some desktop layout concepts still need native support; see
[preview limitations](parity.md).

The Windows reader always opens a temporary copy of localStorage. Copying while
Tauri is writing is best-effort: an inconsistent copy can leave desktop
preferences incomplete or unavailable for that launch. Close Tauri before
starting GPUI for the first time.

To inspect the installed database without opening a window or changing it, use
the executable from a release build:

```sh
crates/gespan-gpui/target/release/gespan-gpui --check-data
```

Provider CLI paths can be changed in Settings → Providers. CLI authentication
continues to use each provider's existing sign-in.

## Checks and fixtures

```sh
bun run native:check
```

This runs the script tests, Rust formatting, strict Clippy and native tests.
See [native checks](verification.md) for individual commands and optional
provider checks.

A synthetic long-history fixture uses development data and makes no provider
request:

```sh
GESPAN_NATIVE_DATA_DIR=/tmp/gespan-native-lab bun run native:dev --fixture
```

Fixtures are refused with production data. Use a fresh temporary directory when
you want an empty fixture store.

## Contributing

See [architecture](migration.md) for the crate boundaries and renderer patches,
and [preview limitations](parity.md) for remaining work. The
[session visuals guide](visuals.md) documents the agent-facing visual API and
examples.
