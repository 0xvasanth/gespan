import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  sanitizeLinkedWorkItem,
  setSessionLinkedWorkItem,
} from "./sessionStore";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

beforeEach(() => {
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
});

describe("setSessionLinkedWorkItem", () => {
  it("persists a canonical GitHub work item through the metadata command", async () => {
    await setSessionLinkedWorkItem("session-1", {
      kind: "pr",
      repo: "openai/codex",
      number: 42,
      url: "https://example.com/untrusted",
    });

    expect(invoke).toHaveBeenCalledWith("session_set_linked_work_item", {
      sessionId: "session-1",
      linkedWorkItem: {
        kind: "pr",
        repo: "openai/codex",
        number: 42,
        url: "https://github.com/openai/codex/pull/42",
      },
    });
  });

  it("uses null to remove a persisted link", async () => {
    await setSessionLinkedWorkItem("session-1", undefined);

    expect(invoke).toHaveBeenCalledWith("session_set_linked_work_item", {
      sessionId: "session-1",
      linkedWorkItem: null,
    });
  });
});

describe("sanitizeLinkedWorkItem tickets", () => {
  const ticket = {
    tracker: "linear",
    key: "ENG-7",
    url: "https://linear.app/acme/issue/ENG-7",
    title: "Fix",
    status: "Todo",
    statusGroup: "unstarted",
    fetchedAt: 1700000000000,
  };

  it("keeps a valid ticket item", () => {
    expect(sanitizeLinkedWorkItem({ kind: "ticket", ticket })).toEqual({
      kind: "ticket",
      ticket,
    });
  });

  it("nulls unknown optional fields instead of dropping the ticket", () => {
    const out = sanitizeLinkedWorkItem({
      kind: "ticket",
      ticket: {
        tracker: "jira",
        key: "A-1",
        statusGroup: "weird",
        fetchedAt: "x",
      },
    });
    expect(out).toEqual({
      kind: "ticket",
      ticket: {
        tracker: "jira",
        key: "A-1",
        url: null,
        title: null,
        status: null,
        statusGroup: null,
        fetchedAt: null,
      },
    });
  });

  it("rejects an unknown tracker, a blank key or a missing ticket", () => {
    for (const bad of [
      { kind: "ticket", ticket: { ...ticket, tracker: "trello" } },
      { kind: "ticket", ticket: { ...ticket, key: "  " } },
      { kind: "ticket" },
    ]) {
      expect(sanitizeLinkedWorkItem(bad)).toBeUndefined();
    }
  });
});
