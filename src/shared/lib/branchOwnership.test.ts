import { expect, it } from "vitest";
import {
  isAppBranch,
  isOrchestrationBranch,
  isTemporaryWorktreeBranch,
} from "./branchOwnership";

it("recognises current and legacy app-owned branches", () => {
  expect(isAppBranch("gespan/12345678")).toBe(true);
  expect(isAppBranch("mc/12345678")).toBe(true);
  expect(isAppBranch("feature/x")).toBe(false);
  expect(isAppBranch("gespan")).toBe(false);
  expect(isAppBranch("mcx/12345678")).toBe(false);
  expect(isAppBranch(null)).toBe(false);
});

it("recognises orchestration branches under both prefixes only", () => {
  expect(isOrchestrationBranch("gespan/orch-x")).toBe(true);
  expect(isOrchestrationBranch("mc/orch-x")).toBe(true);
  expect(isOrchestrationBranch("feature/x")).toBe(false);
  expect(isOrchestrationBranch("gespan/other")).toBe(false);
  expect(isOrchestrationBranch("feature/orch-x")).toBe(false);
});

it("recognises automatically named temporary worktree branches", () => {
  expect(isTemporaryWorktreeBranch("gespan/12345678")).toBe(true);
  expect(isTemporaryWorktreeBranch("mc/12345678")).toBe(true);
  expect(isTemporaryWorktreeBranch("gespan/1234567")).toBe(false);
  expect(isTemporaryWorktreeBranch("gespan/orch-abcdef")).toBe(false);
  expect(isTemporaryWorktreeBranch("feature/12345678")).toBe(false);
});
