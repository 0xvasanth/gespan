# Build from source

Gespan supports macOS, Linux, and Windows. You need Node.js 20+ and a current stable Rust toolchain. Install and sign in to at least one agent CLI using [provider setup](providers.md).

## Platform prerequisites

On Linux, install the standard Tauri prerequisites, including `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libsoup-3.0-dev`, and `libjavascriptcoregtk-4.1-dev` on Debian/Ubuntu. The setup helpers below install the native dependencies and packaging tools for their distributions.

On Windows, the packaging script uses `stable-x86_64-pc-windows-msvc`. The installer bootstraps the [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) runtime when it is missing.

## Run locally

From the repository root:

```bash
bun install
bun run tauri dev
```

Development builds run as **Gespan Dev** (`dev.gespan.desktop.dev`), so they
keep their own `gespan.db`, settings and notification permissions in
`~/Library/Application Support/dev.gespan.desktop.dev/` and never touch an
installed Gespan you are using at the same time. `bun run native:dev` likewise
runs as **Gespan Native Dev** (`dev.gespan.native.dev`) with its database in
`~/Library/Application Support/Gespan Development/`.

## Ubuntu / Debian packages

On an Ubuntu/Debian workstation, the repository can install the native Tauri prerequisites and build distributable Linux packages directly:

```bash
bun run setup:linux:deb
bun install --frozen-lockfile
bun run build:linux
```

The Linux build emits `.deb` and AppImage bundles under `target/release/bundle/`, in the `deb/` and `appimage/` directories. `build:linux` repacks the AppImage so it uses the host WebKitGTK 4.1 stack instead of bundled Ubuntu libraries. See [Linux installation](install.md) for runtime dependencies and package instructions.

## Fedora / Enterprise Linux packages

On Fedora or Enterprise Linux 10 (registered RHEL, Rocky, Alma, CentOS Stream, Oracle):

```bash
bun run setup:linux:fedora
bun install --frozen-lockfile
bun run build:fedora
```

On EL 10, the setup helper enables EPEL 10 and CRB automatically, since the development packages need CRB. On Oracle Linux it enables `ol10_codeready_builder` and `ol10_developer_EPEL`. Fedora does not need these extra repositories.

The build emits a `.rpm` under `target/release/bundle/rpm/`, installable with:

```bash
sudo dnf install ./target/release/bundle/rpm/Gespan-*.rpm
```

EL 9 and older are unsupported (`webkit2gtk4.1-devel` only exists in EPEL 10). The published release `.rpm` is built on Enterprise Linux 10 so it loads on Fedora and EL 10. See [Linux installation](install.md#fedora--enterprise-linux-packages) for release package installation and runtime repository setup.

## Windows packages

```bash
bun install --frozen-lockfile
bun run build:windows
```

The Windows build emits an NSIS installer under `target/release/bundle/nsis/`.

## Tauri configuration

- [src-tauri/tauri.conf.json](../src-tauri/tauri.conf.json) contains the shared application and bundle configuration.
- [src-tauri/tauri.linux.conf.json](../src-tauri/tauri.linux.conf.json) loads automatically for Linux development and builds.
- [src-tauri/tauri.windows.conf.json](../src-tauri/tauri.windows.conf.json) loads automatically for Windows development and builds.

## Releases

A release ships Gespan Native and the mobile app. The Tauri app is no longer
released or tested in CI.

Both release workflows run only when started by hand from a `native-vX.Y.Z` tag,
so pushing a tag never publishes anything on its own.

1. Add the secrets below under Settings → Secrets and variables → Actions.
2. Bump the version (`bun run set-version`), update [CHANGELOG.md](../CHANGELOG.md), and merge that through a pull request.
3. Tag the merge commit as `native-vX.Y.Z` (for example `native-v0.0.2-beta`) and push the tag.
4. In the Actions tab, run **Native preview release** from that tag. It creates the `vX.Y.Z` GitHub release with the version's CHANGELOG section as its notes, uploads the Native packages and the standalone host packages (Gespan Native's SSH remotes download these), and refreshes the signed `native-latest` update feed. Published files are never replaced, so a rerun only fills in a build that never finished uploading.
5. Run **Mobile release** from the same tag. By default it builds the iOS app on EAS with the `production` profile (beta update channel, auto-incremented build number) and submits it to TestFlight. Choose the `android` or `all` platform to build for Google Play as well; Android submission needs a Google service account key in EAS and one manual upload in Play Console first. Untick **submit** to build without submitting. Choose the `update` release instead to publish the tag's JavaScript over the air to the beta channel. Updates only reach builds with a matching native fingerprint, so make a new build after any native change.

| Secret | Needed for |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `TAURI_UPDATER_PUBKEY` | Signing and verifying Native update packages (always required; the names predate Native) |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | Signing the macOS app |
| `APPLE_API_KEY`, `APPLE_API_KEY_P8`, `APPLE_API_ISSUER` | Notarizing the macOS app |
| `R2_PUBLIC_BASE_URL`, `R2_ACCOUNT_ID`, `R2_BUCKET_NAME`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` | Optional R2 mirror. All or none |
| `EXPO_TOKEN` | The Mobile release and Mobile development workflows. An access token for an Expo account with access to the `@gespan/gespan` project; signing and store credentials live in EAS |

Until the Apple secrets exist, a prerelease can run with **allow_unsigned** ticked in the Run workflow form. The macOS app is then ad-hoc signed and not notarized, so macOS blocks its first launch until it is opened from Finder with Control-click → Open. Windows installers are not code-signed yet, so SmartScreen warns on first run.

Read [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines and checks. The experimental remote host has its own [build and packaging instructions](remote-access.md#release-packaging).

## Gespan Native preview

The experimental GPUI app uses a separate Cargo workspace. The production
commands above continue to build React/Tauri.

Requires Node.js 22+, stable Rust, and Xcode with macOS components on macOS.
Native commands do not need JavaScript dependencies installed.

```sh
bun run native:dev     # launch with a separate development database
bun run native:build   # create an optimized app using installed Gespan data
bun run native:check   # run native checks
```

Type `r` then Enter in the development terminal to rebuild and restart. There is
no hot reload or automatic rebuild on save. Ctrl+C stops the runner and app.

The build command does not launch the app. On macOS, open the result with:

```sh
open "crates/gespan-gpui/target/release/Gespan Native.app"
```

The release app reads and writes your installed Gespan data. It is locally
signed, without notarization or automatic updates. Linux/Windows GUI builds
remain unverified.

See the [native guide](native/README.md) for data paths, output locations,
fixtures and preview limitations.
