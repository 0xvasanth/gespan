import assert from "node:assert/strict";
import { test } from "node:test";
import { feedNotes, nativeManifest, nativeTargets } from "./native-release.mjs";
import { previewVersion } from "./package-native.mjs";

const artifacts = nativeTargets.map((target) => ({
  target,
  filename: `${target} with spaces.bin`,
  signature: "signed-package",
}));
function manifest(changes = {}) {
  return nativeManifest({
    version: "0.16.0-preview.2",
    assetBaseUrl: "https://dl.example.test/releases/download/native-v0.16.0-preview.2",
    artifacts,
    ...changes,
  });
}

test("all native architectures and installation types have isolated versioned URLs", () => {
  const result = manifest();
  assert.equal(result.app_id, "dev.gespan.native");
  assert.equal(result.channel, "native-preview");
  assert.deepEqual(
    Object.keys(result.platforms).sort(),
    [...nativeTargets].sort(),
  );
  for (const artifact of Object.values(result.platforms)) {
    assert.match(
      artifact.url,
      /^https:\/\/dl\.example\.test\/releases\/download\/native-v0\.16\.0-preview\.2\/[^/]+$/,
    );
    assert.ok(artifact.url.includes("%20"));
  }
});
test("a missing Intel Mac or Linux package prevents publishing the feed", () => {
  for (const target of nativeTargets) {
    assert.throws(
      () =>
        manifest({ artifacts: artifacts.filter((a) => a.target !== target) }),
      /Missing native package/,
    );
  }
});
test("duplicate targets, unsigned packages and path traversal are refused", () => {
  assert.throws(
    () => manifest({ artifacts: [...artifacts, artifacts[0]] }),
    /Duplicate/,
  );
  assert.throws(
    () =>
      manifest({
        artifacts: artifacts.map((a, i) => (i ? a : { ...a, signature: "" })),
      }),
    /Missing signature/,
  );
  for (const filename of ["../latest.json", "directory\\latest.json"]) {
    assert.throws(
      () =>
        manifest({
          artifacts: artifacts.map((a, i) => (i ? a : { ...a, filename })),
        }),
      /plain filename/,
    );
  }
});
test("production tags and insecure download bases cannot create native releases", () => {
  for (const version of [
    "0.16.0",
    "v0.16.0",
    "native-v0.16.0-preview.1",
    "0.16.0-rc.1",
    "0.16.0-preview.1/../latest.json",
  ]) {
    assert.throws(() => manifest({ version }), /native prerelease version/);
  }
  for (const assetBaseUrl of [
    "http://dl.example.test",
    "https://user:secret@dl.example.test",
    "https://dl.example.test?channel=stable",
  ]) {
    assert.throws(() => manifest({ assetBaseUrl }), /HTTPS URL/);
  }
});
test("preview sequence fits native Windows version resources", () => {
  assert.equal(previewVersion("0.16.0-preview.12").windows, "0.16.0.12");
  assert.throws(() => previewVersion("0.16.0-preview.65536"));
});
test("beta versions map to numeric Windows versions and their own label", () => {
  assert.deepEqual(previewVersion("0.0.1-beta"), {
    version: "0.0.1-beta",
    label: "beta",
    base: "0.0.1",
    sequence: "0",
    windows: "0.0.1.0",
  });
  assert.equal(previewVersion("0.0.1-beta.3").windows, "0.0.1.3");
  assert.equal(previewVersion("0.16.0-preview.2").label, "preview");
  for (const bad of ["0.0.1", "0.0.1-beta.", "0.0.1-alpha", "0.0.1-preview", "0.0.1-beta.65536"]) {
    assert.throws(() => previewVersion(bad), bad);
  }
});

test("the feed's notes are the version's CHANGELOG section", () => {
  const changelog = [
    "# Changelog",
    "",
    "## [Unreleased]",
    "",
    "- Not shipped yet.",
    "",
    "## [0.0.2-beta] - 2026-10-30",
    "",
    "### Added",
    "",
    "- Windows tray icon.",
    "",
    "## [0.0.1-beta] - 2026-10-27",
    "",
    "- First beta.",
  ].join("\n");
  const notes = feedNotes("0.0.2-beta", changelog);
  assert.match(notes, /^### Added\n\n- Windows tray icon\./);
  assert.doesNotMatch(notes, /Not shipped yet|First beta/);
  // A version without a section fails the release rather than shipping other notes.
  assert.throws(() => feedNotes("0.0.3-beta", changelog), /no section for 0\.0\.3-beta/);
  assert.equal(feedNotes("0.0.2-beta", changelog, { NATIVE_RELEASE_NOTES: "Override" }), "Override");
});
