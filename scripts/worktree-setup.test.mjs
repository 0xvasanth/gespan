import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, mkdir, mkdtemp, readdir, readFile, rm, stat, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const scripts = fileURLToPath(new URL("./", import.meta.url));
const setup = join(scripts, "worktree-setup.sh");
const prune = join(scripts, "worktree-prune-builds.sh");
const TARGETS = ["target", "crates/gespan-gpui/target"];
const DAY = 86400;
const skip = process.platform === "win32" ? "bash scripts" : false;

const gitEnv = {
  GIT_AUTHOR_NAME: "t",
  GIT_AUTHOR_EMAIL: "t@example.com",
  GIT_COMMITTER_NAME: "t",
  GIT_COMMITTER_EMAIL: "t@example.com",
  GIT_CONFIG_GLOBAL: "/dev/null",
  GIT_CONFIG_NOSYSTEM: "1",
};

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    ...options,
    env: { ...process.env, ...gitEnv, ...options.env },
  });
  if (result.error) throw result.error;
  return result;
}

function git(cwd, ...args) {
  const result = run("git", args, { cwd });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}

/** A repository with a main checkout and linked worktrees, plus a bin folder
 * stubbing `bun` so setup never installs anything for real. */
async function repository(t) {
  const directory = await mkdtemp(join(tmpdir(), "gespan-worktree-setup-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const main = join(directory, "main");
  await mkdir(join(main, "crates/gespan-gpui"), { recursive: true });
  await writeFile(join(main, "crates/gespan-gpui/Cargo.toml"), "");
  await writeFile(join(main, ".gitignore"), "target/\nnode_modules/\n");
  git(main, "init", "-q");
  git(main, "add", ".");
  git(main, "commit", "-qm", "init");
  const bin = join(directory, "bin");
  await mkdir(bin);
  await stub(bin, "bun", "mkdir -p node_modules");
  return {
    directory,
    main,
    bin,
    async worktree(name) {
      const path = join(directory, name);
      git(main, "worktree", "add", "-q", "--detach", path);
      return path;
    },
  };
}

async function stub(bin, name, body) {
  await writeFile(join(bin, name), `#!/bin/sh\n${body}\n`);
  await chmod(join(bin, name), 0o755);
}

/** Fakes a Cargo build of one profile `ageDays` old: a marker plus `rlibs - 1`
 * more .rlib files under deps, and a check's .rmeta, with every folder the
 * scripts look at dated to that time. */
async function build(tree, relative, ageDays, marker, { profile = "debug", rlibs = 1 } = {}) {
  const target = join(tree, relative);
  const deps = join(target, profile, "deps");
  await mkdir(deps, { recursive: true });
  const files = [join(deps, "marker.rlib"), join(deps, "libcheck.rmeta")];
  for (let i = 1; i < rlibs; i++) files.push(join(deps, `libdep${i}.rlib`));
  await writeFile(files[0], marker);
  for (const file of files.slice(1)) await writeFile(file, "");
  const time = Date.now() / 1000 - ageDays * DAY;
  for (const path of [...files, deps, join(target, profile), target]) await utimes(path, time, time);
}

const marker = (tree, relative, profile = "debug") =>
  readFile(join(tree, relative, profile, "deps/marker.rlib"), "utf8");

function runSetup(cwd, bin, env = {}) {
  return run("bash", [setup], {
    cwd,
    env: { PATH: `${bin}${delimiter}${process.env.PATH}`, GESPAN_NO_TARGET_PRUNE: "1", ...env },
  });
}

function runPrune(cwd, ...args) {
  return run("bash", [prune, ...args], { cwd });
}

async function leftovers(tree) {
  const found = [];
  for (const relative of TARGETS) {
    const parent = join(tree, dirname(relative));
    if (!existsSync(parent)) continue;
    for (const name of await readdir(parent)) {
      if (name.includes(".seeding.")) found.push(join(parent, name));
    }
  }
  return found;
}

/** Whether this machine's temp folder can clone files, which seeding needs. */
async function canClone() {
  if (process.platform === "darwin") return true;
  const directory = await mkdtemp(join(tmpdir(), "gespan-reflink-"));
  try {
    await writeFile(join(directory, "a"), "a");
    return run("cp", ["--reflink=always", join(directory, "a"), join(directory, "b")]).status === 0;
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}
const cloneSkip = skip || ((await canClone()) ? false : "filesystem can't clone");

test("each profile is cloned from the most complete recent build, so a folder that was only checked doesn't win", {
  skip: cloneSkip,
}, async (t) => {
  const repo = await repository(t);
  const built = await repo.worktree("built");
  const checked = await repo.worktree("checked");
  for (const relative of TARGETS) {
    await build(built, relative, 2, "built", { rlibs: 5 });
    // clippy ran here a moment ago: newest, but hardly any .rlib files...
    await build(checked, relative, 0, "checked", { rlibs: 1 });
    // ...while its native-dev build is the complete one.
    await build(checked, relative, 1, "checked", { profile: "native-dev", rlibs: 5 });
    await build(built, relative, 1, "built", { profile: "native-dev", rlibs: 2 });
  }
  const fresh = await repo.worktree("fresh");

  const result = runSetup(fresh, repo.bin);

  assert.equal(result.status, 0, result.stderr);
  for (const relative of TARGETS) {
    assert.equal(await marker(fresh, relative, "debug"), "built", relative);
    assert.equal(await marker(fresh, relative, "native-dev"), "checked", relative);
    // Cargo decides freshness by modification times, so they must survive.
    const copied = await stat(join(fresh, relative, "debug/deps/marker.rlib"));
    const original = await stat(join(built, relative, "debug/deps/marker.rlib"));
    assert.equal(Math.floor(copied.mtimeMs / 1000), Math.floor(original.mtimeMs / 1000), relative);
  }
  assert.deepEqual(await leftovers(fresh), []);
  // The sources are untouched.
  assert.equal(await marker(built, "target"), "built");
  assert.equal(await marker(checked, "target"), "checked");
});

test("an old build only seeds a worktree when nothing recent exists", { skip: cloneSkip }, async (t) => {
  const repo = await repository(t);
  const old = await repo.worktree("old");
  const recent = await repo.worktree("recent");
  await build(old, "target", 10, "old", { rlibs: 10 });
  await build(recent, "target", 1, "recent", { rlibs: 2 });
  await build(old, "crates/gespan-gpui/target", 10, "old", { rlibs: 10 });
  const fresh = await repo.worktree("fresh");

  const result = runSetup(fresh, repo.bin);

  assert.equal(result.status, 0, result.stderr);
  assert.equal(await marker(fresh, "target"), "recent");
  assert.equal(await marker(fresh, "crates/gespan-gpui/target"), "old");
});

test("downloads kept beside the profiles come along, editor check folders don't", { skip: cloneSkip }, async (t) => {
  const repo = await repository(t);
  const source = await repo.worktree("source");
  await build(source, "target", 1, "source");
  await mkdir(join(source, "target/sherpa-onnx-prebuilt/lib"), { recursive: true });
  await writeFile(join(source, "target/sherpa-onnx-prebuilt/lib/libsherpa.a"), "archive");
  await build(source, "target/flycheck0", 0, "flycheck");
  const fresh = await repo.worktree("fresh");

  const result = runSetup(fresh, repo.bin);

  assert.equal(result.status, 0, result.stderr);
  assert.equal(await readFile(join(fresh, "target/sherpa-onnx-prebuilt/lib/libsherpa.a"), "utf8"), "archive");
  assert.deepEqual((await readdir(join(fresh, "target"))).sort(), ["debug", "sherpa-onnx-prebuilt"]);
  // Each profile is cloned once, not again as a download.
  assert.deepEqual(await readdir(join(fresh, "target/debug")), ["deps"]);
});

test("an existing target folder is left alone, and a missing one beside it is still seeded", {
  skip: cloneSkip,
}, async (t) => {
  const repo = await repository(t);
  const source = await repo.worktree("source");
  for (const relative of TARGETS) await build(source, relative, 1, "source");
  const fresh = await repo.worktree("fresh");
  await build(fresh, "target", 5, "mine");

  const result = runSetup(fresh, repo.bin);

  assert.equal(result.status, 0, result.stderr);
  assert.equal(await marker(fresh, "target"), "mine");
  assert.equal(await marker(fresh, "crates/gespan-gpui/target"), "source");
});

test("the main checkout's target folder can seed a worktree, but the main checkout is never seeded", {
  skip: cloneSkip,
}, async (t) => {
  const repo = await repository(t);
  await build(repo.main, "target", 1, "main");
  const linked = await repo.worktree("linked");
  await build(linked, "crates/gespan-gpui/target", 1, "linked");

  const inMain = runSetup(repo.main, repo.bin);
  assert.equal(inMain.status, 0, inMain.stderr);
  assert.equal(existsSync(join(repo.main, "crates/gespan-gpui/target")), false);

  const fresh = await repo.worktree("fresh");
  const inFresh = runSetup(fresh, repo.bin);
  assert.equal(inFresh.status, 0, inFresh.stderr);
  assert.equal(await marker(fresh, "target"), "main");
  assert.equal(await marker(fresh, "crates/gespan-gpui/target"), "linked");
});

test("a clone that fails part-way leaves no target folder behind and doesn't fail the hook", { skip }, async (t) => {
  const repo = await repository(t);
  const source = await repo.worktree("source");
  for (const relative of TARGETS) await build(source, relative, 1, "source");
  const fresh = await repo.worktree("fresh");
  // Starts writing the destination, then fails, as an interrupted or refused
  // clone would. On macOS the APFS check still runs for real first.
  await stub(repo.bin, "cp", 'for last; do :; done; mkdir -p "$last/debug"; exit 1');

  const result = runSetup(fresh, repo.bin);

  assert.equal(result.status, 0, result.stderr);
  for (const relative of TARGETS) assert.equal(existsSync(join(fresh, relative)), false, relative);
  assert.deepEqual(await leftovers(fresh), []);
  assert.match(result.stderr, /could not clone/);
});

test("on macOS, a volume that isn't APFS is never copied, since cp -c would fall back to a full copy", {
  skip: process.platform === "darwin" ? false : "macOS only",
}, async (t) => {
  const repo = await repository(t);
  const source = await repo.worktree("source");
  await build(source, "target", 1, "source");
  const fresh = await repo.worktree("fresh");
  const calls = join(repo.directory, "cp-calls");
  await stub(repo.bin, "mount", 'echo "/dev/disk9 on / (hfs, local)"');
  await stub(repo.bin, "cp", `echo "$@" >> "${calls}"; exec /bin/cp "$@"`);

  const result = runSetup(fresh, repo.bin);

  assert.equal(result.status, 0, result.stderr);
  assert.equal(existsSync(join(fresh, "target")), false);
  assert.equal(existsSync(calls), false, "cp ran");
});

test("git works as usual in a seeded worktree: clean status, commit, push, fetch and remove", {
  skip: cloneSkip,
}, async (t) => {
  const repo = await repository(t);
  const remote = join(repo.directory, "remote.git");
  git(repo.directory, "init", "-q", "--bare", remote);
  git(repo.main, "remote", "add", "origin", remote);
  const source = await repo.worktree("source");
  for (const relative of TARGETS) await build(source, relative, 1, "source");
  const fresh = await repo.worktree("fresh");
  // Seeding is ours alone: a worktrunk binary on PATH is never run.
  const called = join(repo.directory, "wt-called");
  await stub(repo.bin, "wt", `touch "${called}"; exit 1`);

  const result = runSetup(fresh, repo.bin);

  assert.equal(result.status, 0, result.stderr);
  assert.equal(existsSync(called), false, "worktrunk ran");
  for (const relative of TARGETS) assert.equal(await marker(fresh, relative), "source");
  assert.equal(git(fresh, "status", "--porcelain"), "");
  git(fresh, "switch", "-q", "-c", "feature");
  await writeFile(join(fresh, "change"), "change\n");
  git(fresh, "add", "change");
  git(fresh, "commit", "-qm", "change");
  git(fresh, "push", "-q", "origin", "feature");
  git(repo.main, "fetch", "-q", "origin");
  assert.equal(git(repo.main, "rev-parse", "origin/feature"), git(fresh, "rev-parse", "HEAD"));
  assert.equal(git(repo.main, "rev-parse", "feature"), git(fresh, "rev-parse", "HEAD"));
  // The clones are the worktree's own: writing one leaves the source alone...
  await writeFile(join(fresh, "target/debug/deps/marker.rlib"), "rebuilt");
  assert.equal(await marker(source, "target"), "source");
  // ...and so does removing the worktree with them.
  git(repo.main, "worktree", "remove", fresh);
  assert.equal(existsSync(fresh), false);
  for (const relative of TARGETS) assert.equal(await marker(source, relative), "source");
  assert.equal(git(source, "status", "--porcelain"), "");
});

test("with no build output anywhere, the worktree is left to build and the hook says nothing", { skip }, async (t) => {
  const repo = await repository(t);
  await repo.worktree("other");
  const fresh = await repo.worktree("fresh");
  await mkdir(join(fresh, "node_modules"));

  const result = runSetup(fresh, repo.bin);

  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stderr, "");
  for (const relative of TARGETS) assert.equal(existsSync(join(fresh, relative)), false, relative);
  assert.deepEqual(await leftovers(fresh), []);
});

test("on macOS, a source on another volume is never copied, since clonefile can't cross volumes", {
  skip: process.platform === "darwin" ? false : "macOS only",
}, async (t) => {
  const repo = await repository(t);
  const source = await repo.worktree("source");
  await build(source, "target", 1, "source");
  const fresh = await repo.worktree("fresh");
  const calls = join(repo.directory, "cp-calls");
  // The source reports a different device from the new worktree.
  await stub(
    repo.bin,
    "stat",
    `if [ "$1" = -f ] && [ "$2" = %d ]; then case "$3" in *source*) echo 1 ;; *) echo 2 ;; esac; else exec /usr/bin/stat "$@"; fi`,
  );
  await stub(repo.bin, "cp", `echo "$@" >> "${calls}"; exec /bin/cp "$@"`);

  const result = runSetup(fresh, repo.bin);

  assert.equal(result.status, 0, result.stderr);
  assert.equal(existsSync(join(fresh, "target")), false);
  assert.equal(existsSync(calls), false, "cp ran");
  assert.deepEqual(await leftovers(fresh), []);
  assert.match(result.stderr, /could not clone/);
});

test("GESPAN_NO_TARGET_SEED=1 skips seeding", { skip }, async (t) => {
  const repo = await repository(t);
  const source = await repo.worktree("source");
  await build(source, "target", 1, "source");
  const fresh = await repo.worktree("fresh");

  const result = runSetup(fresh, repo.bin, { GESPAN_NO_TARGET_SEED: "1" });

  assert.equal(result.status, 0, result.stderr);
  assert.equal(existsSync(join(fresh, "target")), false);
});

test("pruning removes idle build output but keeps recent builds, the copies seeding uses, the main checkout and the current worktree", {
  skip,
}, async (t) => {
  const repo = await repository(t);
  await build(repo.main, "target", 60, "main");
  const idle = await repo.worktree("idle");
  await build(idle, "target", 30, "idle");
  await build(idle, "crates/gespan-gpui/target", 30, "idle");
  const recent = await repo.worktree("recent");
  await build(recent, "target", 2, "recent");
  // Older than `idle`'s gpui build, but more complete and nothing is recent,
  // so it's what seeding would clone: it stays.
  const complete = await repo.worktree("complete");
  await build(complete, "crates/gespan-gpui/target", 40, "complete", { rlibs: 3 });
  const current = await repo.worktree("current");
  await build(current, "target", 90, "current");

  const result = runPrune(current);

  assert.equal(result.status, 0, result.stderr);
  assert.equal(existsSync(join(idle, "target")), false);
  assert.equal(existsSync(join(idle, "crates/gespan-gpui/target")), false);
  assert.equal(await marker(recent, "target"), "recent");
  assert.equal(await marker(complete, "crates/gespan-gpui/target"), "complete");
  assert.equal(await marker(repo.main, "target"), "main");
  assert.equal(await marker(current, "target"), "current");
  // The worktree itself is never touched, only its build output.
  assert.equal(existsSync(join(idle, ".git")), true);
});

test("pruning honours --days and --dry-run, and rejects a bad --days", { skip }, async (t) => {
  const repo = await repository(t);
  const five = await repo.worktree("five");
  await build(five, "target", 5, "five");
  const newest = await repo.worktree("newest");
  await build(newest, "target", 0, "newest");

  assert.equal(runPrune(repo.main).status, 0);
  assert.equal(await marker(five, "target"), "five", "kept under the default 14 days");

  const dry = runPrune(repo.main, "--days", "3", "--dry-run");
  assert.equal(dry.status, 0, dry.stderr);
  assert.match(dry.stdout, /would remove .*five\/target/);
  assert.equal(await marker(five, "target"), "five", "dry run deleted");

  assert.equal(runPrune(repo.main, "--days", "3").status, 0);
  assert.equal(existsSync(join(five, "target")), false);
  assert.equal(await marker(newest, "target"), "newest");

  const bad = runPrune(repo.main, "--days", "soon");
  assert.equal(bad.status, 2);
});

test("the setup hook prunes idle build output in the background", { skip }, async (t) => {
  const repo = await repository(t);
  const idle = await repo.worktree("idle");
  await build(idle, "target", 30, "idle");
  const recent = await repo.worktree("recent");
  await build(recent, "target", 1, "recent");
  const fresh = await repo.worktree("fresh");

  const result = runSetup(fresh, repo.bin, { GESPAN_NO_TARGET_PRUNE: "", GESPAN_NO_TARGET_SEED: "1" });
  assert.equal(result.status, 0, result.stderr);

  const deadline = Date.now() + 10_000;
  while (existsSync(join(idle, "target")) && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.equal(existsSync(join(idle, "target")), false);
  assert.equal(await marker(recent, "target"), "recent");
});
