import { execFileSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { afterEach, expect, it, vi } from "vitest";
import { GithubInbox, GITHUB_INBOX_COMMANDS } from "./github-inbox";
import { HostStore } from "./store";
import { WorkspaceCommands } from "./workspace-commands";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  vi.useRealTimers();
});
const cwd = "/home/remote/my repo",
  repo = "acme/web";
const input = { repo, kind: "pr", number: 42 };
const row = {
  number: 42,
  title: "Remote PR",
  url: "https://github.com/acme/web/pull/42",
  state: "OPEN",
  updatedAt: "2026-10-10",
  isDraft: true,
  labels: [{ name: "bug" }],
  assignees: [{ login: "remote-user" }],
};
const nodes = <T>(nodes: T[], totalCount = nodes.length) => ({
  nodes,
  totalCount,
});

it("lists with the remote account, bounds history, and maps the existing item shape", async () => {
  const raw = vi.fn().mockResolvedValue(JSON.stringify([row]));
  const inbox = new GithubInbox(raw);
  const result = await inbox.run("git_github_work_items", cwd, {
    ...input,
    assignedToMe: true,
    state: "all",
    limit: 1000,
    search: " label:bug ",
  });
  expect(result).toEqual([
    expect.objectContaining({
      kind: "pr",
      repo,
      state: "open",
      draft: true,
      labels: [{ name: "bug", color: "" }],
      assignees: [
        {
          login: "remote-user",
          avatarUrl: "https://avatars.githubusercontent.com/remote-user?s=64",
        },
      ],
    }),
  ]);
  expect(raw).toHaveBeenCalledWith(
    cwd,
    [
      "pr",
      "list",
      "--state",
      "all",
      "--limit",
      "100",
      "--repo",
      repo,
      "--json",
      expect.any(String),
      "--assignee",
      "@me",
      "--search",
      "label:bug",
    ],
    false,
  );
  raw.mockResolvedValue(JSON.stringify(row));
  expect(await inbox.run("git_github_work_item", cwd, input)).toMatchObject({
    number: 42,
    draft: true,
    repo,
  });
});

it("reads details and bounded conversations, including reviews, replies and commit authors", async () => {
  const raw = vi
    .fn()
    .mockResolvedValueOnce(
      JSON.stringify({
        body: "Remote body",
        author: { login: "remote-user" },
        headRefName: "fix",
      }),
    );
  const inbox = new GithubInbox(raw);
  expect(
    await inbox.run("git_github_work_item_details", cwd, input),
  ).toMatchObject({
    body: "Remote body",
    author: "remote-user",
    headRefName: "fix",
  });
  raw.mockResolvedValueOnce(
    JSON.stringify({
      data: {
        repository: {
          pullRequest: {
            reviewDecision: "APPROVED",
            baseRefName: "main",
            headRefName: "fix",
            comments: nodes(
              [
                { id: "hidden", isMinimized: true },
                { id: "issue-comment", createdAt: "2026-10-02" },
              ],
              50,
            ),
            reviews: nodes([
              { id: "pending", state: "PENDING" },
              { id: "review", state: "APPROVED", submittedAt: "2026-10-01" },
            ]),
            reviewThreads: nodes([
              {
                id: "thread",
                path: "a.ts",
                isResolved: true,
                comments: nodes([
                  { id: "inline", originalLine: 12, createdAt: "2026-10-03" },
                  { id: "reply", createdAt: "2026-10-04" },
                ]),
              },
            ]),
            commits: nodes([
              {
                commit: {
                  oid: "abc",
                  messageHeadline: "Fix",
                  committedDate: "2026-10-01",
                  url: "url",
                  author: {
                    name: "Git Author",
                    user: { login: "github-user" },
                  },
                },
              },
            ]),
          },
        },
      },
    }),
  );
  expect(
    await inbox.run("git_github_work_item_thread", cwd, input),
  ).toMatchObject({
    truncated: true,
    reviewDecision: "APPROVED",
    commits: [{ author: "github-user" }],
    comments: [
      { id: "review", state: "APPROVED" },
      { id: "issue-comment" },
      {
        id: "inline",
        threadId: "thread",
        path: "a.ts",
        line: 12,
        resolved: true,
        replies: [{ id: "reply", threadId: "thread" }],
      },
    ],
  });
  expect(raw.mock.calls[1][1]).toContain("number=42");
  raw.mockResolvedValueOnce(
    JSON.stringify({
      data: { repository: { issue: { comments: nodes([]) } } },
    }),
  );
  expect(
    await inbox.run("git_github_work_item_thread", cwd, {
      ...input,
      kind: "issue",
    }),
  ).toMatchObject({ comments: [], commits: [], truncated: false });
  raw.mockResolvedValueOnce(
    JSON.stringify({ errors: [{ message: "not permitted" }] }),
  );
  await expect(
    inbox.run("git_github_work_item_thread", cwd, input),
  ).rejects.toThrow("not permitted");
});

