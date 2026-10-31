import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { access, chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));

test("Finder launches use development data for dev bundles and installed data for release bundles", {
  skip: process.platform === "win32",
}, async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "gespan-bundle-data-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const bin = join(directory, "bin");
  const target = join(directory, "target with spaces");
  await mkdir(bin);
  // Exercise packaging without signing or opening an app.
  for (const [name, script] of [
    ["uname", "#!/bin/sh\nprintf 'Darwin\\n'\n"],
    ["codesign", "#!/bin/sh\nexit 0\n"],
  ]) {
    await writeFile(join(bin, name), script);
    await chmod(join(bin, name), 0o755);
  }
  for (const [profile, override, mode] of [
    ["native-dev", undefined, "dev-data"],
    ["release", undefined, "gespan-data"],
    ["release", "dev-data", "dev-data"],
    ["native-dev", "gespan-data", "gespan-data"],
  ]) {
    await t.test(`${profile}: ${override ?? "default"}`, async () => {
      const output = join(target, profile);
      await mkdir(output, { recursive: true });
      const executable = join(output, "gespan-gpui");
      await writeFile(executable, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n");
      await chmod(executable, 0o755);
      const args = ["scripts/bundle-native-macos.sh", profile, target, "0.16.0"];
      if (override) args.push(override);
      const packaged = spawnSync("bash", args, {
        cwd: root,
        env: { ...process.env, PATH: `${bin}${delimiter}${process.env.PATH}` },
        encoding: "utf8",
      });
      assert.ifError(packaged.error);
      assert.equal(packaged.status, 0, packaged.stdout + packaged.stderr);
      // Development data must never run under the installed app's name or identifier.
      const [name, id] =
        mode === "dev-data" ? ["Gespan Dev", "dev.gespan.native.dev"] : ["Gespan", "dev.gespan.native"];
      const bundle = join(output, `${name}.app/Contents`);
      const plist = await readFile(join(bundle, "Info.plist"), "utf8");
      assert.ok(plist.includes(`<key>GespanDataMode</key><string>${mode}</string>`));
      assert.ok(plist.includes(`<key>CFBundleIdentifier</key><string>${id}</string>`));
      assert.ok(plist.includes(`<key>CFBundleDisplayName</key><string>${name}</string>`));
      // macOS rejects menu bar items from a process that exec'd after launch.
      assert.ok(plist.includes("<key>CFBundleExecutable</key><string>gespan-gpui</string>"));
      // Dictation asks for the microphone, and says the audio stays local.
      assert.ok(
        plist.includes(
          "<key>NSMicrophoneUsageDescription</key><string>Gespan uses the microphone when you dictate a message. Audio stays on this device.</string>",
        ),
      );
      // The Dock and Finder show the icon Native owns.
      assert.deepEqual(
        await readFile(join(bundle, "Resources/gespan.icns")),
        await readFile(new URL("../crates/gespan-gpui/packaging/icon.icns", import.meta.url)),
      );
      await assert.rejects(access(join(bundle, "MacOS/gespan-launcher")));
      const launched = spawnSync(join(bundle, "MacOS/gespan-gpui"), ["--cwd", "a project with spaces"], {
        encoding: "utf8",
      });
      assert.ifError(launched.error);
      assert.equal(launched.status, 0, launched.stderr);
      assert.deepEqual(launched.stdout.trim().split("\n"), ["--cwd", "a project with spaces"]);
    });
  }
});
