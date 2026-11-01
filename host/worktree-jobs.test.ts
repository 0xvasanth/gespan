import { execFileSync } from "node:child_process";
import {
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { createHostWorktree, hostWorktrees } from "./git-worktrees";
import {
  cmdSafe,
  copyIncluded,
  removeHostWorktree,
  WorktreeJobs,
} from "./worktree-jobs";

function repository(): { main: string; worktree: string } {
  // The native call, for the long name of a temp folder Windows shortens.
  const root = realpathSync.native(
    mkdtempSync(join(tmpdir(), "gespan-host-jobs-")),
  );
  const main = join(root, "main");
  const worktree = join(root, "tree");
  mkdirSync(main);
  mkdirSync(worktree);
  const git = (...args: string[]) =>
    execFileSync("git", args, { cwd: main, stdio: "ignore" });
  git("init", "-b", "main");
  git("config", "user.email", "fixture@example.invalid");
  git("config", "user.name", "Fixture");
  writeFileSync(join(main, "tracked.env"), "tracked");
  writeFileSync(join(main, ".gitignore"), "*.env\nbuild/\n");
  git("add", ".gitignore");
  git("add", "-f", "tracked.env");
  git("commit", "-m", "Initial");
  return { main, worktree };
}

async function finished(jobs: WorktreeJobs, id: string) {
  await vi.waitFor(() => expect(jobs.poll(id, 0).done).toBe(true), {
    timeout: 20_000,
  });
  return jobs.poll(id, 0);
}

it("copies only files that are listed and ignored", async () => {
  const { main, worktree } = repository();
  writeFileSync(
    join(main, ".worktreeinclude"),
    "# comment\n*.env\ntracked.env\n!skip.env\nbuild/\n",
  );
  writeFileSync(join(main, "a b é.env"), "spaces");
  writeFileSync(join(main, "skip.env"), "negated");
  writeFileSync(join(main, "tracked.env"), "edited in main");
  writeFileSync(join(main, "untracked.txt"), "not ignored");
  mkdirSync(join(main, "build"));
  writeFileSync(join(main, "build", "out"), "built");
  const copied = await copyIncluded(main, worktree);
  expect(copied.names).toEqual(["a b é.env", "build/out"]);
  expect(copied.files).toBe(2);
  expect(readFileSync(join(worktree, "a b é.env"), "utf8")).toBe("spaces");
  expect(() => lstatSync(join(worktree, "skip.env"))).toThrow();
  expect(() => lstatSync(join(worktree, "tracked.env"))).toThrow();
  expect(() => lstatSync(join(worktree, "untracked.txt"))).toThrow();
});

it("does nothing without an include file", async () => {
  const { main, worktree } = repository();
  writeFileSync(join(main, "x.env"), "x");
  expect((await copyIncluded(main, worktree)).files).toBe(0);
});

it.skipIf(process.platform === "win32")(
  "does not read an include file linked from elsewhere",
  async () => {
    const { main, worktree } = repository();
    writeFileSync(join(main, "x.env"), "x");
    const outside = join(worktree, "..", "list");
    writeFileSync(outside, "*.env\n");
    symlinkSync(outside, join(main, ".worktreeinclude"));
    expect((await copyIncluded(main, worktree)).files).toBe(0);
  },
);

it.skipIf(process.platform === "win32")(
  "doesn't grow folders through a link in the worktree",
  async () => {
    const { main, worktree } = repository();
    writeFileSync(join(main, ".worktreeinclude"), "build/\n");
    mkdirSync(join(main, "build", "deep"), { recursive: true });
    writeFileSync(join(main, "build", "deep", "out"), "x");
    const outside = join(worktree, "..", "outside");
    mkdirSync(outside);
    symlinkSync(outside, join(worktree, "build"));
    const copied = await copyIncluded(main, worktree);
    expect(copied.skipped).toBe(1);
    expect(() => lstatSync(join(outside, "deep"))).toThrow();
  },
);

it.skipIf(process.platform === "win32")(
  "never follows a link out of the checkout",
  async () => {
    const { main, worktree } = repository();
    writeFileSync(join(main, ".worktreeinclude"), "*.env\n");
    symlinkSync("/etc/hosts", join(main, "absolute.env"));
    symlinkSync("../../outside", join(main, "relative.env"));
    writeFileSync(join(main, "real.env"), "real");
    symlinkSync("real.env", join(main, "alias.env"));
    const copied = await copyIncluded(main, worktree);
    expect(copied.skipped).toBe(2);
    expect(copied.names).toEqual(["alias.env", "real.env"]);
    expect(lstatSync(join(worktree, "alias.env")).isSymbolicLink()).toBe(true);
  },
);

it("runs a script in the worktree and reports its output and code", async () => {
  const { main, worktree } = repository();
  const jobs = new WorktreeJobs();
  const script =
    process.platform === "win32"
      ? "@echo one\r\n@echo %GESPAN_BRANCH%\r\n@exit 3"
      : 'echo one\necho "$GESPAN_BRANCH"\nexit 3';
  const id = await jobs.startScript({
    script,
    root: main,
    worktree,
    branch: "feature",
  });
  const result = await finished(jobs, id);
  expect(result.lines).toEqual(["one", "feature"]);
  expect(result.code).toBe(3);
  expect(jobs.poll(id, 1).lines).toEqual(["feature"]);
  expect(jobs.poll(id, 2).lines).toEqual([]);
});

it("runs copying as a job too", async () => {
  const { main, worktree } = repository();
  writeFileSync(join(main, ".worktreeinclude"), "*.env\n");
  writeFileSync(join(main, "x.env"), "x");
  const jobs = new WorktreeJobs();
  const result = await finished(jobs, jobs.startCopy(main, worktree));
  expect(result.copied?.names).toEqual(["x.env"]);
});

it("stops a running script", async () => {
  const { main, worktree } = repository();
  const jobs = new WorktreeJobs();
  const id = await jobs.startScript({
    script: process.platform === "win32" ? "ping -n 30 127.0.0.1" : "sleep 30",
    root: main,
    worktree,
    branch: "",
  });
  jobs.cancel(id);
  const result = await finished(jobs, id);
  expect(result.code).not.toBe(0);
});

it.skipIf(process.platform === "win32")(
  "finishes even when something it left running holds the pipes",
  async () => {
    const { main, worktree } = repository();
    const jobs = new WorktreeJobs();
    const id = await jobs.startScript({
      script: "sleep 20 &\necho started",
      root: main,
      worktree,
      branch: "",
    });
    const result = await finished(jobs, id);
    expect(result.code).toBe(0);
    expect(result.lines).toEqual(["started"]);
  },
);

it("keeps the last lines of a script that writes a lot", async () => {
  const { main, worktree } = repository();
  const jobs = new WorktreeJobs();
  const script =
    process.platform === "win32"
      ? "@for /L %%i in (1,1,300) do @echo line %%i"
      : "i=0; while [ $i -lt 300 ]; do i=$((i+1)); echo line $i; done";
  const id = await jobs.startScript({ script, root: main, worktree, branch: "" });
  const result = await finished(jobs, id);
  expect(result.lines.at(-1)).toBe("line 300");
});

it("refuses to answer for a job it doesn't know", () => {
  expect(() => new WorktreeJobs().poll("missing", 0)).toThrow("no longer");
});

it("keeps a branch name out of cmd's reach", () => {
  expect(cmdSafe("feature/a-b_c.1")).toBe("feature/a-b_c.1");
  expect(cmdSafe('a&calc|x%PATH%"^')).toBe("a_calc_x_PATH___");
});

it("removes a worktree and keeps its branch", async () => {
  const { main } = repository();
  const tree = await createHostWorktree(main, "going", "HEAD", false);
  await removeHostWorktree(main, tree.path, false);
  const left = await hostWorktrees(main);
  expect(left.worktrees.map((entry) => entry.path)).toEqual([main]);
  expect(
    execFileSync("git", ["branch", "--list", "going"], { cwd: main }).toString(),
  ).toContain("going");
});
