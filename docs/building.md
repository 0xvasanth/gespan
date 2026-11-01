# Build from source

Gespan supports macOS, Linux, and Windows. You need Node.js 22+, a current stable Rust toolchain, and, on macOS, Xcode with macOS components. Install and sign in to at least one agent CLI using [provider setup](providers.md).

## Platform prerequisites

On Debian or Ubuntu, install the GUI build dependencies:

```bash
sudo apt-get install --no-install-recommends -y clang cmake ninja-build pkg-config libfontconfig1-dev libfreetype6-dev libxcb1-dev libxcb-shape0-dev libxcb-xfixes0-dev libxcb-randr0-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libgtk-3-dev libwebkit2gtk-4.1-dev libasound2-dev libx11-xcb-dev libegl1-mesa-dev libvulkan-dev libdbus-1-dev
```

## Run locally

From the repository root:

```bash
bun run native:dev     # launch with a separate development database
bun run native:build   # create an optimized app using installed Gespan data
bun run native:check   # run native checks
```

Native commands do not need JavaScript dependencies installed.

Type `r` then Enter in the development terminal to rebuild and restart. There is
no hot reload or automatic rebuild on save. Ctrl+C stops the runner and app.

`bun run native:dev` runs as **Gespan Native Dev** (`dev.gespan.native.dev`) with
its database in `~/Library/Application Support/Gespan Development/`, so it never
touches an installed Gespan you are using at the same time.

The build command does not launch the app. On macOS, open the result with:

```sh
open "crates/gespan-gpui/target/release/Gespan Native.app"
```

The release app reads and writes your installed Gespan data.

See the [native guide](native/README.md) for data paths, output locations and
fixtures.

## Releases

A release ships Gespan Native and the mobile app.

Both release workflows run only when started by hand from a `vX.Y.Z` tag,
so pushing a tag never publishes anything on its own. The **Release** workflow starts them from the version's tag.

1. Add the secrets below under Settings → Secrets and variables → Actions.
2. Bump the version (`bun run set-version`), update [CHANGELOG.md](../CHANGELOG.md), and merge that through a pull request.

When CI passes on that commit, the **Release** workflow creates the `vX.Y.Z` tag and a draft release, builds iOS for TestFlight and an Android APK, then runs the desktop release, which publishes the draft. See [Releases and the update feed](native/README.md#releases-and-the-update-feed).

| Secret | Needed for |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `TAURI_UPDATER_PUBKEY` | Signing and verifying Native update packages (always required). The names and the `@tauri-apps/cli` dev dependency stay because Native update packages are signed with Tauri's signer, in the format installed apps verify |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | Signing the macOS app |
| `APPLE_API_KEY`, `APPLE_API_KEY_P8`, `APPLE_API_ISSUER` | Notarizing the macOS app |
| `R2_PUBLIC_BASE_URL`, `R2_ACCOUNT_ID`, `R2_BUCKET_NAME`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` | Optional R2 mirror. All or none |
| `EXPO_TOKEN` | The Mobile release and Mobile development workflows. An access token for an Expo account with access to the `@gespan/gespan` project; signing and store credentials live in EAS |

Until the Apple secrets exist, a prerelease can run with **allow_unsigned** ticked in the Run workflow form. The macOS app is then ad-hoc signed and not notarized, so macOS blocks its first launch until it is opened from Finder with Control-click → Open. Windows installers are not code-signed yet, so SmartScreen warns on first run.

Read [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines and checks. The experimental remote host has its own [build and packaging instructions](remote-access.md#release-packaging).
