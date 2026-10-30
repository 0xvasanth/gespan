import { describe, expect, it } from "vitest";
import { parseBranchName } from "./gitText";

describe("parseBranchName", () => {
  it("reads the branch from JSON or a plain reply", () => {
    expect(parseBranchName('```json\n{"branch": "Fix Worktree Names"}\n```')).toBe(
      "fix-worktree-names",
    );
    expect(parseBranchName('Sure! {"branch":"open-in-editor"}')).toBe(
      "open-in-editor",
    );
    expect(parseBranchName("add jetbrains ide support")).toBe(
      "add-jetbrains-ide-support",
    );
  });

  it("keeps the placeholder for broken JSON instead of naming it after the braces", () => {
    expect(parseBranchName('{"branch": "fix login",}')).toBeNull();
    expect(parseBranchName('```json\n{"branch": fix login}\n```')).toBeNull();
  });

  it("returns nothing for an empty suggestion", () => {
    expect(parseBranchName('{"branch": "..."}')).toBeNull();
    expect(parseBranchName("   ")).toBeNull();
  });
});
