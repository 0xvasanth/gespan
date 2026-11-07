import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { access, chmod, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
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
      const args = ["scripts/bundle-macos.sh", profile, target, "0.16.0"];
      if (override) args.push(override);
      const packaged = spawnSync("bash", args, {
        cwd: root,
        // A ready herdr keeps release bundles off the network.
        env: { ...process.env, PATH: `${bin}${delimiter}${process.env.PATH}`, GESPAN_HERDR_BINARY: executable },
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

test("herdr is in the bundle before the app is signed, so the signature covers it", {
  skip: process.platform === "win32",
}, async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "gespan-bundle-herdr-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const bin = join(directory, "bin");
  const target = join(directory, "target");
  const output = join(target, "release");
  await mkdir(bin);
  await mkdir(output, { recursive: true });
  await writeFile(join(output, "gespan-gpui"), "#!/bin/sh\n");
  await chmod(join(output, "gespan-gpui"), 0o755);
  const herdr = join(directory, "herdr-fixture");
  await writeFile(herdr, "herdr fixture");
  await chmod(herdr, 0o644);
  // codesign records whether herdr already existed when it ran.
  const seen = join(directory, "seen");
  for (const [name, script] of [
    ["uname", "#!/bin/sh\nprintf 'Darwin\\n'\n"],
    ["codesign", `#!/bin/sh\nfor last do :; done\nif [ -x "$last/Contents/MacOS/herdr" ]; then echo present > ${JSON.stringify(seen)}; else echo missing > ${JSON.stringify(seen)}; fi\n`],
  ]) {
    await writeFile(join(bin, name), script);
    await chmod(join(bin, name), 0o755);
  }
  const bundle = (extra, profile = "release") =>
    spawnSync("bash", ["scripts/bundle-macos.sh", profile, target, "0.16.0"], {
      cwd: root,
      env: { ...process.env, PATH: `${bin}${delimiter}${process.env.PATH}`, ...extra },
      encoding: "utf8",
    });
  const packaged = bundle({ GESPAN_HERDR_BINARY: herdr });
  assert.equal(packaged.status, 0, packaged.stdout + packaged.stderr);
  const copy = join(output, "Gespan.app/Contents/MacOS/herdr");
  assert.equal(await readFile(copy, "utf8"), "herdr fixture");
  assert.equal(await readFile(seen, "utf8"), "present\n");
  assert.equal((await stat(copy)).mode & 0o777, 0o755);

  // A development bundle needs no herdr and does not fetch one.
  await mkdir(join(target, "native-dev"));
  await writeFile(join(target, "native-dev/gespan-gpui"), "#!/bin/sh\n");
  const dev = bundle({ GESPAN_HERDR_BINARY: "" }, "native-dev");
  assert.equal(dev.status, 0, dev.stdout + dev.stderr);
  await assert.rejects(access(join(target, "native-dev/Gespan Dev.app/Contents/MacOS/herdr")));
});