it("loads diffs and preserves the size limit", async () => {
  const meta = {
    files: [{ path: "a.ts", additions: 1, deletions: 2 }],
    additions: 0,
    deletions: 0,
  };
  const raw = vi
    .fn()
    .mockResolvedValueOnce(JSON.stringify(meta))
    .mockResolvedValueOnce("patch");
  const inbox = new GithubInbox(raw);
  expect(await inbox.run("git_github_pr_diff", cwd, input)).toEqual({
    ...meta,
    additions: 1,
    deletions: 2,
    patch: "patch",
    truncated: false,
  });
  raw
    .mockResolvedValueOnce(JSON.stringify(meta))
    .mockRejectedValueOnce(
      Object.assign(new Error("stdout maxBuffer length exceeded"), {
        code: "ERR_CHILD_PROCESS_STDIO_MAXBUFFER",
      }),
    );
  expect(await inbox.run("git_github_pr_diff", cwd, input)).toMatchObject({
    patch: "",
    truncated: true,
  });
});

it("uses the host checkout's commits for full-context diffs without changing its files or index", async () => {
  const root = realpathSync.native(
    mkdtempSync(join(process.cwd(), "node_modules/gespan-gh-diff-")),
  );
  const git = (...args: string[]) =>
    execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
  try {
    git("init", "-q", "-b", "main");
    git("config", "user.name", "Test");
    git("config", "user.email", "test@example.com");
    git("config", "commit.gpgsign", "false");
    writeFileSync(join(root, "a.txt"), "before\nunchanged context\n");
    git("add", ".");
    git("commit", "-q", "-m", "base");
    const baseRefOid = git("rev-parse", "HEAD");
    writeFileSync(join(root, "a.txt"), "after\nunchanged context\n");
    git("commit", "-qam", "head");
    const headRefOid = git("rev-parse", "HEAD");
    writeFileSync(join(root, "a.txt"), "unrelated working change\n");
    const index = readFileSync(join(root, ".git/index"));
    const raw = vi
      .fn()
      .mockResolvedValue(
        JSON.stringify({
          baseRefOid,
          headRefOid,
          files: [{ path: "a.txt", additions: 1, deletions: 1 }],
          additions: 1,
          deletions: 1,
        }),
      );
    const diff = await new GithubInbox(raw).run("git_github_pr_diff", root, {
      ...input,
      fullContext: true,
    });
    expect(diff).toMatchObject({
      patch: expect.stringContaining(" unchanged context"),
      truncated: false,
    });
    expect((diff as { patch: string }).patch).not.toContain(
      "unrelated working change",
    );
    expect(readFileSync(join(root, ".git/index"))).toEqual(index);
    expect(readFileSync(join(root, "a.txt"), "utf8")).toBe(
      "unrelated working change\n",
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

it("distinguishes missing cwd, non-directory cwd and missing host gh", async () => {
  const root = mkdtempSync(
    join(process.cwd(), "node_modules/gespan-gh-errors-"),
  );
  try {
    const inbox = new GithubInbox();
    const list = { repo, kind: "issue" };
    await expect(
      inbox.run("git_github_work_items", join(root, "gone"), list),
    ).rejects.toThrow("Cannot access GitHub working directory");
    const file = join(root, "file");
    writeFileSync(file, "file");
    await expect(
      inbox.run("git_github_work_items", file, list),
    ).rejects.toThrow("is not a directory");
    vi.stubEnv("PATH", root);
    await expect(
      inbox.run("git_github_work_items", root, list),
    ).rejects.toThrow("not installed on this machine");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

it("maps checks and fetches annotations only from the requested GitHub repo", async () => {
  const raw = vi.fn().mockResolvedValueOnce(
    JSON.stringify({
      headRefOid: "abc",
      statusCheckRollup: [
        { name: "running", status: "IN_PROGRESS", conclusion: "FAILURE" },
        { name: "unknown", status: "NEW_STATE", conclusion: "SUCCESS" },
        {
          __typename: "StatusContext",
          context: "external",
          state: "SUCCESS",
          targetUrl: "https://example.com",
          createdAt: "2026-10-01",
        },
      ],
    }),
  );
  const inbox = new GithubInbox(raw);
  expect(await inbox.run("git_github_pr_checks", cwd, input)).toMatchObject({
    headOid: "abc",
    checks: [
      { state: "pending" },
      { state: "unknown" },
      { name: "external", state: "pass", startedAt: "2026-10-01" },
    ],
  });
  raw
    .mockResolvedValueOnce(
      JSON.stringify({
        id: 123,
        check_run_url: "https://api.github.com/repos/acme/web/check-runs/456",
        steps: [{ name: "test", status: "completed", conclusion: "failure" }],
      }),
    )
    .mockResolvedValueOnce(
      JSON.stringify([
        {
          path: "a.ts",
          start_line: 3,
          message: "failed",
          annotation_level: "failure",
        },
      ]),
    );
  expect(
    await inbox.run("git_github_check_details", cwd, { repo, jobId: "123" }),
  ).toMatchObject({
    steps: [{ state: "fail" }],
    annotations: [
      { path: "a.ts", line: 3, message: "failed", level: "failure" },
    ],
    notice: null,
  });
  expect(raw).toHaveBeenLastCalledWith(
    cwd,
    [
      "api",
      "--hostname",
      "github.com",
      "repos/acme/web/check-runs/456/annotations?per_page=100",
    ],
    false,
  );
  raw.mockResolvedValueOnce(
    JSON.stringify({
      id: 123,
      check_run_url: "https://evil.example/annotations",
    }),
  );
  expect(
    await inbox.run("git_github_check_details", cwd, { repo, jobId: "123" }),
  ).toMatchObject({
    annotations: [],
    notice: expect.stringContaining("unavailable"),
  });
  expect(raw).toHaveBeenCalledTimes(4);
});

it("uses fixed action arguments and sends comments by a temporary body file", async () => {
  const raw = vi
    .fn()
    .mockResolvedValueOnce("")
    .mockResolvedValueOnce(JSON.stringify(row));
  const inbox = new GithubInbox(raw);
  expect(
    await inbox.run("git_github_pr_action", cwd, {
      ...input,
      action: "squash",
    }),
  ).toMatchObject({ number: 42 });
  expect(raw).toHaveBeenNthCalledWith(
    1,
    cwd,
    ["pr", "merge", "42", "--repo", repo, "--squash"],
    true,
  );
  let bodyPath = "";
  raw.mockImplementationOnce(async (_cwd, args) => {
    bodyPath = args.at(-1)!;
    expect(readFileSync(bodyPath, "utf8")).toBe(
      "literal $(echo secret)\nsecond line",
    );
    return "https://github.com/acme/web/issues/42#comment";
  });
  expect(
    await inbox.run("git_github_work_item_comment", cwd, {
      ...input,
      body: "literal $(echo secret)\nsecond line",
    }),
  ).toContain("#comment");
  expect(existsSync(bodyPath)).toBe(false);
  raw.mockImplementationOnce(async (_cwd, args) => {
    bodyPath = args.at(-1)!.slice("body=@".length);
    expect(readFileSync(bodyPath, "utf8")).toBe("reply");
    return JSON.stringify({
      data: {
        addPullRequestReviewThreadReply: {
          comment: { url: "https://github.com/reply" },
        },
      },
    });
  });
  expect(
    await inbox.run("git_github_work_item_comment", cwd, {
      ...input,
      body: "reply",
      inReplyTo: "PRRT_123",
    }),
  ).toBe("https://github.com/reply");
  expect(existsSync(bodyPath)).toBe(false);
});

it("accepts the exact arguments Gespan Native sends for every command", async () => {
  const raw = vi.fn(async (_cwd: string, args: string[]) => {
    if (args[0] === "api" && args[1] === "graphql")
      return JSON.stringify({
        data: { repository: { pullRequest: {} } },
      });
    if (args[0] === "api") return JSON.stringify({ id: 7, steps: [] });
    if (args.includes("headRefOid,statusCheckRollup"))
      return JSON.stringify({ headRefOid: "abc", statusCheckRollup: [] });
    if (args.at(-1)?.startsWith("files,")) return JSON.stringify({ files: [] });
    if (args[1] === "view" && args.includes("body,author"))
      return JSON.stringify({ body: "" });
    if (args[1] === "list") return "[]";
    if (args[1] === "comment") return "https://github.com/acme/web/pull/42#c";
    return JSON.stringify(row);
  });
  const inbox = new GithubInbox(raw);
  // Keep in step with `Service` in crates/gespan-core/src/inbox.rs.
  const calls: [(typeof GITHUB_INBOX_COMMANDS)[number], object][] = [
    [
      "git_github_work_items",
      {
        repo,
        kind: "pr",
        assignedToMe: true,
        state: "all",
        search: "",
        limit: 100,
      },
    ],
    ["git_github_work_item", { repo, kind: "issue", number: 42 }],
    ["git_github_work_item_details", { repo, kind: "issue", number: 42 }],
    ["git_github_work_item_thread", { repo, kind: "pr", number: 42 }],
    [
      "git_github_work_item_comment",
      { repo, kind: "pr", number: 42, body: "Thanks", inReplyTo: "" },
    ],
    ["git_github_pr_diff", { repo, number: 42, fullContext: false }],
    ["git_github_pr_checks", { repo, number: 42 }],
    ["git_github_pr_action", { repo, number: 42, action: "ready" }],
    ["git_github_check_details", { repo, jobId: "7" }],
    ["git_github_run_rerun_failed", { repo, runId: "9" }],
  ];
  expect(calls.map(([command]) => command).sort()).toEqual(
    [...GITHUB_INBOX_COMMANDS].sort(),
  );
  for (const [command, args] of calls)
    await expect(
      inbox.run(command, cwd, args as Record<string, unknown>),
    ).resolves.toBeDefined();
});

it("reruns only a run's failed jobs, in the requested repository", async () => {
  const raw = vi.fn().mockResolvedValue("");
  const inbox = new GithubInbox(raw);
  expect(
    await inbox.run("git_github_run_rerun_failed", cwd, { repo, runId: "99" }),
  ).toBeNull();
  expect(raw).toHaveBeenCalledWith(
    cwd,
    ["run", "rerun", "99", "--repo", repo, "--failed"],
    true,
  );
});

it("rejects caller-controlled process, API path and action inputs before gh runs", async () => {
  const raw = vi.fn(),
    inbox = new GithubInbox(raw);
  for (const args of [
    { ...input, repo: "../web" },
    { ...input, repo: "acme/web/escape" },
    { ...input, kind: "exec" },
    { ...input, number: 0 },
  ])
    await expect(
      inbox.run("git_github_work_item", cwd, args),
    ).rejects.toThrow();
  await expect(
    inbox.run("git_github_pr_action", cwd, { ...input, action: "toString" }),
  ).rejects.toThrow();
  await expect(
    inbox.run("git_github_check_details", cwd, { repo, jobId: "1/../../evil" }),
  ).rejects.toThrow();
  for (const runId of ["0", "1 --help", "--failed", 99])
    await expect(
      inbox.run("git_github_run_rerun_failed", cwd, { repo, runId }),
    ).rejects.toThrow("Invalid GitHub workflow run ID");
  await expect(
    inbox.run("git_github_work_item_comment", cwd, {
      ...input,
      body: "reply",
      inReplyTo: "bad/thread",
    }),
  ).rejects.toThrow();
  expect(raw).not.toHaveBeenCalled();
});

it("shares primary GraphQL backoff across Inbox operations until the actual reset", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-12T12:00:00Z"));
  const raw = vi
    .fn()
    .mockRejectedValueOnce(new Error("GraphQL API rate limit exceeded"))
    .mockResolvedValueOnce(
      JSON.stringify({
        data: { rateLimit: { remaining: 0, resetAt: "2026-10-12T12:10:00Z" } },
      }),
    )
    .mockResolvedValue(JSON.stringify(row));
  const inbox = new GithubInbox(raw);
  await expect(inbox.run("git_github_work_items", cwd, input)).rejects.toThrow(
    "rate limit",
  );
  await expect(inbox.run("git_github_work_item", cwd, input)).rejects.toThrow(
    "rate limit",
  );
  expect(raw).toHaveBeenCalledTimes(2);
  vi.advanceTimersByTime(10 * 60_000);
  await expect(inbox.run("git_github_work_item", cwd, input)).rejects.toThrow("rate limit");
  vi.advanceTimersByTime(1_000);
  expect(await inbox.run("git_github_work_item", cwd, input)).toMatchObject({
    number: 42,
  });
  expect(raw).toHaveBeenCalledTimes(3);
});

it.each(["secondary rate limit", "abuse detection", "API rate limit exceeded"])(
  "backs off %s for a minute without extra quota calls",
  async (message) => {
    vi.useFakeTimers();
    const raw = vi
      .fn()
      .mockRejectedValueOnce(new Error(message))
      .mockResolvedValue(JSON.stringify(row));
    const inbox = new GithubInbox(raw);
    await expect(inbox.run("git_github_work_item", cwd, input)).rejects.toThrow(
      message,
    );
    vi.advanceTimersByTime(59_999);
    await expect(inbox.run("git_github_pr_checks", cwd, input)).rejects.toThrow(
      message,
    );
    expect(raw).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1);
    await inbox.run("git_github_work_item", cwd, input);
    expect(raw).toHaveBeenCalledTimes(2);
  },
);

it.skipIf(process.platform === "win32")(
  "launches host gh in a registered checkout with host account configuration, rejecting outside paths before launch",
  async () => {
    const home = realpathSync.native(
      mkdtempSync(join(process.cwd(), "node_modules/gespan-gh-inbox-")),
    );
    const store = new HostStore(join(home, "host.db"));
    try {
      const checkout = join(home, "my repo"),
        bin = join(home, "bin"),
        config = join(home, "host-account");
      mkdirSync(checkout);
      mkdirSync(bin);
      mkdirSync(config);
      execFileSync("git", ["init", "-q"], { cwd: checkout });
      store.addProject(checkout, "remote");
      writeFileSync(
        join(bin, "gh"),
        `#!${process.execPath}\nconst fs = require('node:fs');\nfs.appendFileSync('gh-trace.jsonl', JSON.stringify({cwd:process.cwd(),config:process.env.GH_CONFIG_DIR,args:process.argv.slice(2)})+'\\n');\nconsole.log('[]');\n`,
      );
      chmodSync(join(bin, "gh"), 0o755);
      vi.stubEnv("PATH", `${bin}:${process.env.PATH}`);
      vi.stubEnv("GH_CONFIG_DIR", config);
      const commands = new WorkspaceCommands(store, async (_id, action) =>
        action(),
      );
      const list = { repo, kind: "issue", limit: 5 };
      expect(
        await commands.run("git_github_work_items", { cwd: checkout, ...list }),
      ).toEqual([]);
      const trace = JSON.parse(
        readFileSync(join(checkout, "gh-trace.jsonl"), "utf8"),
      );
      expect(trace).toEqual({
        cwd: checkout,
        config,
        args: [
          "issue",
          "list",
          "--state",
          "open",
          "--limit",
          "5",
          "--repo",
          repo,
          "--json",
          "number,title,url,state,createdAt,updatedAt,labels,assignees,stateReason",
        ],
      });
      await expect(
        commands.run("git_github_work_items", { cwd: home, ...list }),
      ).rejects.toThrow("outside");
      await expect(
        commands.run("git_github_work_items", {
          cwd: join(checkout, "gone"),
          ...list,
        }),
      ).rejects.toThrow();
      for (const command of ["exec", "gh", "run_process"])
        expect(() =>
          commands.run(command, { cwd: checkout, args: ["--version"] }),
        ).toThrow("Unsupported workspace command");
      expect(
        readFileSync(join(checkout, "gh-trace.jsonl"), "utf8")
          .trim()
          .split("\n"),
      ).toHaveLength(1);
      expect(GITHUB_INBOX_COMMANDS).toHaveLength(10);
    } finally {
      store.close();
      rmSync(home, { recursive: true, force: true });
    }
  },
);
