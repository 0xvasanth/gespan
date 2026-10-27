// Package a prebuilt native executable. Never invokes the Tauri app build or
// writes production release/update paths.
import { spawnSync } from "node:child_process";
import {
  cp,
  mkdir,
  readFile,
  readdir,
  writeFile,
  chmod,
} from "node:fs/promises";
import { join, resolve, dirname, basename } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
// A prerelease version: X.Y.Z-preview.N, X.Y.Z-beta or X.Y.Z-beta.N. Windows
// version resources need four numbers, so the sequence (0 when absent) is
// the fourth.
export function previewVersion(version) {
  const match =
    /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-(?:preview\.(0|[1-9]\d*)|beta(?:\.(0|[1-9]\d*))?)$/.exec(
      version,
    );
  const sequence = match && (match[4] ?? match[5] ?? "0");
  const numbers = match ? [...match.slice(1, 4), sequence] : [];
  if (!match || numbers.some((n) => Number(n) > 65535))
    throw new Error(
      "Expected a native prerelease version such as 0.16.0-preview.1 or 0.0.1-beta",
    );
  return {
    version,
    label: match[4] === undefined ? "beta" : "preview",
    base: match.slice(1, 4).join("."),
    sequence,
    windows: numbers.join("."),
  };
}
function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    ...options,
  });
  if (result.error) throw result.error;
  if (result.status !== 0)
    throw new Error(`${command} failed (${result.status})`);
  return result;
}
async function notices(directory) {
  await mkdir(directory, { recursive: true });
  for (const file of [
    "LICENSE",
    "NOTICE",
    "docs/native/LICENSE-GPUI-APACHE",
    "crates/gespan-gpui/assets/hugeicons.LICENSE",
    "crates/gespan-gpui/assets/dark-plus.LICENSE",
    "crates/gespan-gpui/assets/native-editor.LICENSE.txt",
  ]) {
    await cp(join(root, file), join(directory, file.split("/").at(-1)));
  }
  await cp(
    join(root, "crates/gespan-gpui/assets/material/LICENSE"),
    join(directory, "material-icons.LICENSE"),
  );
}
async function linuxTree(directory, binary, kind) {
  const app = join(directory, "usr/lib/gespan-native");
  await mkdir(app, { recursive: true });
  await cp(binary, join(app, "gespan-gpui"));
  await chmod(join(app, "gespan-gpui"), 0o755);
  await writeFile(
    join(app, "native-install.json"),
    JSON.stringify({ app_id: "dev.gespan.native", kind }),
  );
  await notices(join(app, "licenses"));
  await mkdir(join(directory, "usr/bin"), { recursive: true });
  await writeFile(
    join(directory, "usr/bin/gespan-native"),
    '#!/bin/sh\nexec /usr/lib/gespan-native/gespan-gpui "$@"\n',
    { mode: 0o755 },
  );
  await mkdir(join(directory, "usr/share/applications"), { recursive: true });
  const desktop =
    "[Desktop Entry]\nType=Application\nName=Gespan Native\nExec=gespan-native %F\nIcon=gespan-native\nCategories=Development;\nTerminal=false\nStartupWMClass=dev.gespan.native\n";
  await writeFile(
    join(directory, "usr/share/applications/gespan-native.desktop"),
    desktop,
  );
  const icons = join(directory, "usr/share/icons/hicolor/256x256/apps");
  await mkdir(icons, { recursive: true });
  await cp(
    join(root, "public/gespan.png"),
    join(icons, "gespan-native.png"),
  );
  return { app, desktop };
}

