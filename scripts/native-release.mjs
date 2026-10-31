import { spawnSync } from "node:child_process";
import { readFile, readdir, writeFile } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { createRequire } from "node:module";
import { nativeVersion } from "./package-native.mjs";

const { nativeAssetBase, releaseNotes } = createRequire(import.meta.url)("./release-channel.cjs");

// The feed carries the same notes as the vX.Y.Z release: the version's
// CHANGELOG section, which the app shows after it updates.
export function feedNotes(version, changelog, env = {}) {
  return env.NATIVE_RELEASE_NOTES || releaseNotes(changelog, version);
}

export const nativeTargets = [
  "darwin-aarch64-app",
  "darwin-x86_64-app",
  "windows-x86_64-nsis",
  "linux-x86_64-appimage",
  "linux-x86_64-deb",
  "linux-x86_64-rpm",
];
export function nativeManifest({ version, assetBaseUrl, artifacts, notes = "" }) {
  const { label } = nativeVersion(version);
  const base = new URL(assetBaseUrl);
  if (
    base.protocol !== "https:" ||
    base.username ||
    base.password ||
    base.search ||
    base.hash
  )
    throw new Error(
      "The native asset base must be an HTTPS URL without credentials, query or fragment",
    );
  const platforms = {};
  for (const artifact of artifacts) {
    if (
      basename(artifact.filename) !== artifact.filename ||
      artifact.filename.includes("\\")
    )
      throw new Error("An artifact must be a plain filename");
    if (!artifact.target) continue;
    if (!nativeTargets.includes(artifact.target) || platforms[artifact.target])
      throw new Error(
        `Duplicate or unexpected native target: ${artifact.target}`,
      );
    if (!artifact.signature?.trim())
      throw new Error(`Missing signature for ${artifact.target}`);
    platforms[artifact.target] = {
      url: `${base.href.replace(/\/$/, "")}/${encodeURIComponent(artifact.filename)}`,
      signature: artifact.signature.trim(),
    };
  }
  for (const target of nativeTargets)
    if (!platforms[target])
      throw new Error(`Missing native package: ${target}`);
  return {
    app_id: "dev.gespan.native",
    channel: label === "stable" ? "native-stable" : "native-preview",
    version,
    notes,
    platforms,
  };
}

export async function prepareNativeRelease(directory, env = process.env) {
  const version = env.GESPAN_NATIVE_VERSION;
  nativeVersion(version);
  const missing = ["TAURI_SIGNING_PRIVATE_KEY", "TAURI_UPDATER_PUBKEY"].filter(
    (name) => !env[name],
  );
  if (missing.length)
    throw new Error(
      `The existing Tauri signing secrets are required; missing: ${missing.join(", ")}`,
    );
  directory = resolve(directory);
  const records = [];
  const files = [];
  const sign = async (filename) => {
    const result = spawnSync(
      process.execPath,
      ["node_modules/@tauri-apps/cli/tauri.js", "signer", "sign", filename],
      {
        env: { ...env, CI: "true" },
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    if (result.error || result.status !== 0)
      throw new Error(`Could not sign ${basename(filename)}`);
    return (await readFile(`${filename}.sig`, "utf8")).trim();
  };
  for (const metadata of (await readdir(directory)).filter((name) =>
    /^artifacts-.*\.json$/.test(name),
  )) {
    for (const record of JSON.parse(
      await readFile(join(directory, metadata), "utf8"),
    )) {
      if (
        basename(record.filename) !== record.filename ||
        record.filename.includes("\\")
      )
        throw new Error("Invalid artifact filename");
      if (files.includes(record.filename))
        throw new Error(`Duplicate native artifact: ${record.filename}`);
      const path = join(directory, record.filename);
      files.push(record.filename);
      if (record.target) {
        record.signature = await sign(path);
        files.push(`${record.filename}.sig`);
      }
      records.push(record);
    }
  }
  const manifest = nativeManifest({
    version,
    assetBaseUrl: nativeAssetBase(env),
    artifacts: records,
    notes: feedNotes(version, await readFile("CHANGELOG.md", "utf8"), env),
  });
  const payload = Buffer.from(JSON.stringify(manifest));
  const payloadPath = join(directory, "native-manifest.payload");
  await writeFile(payloadPath, payload);
  const signature = await sign(payloadPath);
  await writeFile(
    join(directory, "latest.json"),
    JSON.stringify({ payload: payload.toString("base64"), signature }),
  );
  await writeFile(join(directory, "release-files.json"), JSON.stringify(files));
  return files;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  await prepareNativeRelease(process.argv[2]);
}
