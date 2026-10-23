import { expect, it } from "vitest";
import {
  isAppBranch,
  isOrchestrationBranch,
  isTemporaryWorktreeBranch,
} from "./branchOwnership";

it("recognises current and legacy app-owned branches", () => {
  expect(isAppBranch("rebase/12345678")).toBe(true);
  expect(isAppBranch("mc/12345678")).toBe(true);
  expect(isAppBranch("feature/x")).toBe(false);
  expect(isAppBranch("rebase")).toBe(false);
  expect(isAppBranch("mcx/12345678")).toBe(false);
  expect(isAppBranch(null)).toBe(false);
});

it("recognises orchestration branches under both prefixes only", () => {
  expect(isOrchestrationBranch("rebase/orch-x")).toBe(true);
  expect(isOrchestrationBranch("mc/orch-x")).toBe(true);
  expect(isOrchestrationBranch("feature/x")).toBe(false);
  expect(isOrchestrationBranch("rebase/other")).toBe(false);
  expect(isOrchestrationBranch("feature/orch-x")).toBe(false);
});

it("recognises automatically named temporary worktree branches", () => {
  expect(isTemporaryWorktreeBranch("rebase/12345678")).toBe(true);
  expect(isTemporaryWorktreeBranch("mc/12345678")).toBe(true);
  expect(isTemporaryWorktreeBranch("rebase/1234567")).toBe(false);
  expect(isTemporaryWorktreeBranch("rebase/orch-abcdef")).toBe(false);
  expect(isTemporaryWorktreeBranch("feature/12345678")).toBe(false);
});
