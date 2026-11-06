// The README's Install section names the current stable release and links to
// its downloads. `bun run set-version` rewrites it between these markers, so a
// release PR carries the new links with the version bump.
import { nativeVersion, rpmRelease } from "./package.mjs";

export const START = "<!-- release:start -->";
export const END = "<!-- release:end -->";
const releases = "https://github.com/0xvasanth/gespan/releases";

export function releaseBlock(version) {
  const parsed = nativeVersion(version);
  const file = (name) => `${releases}/download/v${version}/${name}`;
  const rpm = `gespan-${parsed.base}-${rpmRelease(parsed)}.el10.x86_64.rpm`;
  return [
    START,
    "The current release is",
    `[**${version}**](${releases}/tag/v${version}).`,
    "",
    "| Platform | Download |",
    "| :-- | :-- |",
    `| macOS, Apple Silicon | [Gespan (.dmg)](${file(`Gespan_${version}_aarch64.dmg`)}) |`,
    `| macOS, Intel | [Gespan (.dmg)](${file(`Gespan_${version}_x64.dmg`)}) |`,
    `| Windows, x64 | [Gespan (installer)](${file(`Gespan_${version}_x64-setup.exe`)}) |`,
    `| Linux, x64 | [AppImage](${file(`Gespan_${version}_amd64.AppImage`)}) · [.deb](${file(`gespan_${version}_amd64.deb`)}) · [.rpm](${file(rpm)}) |`,
    END,
  ].join("\n");
}

// Prereleases are not "the current release", so they leave the README as it is.
export function updateReadme(text, version) {
  if (version.includes("-")) return text;
  const eol = text.includes("\r\n") ? "\r\n" : "\n";
  const start = text.indexOf(START);
  const end = text.indexOf(END, start);
  if (start < 0 || end < 0)
    throw new Error(`README.md has no ${START} … ${END} block`);
  const block = releaseBlock(version).replaceAll("\n", eol);
  return text.slice(0, start) + block + text.slice(end + END.length);
}
