import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  getSession,
  listSessionsByProject,
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

describe("a ticket link saved in the session store", () => {
  // As `session_get` and `session_list_by_project` return the saved
  // `linked_work_item_json` column.
  const stored = {
    kind: "ticket",
    ticket: {
      tracker: "linear",
      key: "ENG-7",
      url: "https://linear.app/acme/issue/ENG-7",
      title: "Fix login",
      status: "Todo",
      statusGroup: "unstarted",
      fetchedAt: 1700000000000,
    },
  };

  it("a_session_with_ticket_json_loads_from_the_store", async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "session_get")
        return {
          id: "s1",
          cwd: "/repo",
          harness: "claude",
          model: "claude-sonnet",
          modelSettings: {},
          runtimeMode: "supervised",
          title: "ENG-7",
          blocks: [{ id: "b0", role: "user", text: "Fix login" }],
          linkedWorkItem: stored,
          createdAt: 1,
          updatedAt: 2,
        };
      if (command === "session_list_by_project")
        return [
          {
            id: "s1",
            cwd: "/repo",
            harness: "claude",
            runtimeMode: "supervised",
            title: "ENG-7",
            linkedWorkItem: stored,
            createdAt: 1,
            updatedAt: 2,
          },
        ];
      return undefined;
    });

    const session = await getSession("s1");
    expect(session?.linkedWorkItem).toEqual(stored);
    const [summary] = await listSessionsByProject("/repo");
    expect(summary.linkedWorkItem).toEqual(stored);
  });
});
