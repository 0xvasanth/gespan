import assert from "node:assert/strict";
import {
  access,
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { test } from "node:test";
import { packageNative } from "./package-native.mjs";

test(
  "Mac preview packaging checks notarization, preserves preview identity and emits both download and updater packages",
  { skip: process.platform === "win32" },
  async (t) => {
    const directory = await mkdtemp(
      join(tmpdir(), "native packaging with spaces-"),
    );
    t.after(() => rm(directory, { recursive: true, force: true }));
    const bin = join(directory, "tools");
    const release = join(directory, "target/release");
    await mkdir(bin);
    await mkdir(release, { recursive: true });
    const binary = join(release, "gespan-gpui");
    await writeFile(
      binary,
      "#!/bin/sh\nprintf 'Gespan Native 0.16.0-preview.2\\n'\n",
    );
    await chmod(binary, 0o755);
    const fakeTool = `#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const name = path.basename(process.argv[1]);
const args = process.argv.slice(2);
if (name === 'uname') console.log('Darwin');
if (name === 'lipo') console.log('arm64');
if (name === 'ditto') {
  if (args[0] === '-c') fs.writeFileSync(args.at(-1), 'zip');
  else fs.cpSync(args[0], args[1], { recursive: true });
}
if (name === 'tar') fs.writeFileSync(args[1], 'archive');
if (name === 'hdiutil') fs.writeFileSync(args.at(-1), 'dmg');
if (name === 'xcrun' && args[0] === 'notarytool') console.log(JSON.stringify({ status: process.env.NATIVE_TEST_NOTARY_STATUS || 'Accepted' }));
`;
    for (const name of [
      "uname",
      "lipo",
      "codesign",
      "spctl",
      "ditto",
      "tar",
      "hdiutil",
      "xcrun",
    ]) {
      await writeFile(join(bin, name), fakeTool);
      await chmod(join(bin, name), 0o755);
    }
    const overrides = {
      PATH: `${bin}${delimiter}${process.env.PATH}`,
      APPLE_SIGNING_IDENTITY: "Developer ID Application: Fixture (TESTTEAM)",
      APPLE_API_KEY_PATH: join(directory, "fixture.p8"),
      APPLE_API_KEY: "FIXTURE",
      APPLE_API_ISSUER: "fixture-issuer",
    };
    const original = Object.fromEntries(
      Object.keys(overrides).map((key) => [key, process.env[key]]),
    );
    t.after(() => {
      for (const [key, value] of Object.entries(original)) {
        if (value === undefined) delete process.env[key];
        else process.env[key] = value;
      }
      delete process.env.NATIVE_TEST_NOTARY_STATUS;
    });
    Object.assign(process.env, overrides);
    const out = join(directory, "output");
    const artifacts = await packageNative({
      kind: "macos",
      binary,
      out,
      version: "0.16.0-preview.2",
      arch: "aarch64",
    });
    assert.equal(artifacts.length, 2);
    assert.ok(
      artifacts.some(
        (a) =>
          a.target === "darwin-aarch64-app" &&
          a.filename.endsWith(".app.tar.gz"),
      ),
    );
    assert.ok(artifacts.some((a) => !a.target && a.filename.endsWith(".dmg")));
    for (const artifact of artifacts)
      await access(join(out, artifact.filename));
    const plist = await readFile(
      join(release, "Gespan Native.app/Contents/Info.plist"),
      "utf8",
    );
    assert.ok(
      plist.includes(
        "<key>CFBundleShortVersionString</key><string>0.16.0</string>",
      ),
    );
    assert.ok(plist.includes("<key>CFBundleVersion</key><string>2</string>"));
    assert.ok(
      plist.includes(
        "<key>GespanNativeVersion</key><string>0.16.0-preview.2</string>",
      ),
    );
    assert.ok(plist.includes("<string>dev.gespan.native</string>"));
    process.env.NATIVE_TEST_NOTARY_STATUS = "Invalid";
    await assert.rejects(
      () =>
        packageNative({
          kind: "macos",
          binary,
          out: join(directory, "mismatched"),
          version: "0.16.0-preview.3",
          arch: "aarch64",
        }),
      /executable version does not match/,
    );
    await writeFile(
      binary,
      "#!/bin/sh\nprintf 'Gespan Native 0.16.0-preview.3\\n'\n",
    );
    await assert.rejects(
      () =>
        packageNative({
          kind: "macos",
          binary,
          out: join(directory, "rejected"),
          version: "0.16.0-preview.3",
          arch: "aarch64",
        }),
      /Apple did not accept/,
    );
  },
);

test(
  "Mac beta packaging without a Developer ID needs the opt-in and skips notarization",
  { skip: process.platform === "win32" },
  async (t) => {
    const directory = await mkdtemp(join(tmpdir(), "native unsigned-"));
    t.after(() => rm(directory, { recursive: true, force: true }));
    const bin = join(directory, "tools");
    const release = join(directory, "target/release");
    await mkdir(bin);
    await mkdir(release, { recursive: true });
    const binary = join(release, "gespan-gpui");
    await writeFile(binary, "#!/bin/sh\nprintf 'Gespan Native 0.0.1-beta\\n'\n");
    await chmod(binary, 0o755);
    const calls = join(directory, "calls.log");
    const fakeTool = `#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const name = path.basename(process.argv[1]);
const args = process.argv.slice(2);
fs.appendFileSync(${JSON.stringify(calls)}, [name, ...args].join(' ') + '\\n');
if (name === 'uname') console.log('Darwin');
if (name === 'lipo') console.log('arm64');
if (name === 'ditto') fs.cpSync(args[0], args[1], { recursive: true });
if (name === 'tar') fs.writeFileSync(args[1], 'archive');
if (name === 'hdiutil') fs.writeFileSync(args.at(-1), 'dmg');
`;
    for (const name of ["uname", "lipo", "codesign", "spctl", "ditto", "tar", "hdiutil", "xcrun"]) {
      await writeFile(join(bin, name), fakeTool);
      await chmod(join(bin, name), 0o755);
    }
    const overrides = {
      PATH: `${bin}${delimiter}${process.env.PATH}`,
      APPLE_SIGNING_IDENTITY: "",
      GESPAN_ALLOW_UNSIGNED: "",
    };
    const original = Object.fromEntries(
      Object.keys(overrides).map((key) => [key, process.env[key]]),
    );
    t.after(() => {
      for (const [key, value] of Object.entries(original)) {
        if (value === undefined) delete process.env[key];
        else process.env[key] = value;
      }
    });
    Object.assign(process.env, overrides);
    const pack = (out) =>
      packageNative({ kind: "macos", binary, out: join(directory, out), version: "0.0.1-beta", arch: "aarch64" });
    // A missing identity must not quietly publish an unsigned build.
    await assert.rejects(() => pack("refused"), /allow_unsigned/);

    process.env.GESPAN_ALLOW_UNSIGNED = "1";
    await writeFile(calls, "");
    const artifacts = await pack("output");
    assert.equal(artifacts.length, 2);
    assert.ok(artifacts.some((a) => a.target === "darwin-aarch64-app"));
    assert.ok(artifacts.some((a) => !a.target && a.filename.endsWith(".dmg")));
    const log = await readFile(calls, "utf8");
    assert.match(log, /^codesign --verify --deep --strict /m, "the ad-hoc signature is still verified");
    for (const step of [/^xcrun notarytool/m, /^xcrun stapler/m, /^spctl /m, /^codesign --sign .*\.dmg$/m]) {
      assert.doesNotMatch(log, step);
    }
    const plist = await readFile(join(release, "Gespan Native.app/Contents/Info.plist"), "utf8");
    assert.ok(plist.includes("<key>CFBundleShortVersionString</key><string>0.0.1</string>"));
    assert.ok(plist.includes("<key>GespanNativeVersion</key><string>0.0.1-beta</string>"));
  },
);
