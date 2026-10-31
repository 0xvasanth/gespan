#!/usr/bin/env bash
# Package an already-built native executable; do not build or launch the app.
# Development bundles use separate data; release bundles use installed app data.
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
  echo 'macOS app bundles must be packaged on macOS.' >&2
  exit 1
fi

native_profile="${1:-release}"
native_target="${2:-${CARGO_TARGET_DIR:-crates/gespan-gpui/target}}/$native_profile"
native_version="${3:-$(sed -n 's/^version = "\([^"]*\)".*/\1/p' crates/gespan-gpui/Cargo.toml)}"
if [ "$native_profile" = release ]; then
  native_data_mode="${4:-gespan-data}"
else
  native_data_mode="${4:-dev-data}"
fi
case "$native_data_mode" in
  dev-data|gespan-data) ;;
  *) echo "Unknown data mode: $native_data_mode" >&2; exit 1 ;;
esac
# Development data gets its own app identity, so a dev build never shares the
# installed app's name, Dock icon, defaults or notification permissions.
if [ "$native_data_mode" = dev-data ]; then
  native_app_name="Gespan Dev"
  native_bundle_id="dev.gespan.native.dev"
else
  native_app_name="Gespan"
  native_bundle_id="dev.gespan.native"
fi
native_bundle="$native_target/$native_app_name.app"
native_binary="$native_target/gespan-gpui"
if [ ! -f "$native_binary" ]; then
  echo "Missing native executable: $native_binary" >&2
  echo 'Build it with cargo first, or use bun run native:build.' >&2
  exit 1
fi

mkdir -p "$native_bundle/Contents/MacOS" "$native_bundle/Contents/Resources"
# Replace the inode atomically so macOS does not reuse an older signature cache.
cp "$native_binary" "$native_bundle/Contents/MacOS/gespan-gpui.next"
mv -f "$native_bundle/Contents/MacOS/gespan-gpui.next" "$native_bundle/Contents/MacOS/gespan-gpui"
cp crates/gespan-gpui/packaging/icon.icns "$native_bundle/Contents/Resources/gespan.icns"
# Ship the app's attribution and the license texts for GPUI and embedded assets.
# Add these before signing so the signature covers the notices too.
native_licenses="$native_bundle/Contents/Resources/licenses"
mkdir -p "$native_licenses"
cp LICENSE NOTICE docs/native/LICENSE-GPUI-APACHE docs/native/LICENSE-MERMAID-MIT "$native_licenses/"
cp crates/gespan-gpui/assets/hugeicons.LICENSE \
  crates/gespan-gpui/assets/dark-plus.LICENSE \
  crates/gespan-gpui/assets/native-editor.LICENSE.txt "$native_licenses/"
cp crates/gespan-gpui/assets/material/LICENSE "$native_licenses/material-icons.LICENSE"
rm -f "$native_licenses/Geist-OFL.txt"
# Launch the binary itself: an exec'ing launcher changes the pid version, and macOS
# then refuses the menu bar item. The app reads GespanDataMode from Info.plist.
native_executable="gespan-gpui"
native_short_version="${native_version%%-*}"
native_build_version="$native_short_version"
if [[ "$native_version" == *-preview.* ]]; then
  native_build_version="${native_version##*-preview.}"
fi
rm -f "$native_bundle/Contents/MacOS/gespan-launcher"
cat > "$native_bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>$native_executable</string>
<key>GespanDataMode</key><string>$native_data_mode</string>
<key>CFBundleIdentifier</key><string>$native_bundle_id</string>
<key>CFBundleName</key><string>$native_app_name</string>
<key>CFBundleDisplayName</key><string>$native_app_name</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$native_short_version</string>
<key>CFBundleVersion</key><string>$native_build_version</string>
<key>GespanNativeVersion</key><string>$native_version</string>
<key>CFBundleIconFile</key><string>gespan.icns</string>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSMicrophoneUsageDescription</key><string>Gespan uses the microphone when you dictate a message. Audio stays on this device.</string>
</dict></plist>
PLIST
native_identity="${GESPAN_NATIVE_SIGNING_IDENTITY:--}"
if [ "$native_identity" = - ]; then
  codesign --force --sign - "$native_bundle"
else
  codesign --force --sign "$native_identity" --options runtime --timestamp \
    --entitlements crates/gespan-gpui/packaging/Entitlements.plist "$native_bundle"
fi
printf 'App: %s\n' "$native_bundle"