export async function packageNative({
  kind,
  binary,
  out,
  version,
  arch = "x86_64",
}) {
  const parsed = previewVersion(version);
  binary = resolve(binary);
  out = resolve(out);
  const embedded = run(binary, ["--version"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  }).stdout.trim();
  if (embedded !== `Gespan Native ${version}`)
    throw new Error(`Native executable version does not match ${version}`);
  await mkdir(out, { recursive: true });
  const work = join(out, "work");
  await mkdir(work, { recursive: true });
  const artifacts = [];
  const add = (filename, target = null) => {
    artifacts.push({ filename, target });
    return join(out, filename);
  };
  if (kind === "macos") {
    const expectedArch = arch === "aarch64" ? "arm64" : "x86_64";
    const actualArch = run("lipo", ["-archs", binary], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "inherit"],
    }).stdout.trim();
    if (actualArch !== expectedArch)
      throw new Error(`Expected ${expectedArch}, found ${actualArch}`);
    run(
      "bash",
      [
        "scripts/bundle-native-macos.sh",
        "release",
        dirname(dirname(binary)),
        version,
        "gespan-data",
      ],
      {
        env: {
          ...process.env,
          GESPAN_NATIVE_SIGNING_IDENTITY:
            process.env.APPLE_SIGNING_IDENTITY || "",
        },
      },
    );
    const app = join(dirname(binary), "Gespan Native.app");
    const identity = process.env.APPLE_SIGNING_IDENTITY || "";
    // Without a Developer ID the app keeps the bundle script's ad-hoc
    // signature and is not notarized; only an explicit opt-in publishes that.
    const developerId = identity !== "" && identity !== "-";
    if (!developerId && process.env.GESPAN_ALLOW_UNSIGNED !== "1")
      throw new Error(
        "Public previews require a Developer ID signing identity; run the release with allow_unsigned to publish an ad-hoc signed build",
      );
    if (!developerId)
      console.warn(
        "No Developer ID: packaging an ad-hoc signed, unnotarized build",
      );
    const submit = async (file, staple) => {
      const result = run(
        "xcrun",
        [
          "notarytool",
          "submit",
          file,
          "--key",
          process.env.APPLE_API_KEY_PATH,
          "--key-id",
          process.env.APPLE_API_KEY,
          "--issuer",
          process.env.APPLE_API_ISSUER,
          "--wait",
          "--output-format",
          "json",
        ],
        { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] },
      );
      if (JSON.parse(result.stdout).status !== "Accepted")
        throw new Error(
          `Apple did not accept ${basename(file)} for notarization`,
        );
      if (staple) {
        run("xcrun", ["stapler", "staple", staple]);
        run("xcrun", ["stapler", "validate", staple]);
      }
    };
    if (developerId) {
      const zip = join(work, "notarize.zip");
      run("ditto", ["-c", "-k", "--keepParent", app, zip]);
      await submit(zip, app);
    }
    run("codesign", ["--verify", "--deep", "--strict", app]);
    // Gatekeeper only accepts notarized Developer ID apps.
    if (developerId) run("spctl", ["--assess", "--type", "execute", app]);
    const name = arch === "aarch64" ? "aarch64" : "x64";
    const archive = add(
      `GespanNative_${version}_${name}.app.tar.gz`,
      `darwin-${arch}-app`,
    );
    run("tar", ["-czf", archive, "-C", dirname(app), "Gespan Native.app"], {
      env: { ...process.env, COPYFILE_DISABLE: "1" },
    });
    const imageRoot = join(work, "dmg");
    await mkdir(imageRoot);
    run("ditto", [app, join(imageRoot, "Gespan Native.app")]);
    run("ln", ["-s", "/Applications", join(imageRoot, "Applications")]);
    const dmg = add(`GespanNative_${version}_${name}.dmg`);
    run("hdiutil", [
      "create",
      "-volname",
      "Gespan Native",
      "-srcfolder",
      imageRoot,
      "-ov",
      "-format",
      "UDZO",
      dmg,
    ]);
    if (developerId) {
      run("codesign", ["--sign", identity, "--timestamp", dmg]);
      await submit(dmg, dmg);
    }
  } else if (kind === "linux") {
    const debRoot = join(work, "deb");
    await linuxTree(debRoot, binary, "deb");
    await mkdir(join(debRoot, "DEBIAN"));
    await writeFile(
      join(debRoot, "DEBIAN/control"),
      `Package: gespan-native\nVersion: ${version.replace("-", "~")}\nArchitecture: amd64\nMaintainer: Gespan maintainers\nSection: devel\nPriority: optional\nDepends: libasound2, libfontconfig1, libfreetype6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libx11-xcb1, libxcb1, libxcb-shape0, libxcb-xfixes0, libxcb-randr0, libegl1, libvulkan1, libdbus-1-3, pkexec | policykit-1\nDescription: Gespan Native experimental preview\n`,
    );
    run("dpkg-deb", [
      "--build",
      "--root-owner-group",
      debRoot,
      add(`gespan-native_${version}_amd64.deb`, "linux-x86_64-deb"),
    ]);
    const appDir = join(work, "GespanNative.AppDir");
    const { desktop } = await linuxTree(appDir, binary, "appimage");
    await writeFile(
      join(appDir, "gespan-native.desktop"),
      desktop.replace("Exec=gespan-native %F", "Exec=gespan-gpui %F"),
    );
    await cp(
      join(root, "public/gespan.png"),
      join(appDir, "gespan-native.png"),
    );
    await writeFile(
      join(appDir, "AppRun"),
      '#!/bin/sh\nexec "$APPDIR/usr/lib/gespan-native/gespan-gpui" "$@"\n',
      { mode: 0o755 },
    );
    run(
      process.env.APPIMAGETOOL || "appimagetool",
      [
        appDir,
        add(
          `GespanNative_${version}_amd64.AppImage`,
          "linux-x86_64-appimage",
        ),
      ],
      {
        env: { ...process.env, ARCH: "x86_64", APPIMAGE_EXTRACT_AND_RUN: "1" },
      },
    );
  } else if (kind === "rpm") {
    const top = join(work, "rpmbuild");
    for (const name of ["BUILD", "RPMS", "SOURCES", "SPECS", "SRPMS"])
      await mkdir(join(top, name), { recursive: true });
    const tree = join(top, "SOURCES/tree");
    await linuxTree(tree, binary, "rpm");
    const spec = join(top, "SPECS/gespan-native.spec");
    await writeFile(
      spec,
      `Name: gespan-native\nVersion: ${parsed.base}\nRelease: 0.${parsed.label}.${parsed.sequence}%{?dist}\nSummary: Gespan Native experimental preview\nLicense: MIT AND Apache-2.0\nBuildArch: x86_64\nRequires: polkit\n%description\nGespan Native experimental preview.\n%install\nmkdir -p %{buildroot}\ncp -a %{_sourcedir}/tree/. %{buildroot}/\n%files\n/usr/bin/gespan-native\n/usr/lib/gespan-native\n/usr/share/applications/gespan-native.desktop\n/usr/share/icons/hicolor/256x256/apps/gespan-native.png\n`,
    );
    run("rpmbuild", [
      "--define",
      `_topdir ${top}`,
      "--define",
      "_build_id_links none",
      "-bb",
      spec,
    ]);
    const rpms = (await readdir(join(top, "RPMS/x86_64"))).filter(
      (n) =>
        n.endsWith(".rpm") &&
        !n.includes("debuginfo") &&
        !n.includes("debugsource"),
    );
    if (rpms.length !== 1) throw new Error("Expected exactly one native rpm");
    await cp(
      join(top, "RPMS/x86_64", rpms[0]),
      add(rpms[0], "linux-x86_64-rpm"),
    );
  } else if (kind === "windows") {
    const stage = join(work, "windows");
    await mkdir(stage);
    await cp(binary, join(stage, "gespan-gpui.exe"));
    await writeFile(
      join(stage, "native-install.json"),
      JSON.stringify({ app_id: "dev.gespan.native", kind: "nsis" }),
    );
    await notices(join(stage, "licenses"));
    const installer = add(
      `GespanNative_${version}_x64-setup.exe`,
      "windows-x86_64-nsis",
    );
    const quote = (value) =>
      value.replaceAll("$", "$$").replaceAll('"', '$\\"');
    const template = await readFile(
      join(root, "scripts/native-installer.nsi"),
      "utf8",
    );
    const script = template
      .replaceAll("@STAGE@", quote(stage))
      .replaceAll("@OUTFILE@", quote(installer))
      .replaceAll("@ICON@", quote(join(root, "src-tauri/icons/icon.ico")))
      .replaceAll("@VERSION@", version)
      .replaceAll("@WINDOWS_VERSION@", parsed.windows);
    const nsi = join(work, "installer.nsi");
    await writeFile(nsi, script);
    run("makensis", [nsi]);
  } else throw new Error(`Unknown native package kind: ${kind}`);
  await writeFile(
    join(out, `artifacts-${kind}-${arch}.json`),
    JSON.stringify(artifacts, null, 2),
  );
  return artifacts;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  const [kind, binary, out, arch] = process.argv.slice(2);
  await packageNative({
    kind,
    binary,
    out,
    arch,
    version: process.env.GESPAN_NATIVE_VERSION,
  });
}
