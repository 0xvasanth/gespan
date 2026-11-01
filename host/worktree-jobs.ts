import { execFile, spawn, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";
import {
  constants,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readlink,
  realpath,
  rm,
  symlink,
  utimes,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, resolve, sep } from "node:path";
import { promisify } from "node:util";

const exec = promisify(execFile);
const LISTING = { maxBuffer: 256 * 1024 * 1024, timeout: 120_000 };
const MAX_LINES = 2_000;
const MAX_LINE = 2_000;
const MAX_NAMES = 20;
const KEEP_FINISHED_MS = 10 * 60_000;
const DRAIN_MS = 500;

export type Copied = {
  files: number;
  bytes: number;
  shared: number;
  skipped: number;
  names: string[];
};

const inside = (root: string, path: string) =>
  path === root || path.startsWith(root.endsWith(sep) ? root : root + sep);

/** Whether the nearest part of `path` that exists is under `root`. */
async function landsInside(path: string, root: string): Promise<boolean> {
  let part = path;
  while (!(await lstat(part).catch(() => null))) {
    const parent = dirname(part);
    if (parent === part) return false;
    part = parent;
  }
  const real = await realpath(part).catch(() => null);
  return real !== null && inside(root, real);
}

async function listed(main: string, ...args: string[]): Promise<string[]> {
  const { stdout } = await exec(
    "git",
    ["ls-files", "-z", "--others", "--ignored", ...args],
    { ...LISTING, cwd: main },
  );
  return stdout.split("\0").filter((entry) => entry && !entry.endsWith("/"));
}

/** The same rules as the desktop's worktree_copy: listed, ignored, and never out of the checkout. */
export async function copyIncluded(
  main: string,
  worktree: string,
): Promise<Copied> {
  const copied: Copied = { files: 0, bytes: 0, shared: 0, skipped: 0, names: [] };
  const include = await lstat(join(main, ".worktreeinclude")).catch(() => null);
  if (!include?.isFile()) return copied;
  const ignored = new Set(await listed(main, "--exclude-standard"));
  const entries = (await listed(main, "--exclude-from=.worktreeinclude"))
    .filter((entry) => ignored.has(entry))
    .filter((entry) => !entry.split("/").includes(".git"))
    .sort();
  const realMain = await realpath(main);
  const realWorktree = await realpath(worktree);
  for (const entry of entries) {
    const source = join(main, entry);
    const dest = join(worktree, entry);
    const metadata = await lstat(source).catch(() => null);
    if (!metadata) {
      copied.skipped += 1;
      continue;
    }
    if (await lstat(dest).catch(() => null)) continue;
    // Checked first: mkdir would follow a link out of the worktree.
    if (!(await landsInside(dirname(dest), realWorktree))) {
      copied.skipped += 1;
      continue;
    }
    await mkdir(dirname(dest), { recursive: true });
    if (metadata.isSymbolicLink()) {
      const target = await readlink(source);
      const lands = await realpath(resolve(dirname(source), target)).catch(
        () => resolve(dirname(source), target),
      );
      if (isAbsolute(target) || !inside(realMain, lands)) {
        copied.skipped += 1;
        continue;
      }
      await symlink(target, dest);
    } else if (metadata.isFile()) {
      await copyFile(source, dest, constants.COPYFILE_FICLONE);
      await utimes(dest, metadata.atime, metadata.mtime).catch(() => {});
      copied.bytes += metadata.size;
    } else {
      copied.skipped += 1;
      continue;
    }
    copied.files += 1;
    if (copied.names.length < MAX_NAMES) copied.names.push(entry);
  }
  return copied;
}

/** cmd pastes %GESPAN_BRANCH% into the line before parsing it. */
export const cmdSafe = (branch: string) =>
  branch.replace(/[^\p{L}\p{N}./_-]/gu, "_");

type Job = {
  lines: string[];
  dropped: number;
  done: boolean;
  code: number | null;
  copied: Copied | null;
  error: string | null;
  cancel: () => void;
};

export type ScriptStart = {
  script: string;
  root: string;
  worktree: string;
  branch: string;
};

/** Long work behind short requests: a Setup script outlasts any one RPC. */
export class WorktreeJobs {
  private jobs = new Map<string, Job>();

  private add(cancel: () => void = () => {}): [string, Job] {
    const id = randomUUID();
    const job: Job = {
      lines: [],
      dropped: 0,
      done: false,
      code: null,
      copied: null,
      error: null,
      cancel,
    };
    this.jobs.set(id, job);
    return [id, job];
  }

  private finish(id: string, job: Job) {
    if (job.done) return;
    job.done = true;
    const timer = setTimeout(() => this.jobs.delete(id), KEEP_FINISHED_MS);
    timer.unref?.();
  }

  startCopy(main: string, worktree: string): string {
    const [id, job] = this.add();
    copyIncluded(main, worktree)
      .then((copied) => {
        job.copied = copied;
      })
      .catch((error: unknown) => {
        job.error = error instanceof Error ? error.message : String(error);
      })
      .finally(() => this.finish(id, job));
    return id;
  }

  async startScript(start: ScriptStart): Promise<string> {
    const env = {
      ...process.env,
      GESPAN_PROJECT_ROOT: start.root,
      GESPAN_WORKTREE_PATH: start.worktree,
      GESPAN_BRANCH:
        process.platform === "win32" ? cmdSafe(start.branch) : start.branch,
    };
    const stdio: ["ignore", "pipe", "pipe"] = ["ignore", "pipe", "pipe"];
    let child: ChildProcess;
    let folder: string | null = null;
    if (process.platform === "win32") {
      folder = await mkdtemp(join(tmpdir(), "gespan-script-"));
      const file = join(folder, "script.cmd");
      await writeFile(
        file,
        `@echo off\r\nchcp 65001 >nul\r\n${start.script.replace(/\r?\n/g, "\r\n")}\r\n`,
      );
      child = spawn("cmd", ["/D", "/C", file], {
        cwd: start.worktree,
        env,
        stdio,
        windowsHide: true,
      });
    } else {
      child = spawn("/bin/sh", ["-e", "-c", start.script.replace(/\r\n/g, "\n")], {
        cwd: start.worktree,
        env,
        stdio,
        detached: true,
      });
    }
    const [id, job] = this.add(() => stop(child));
    const push = (line: string) => {
      const text = line.trimEnd();
      if (!text) return;
      job.lines.push(text.slice(0, MAX_LINE));
      if (job.lines.length > MAX_LINES) {
        job.lines.shift();
        job.dropped += 1;
      }
    };
    const follow = (stream: NodeJS.ReadableStream | null) => {
      let pending = "";
      stream?.setEncoding("utf8");
      stream?.on("data", (chunk: string) => {
        const parts = (pending + chunk).split(/\r\n|\n|\r/);
        pending = parts.pop() ?? "";
        parts.forEach(push);
        if (pending.length >= MAX_LINE * 4) {
          push(pending);
          pending = "";
        }
      });
      stream?.on("end", () => push(pending));
    };
    follow(child.stdout);
    follow(child.stderr);
    const clean = () => {
      if (folder) rm(folder, { recursive: true, force: true }).catch(() => {});
    };
    child.on("error", (error) => {
      job.error = `Could not start the script: ${error.message}`;
      this.finish(id, job);
      clean();
    });
    // `close` comes once the pipes have ended. Something the script left
    // running can hold them open, so `exit` sets a limit on waiting for it.
    child.on("close", () => this.finish(id, job));
    child.on("exit", (code) => {
      job.code = code;
      setTimeout(() => this.finish(id, job), DRAIN_MS);
      clean();
    });
    return id;
  }

  poll(id: string, after: number) {
    const job = this.jobs.get(id);
    if (!job) throw new Error("This job is no longer running");
    const skip = Math.max(0, after - job.dropped);
    return {
      lines: job.lines.slice(skip),
      next: job.dropped + job.lines.length,
      done: job.done,
      code: job.code,
      copied: job.copied,
      error: job.error,
    };
  }

  cancel(id: string) {
    this.jobs.get(id)?.cancel();
  }
}

function stop(child: ChildProcess) {
  if (child.pid === undefined) return;
  if (process.platform === "win32") {
    execFile("taskkill", ["/PID", String(child.pid), "/T", "/F"], {
      windowsHide: true,
    }, () => {});
    return;
  }
  try {
    process.kill(-child.pid, "SIGKILL");
  } catch {
    child.kill("SIGKILL");
  }
}

/** Removes a linked worktree of the project. The branch stays. */
export async function removeHostWorktree(
  main: string,
  path: string,
  force: boolean,
): Promise<void> {
  await exec(
    "git",
    ["worktree", "remove", ...(force ? ["--force"] : []), "--", path],
    { cwd: main, timeout: 120_000 },
  );
}

