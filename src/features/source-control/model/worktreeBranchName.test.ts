import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { nameWorktreeAfterMessage, renameWorktreeBranch } from "./worktrees";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (_command: string, args: { path: string; branch: string }) => ({
    path: args.path,
    branch: args.branch,
  })),
}));
vi.mock("../../../platform/tauri/fs", () => ({ notifyGitChanged: vi.fn() }));

afterEach(() => vi.restoreAllMocks());

describe("worktree branch naming", () => {
  it("renames only from the placeholder the worktree was created on", async () => {
    await renameWorktreeBranch(
      "/repo",
      "/repo-worktrees/rebase-k3j9x2a1",
      "rebase/fix-login",
      "rebase/k3j9x2a1",
    );
    expect(vi.mocked(invoke)).toHaveBeenLastCalledWith("git_worktree_rename_branch", {
      cwd: "/repo",
      path: "/repo-worktrees/rebase-k3j9x2a1",
      branch: "rebase/fix-login",
      from: "rebase/k3j9x2a1",
    });
    // Other callers rename unconditionally, as before.
    await renameWorktreeBranch("/repo", "/repo-worktrees/x", "rebase/x");
    expect(vi.mocked(invoke)).toHaveBeenLastCalledWith(
      "git_worktree_rename_branch",
      expect.objectContaining({ from: null }),
    );
  });

  it("logs why naming failed while keeping the placeholder", async () => {
    const debug = vi.spyOn(console, "debug").mockImplementation(() => {});
    const failure = new Error("The branch was renamed while its name was being generated");
    const renamed = await nameWorktreeAfterMessage({
      generate: async () => "fix-login",
      rename: async () => {
        throw failure;
      },
    });
    expect(renamed).toBeNull();
    expect(debug).toHaveBeenCalledWith("[rebase] worktree branch name", failure);
  });
});
