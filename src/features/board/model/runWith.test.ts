import { describe, expect, it, vi } from "vitest";
import type { TicketRef } from "../../sessions/model/session";
import { prepareRun } from "./runWith";

const ticket = {
  tracker: "linear",
  key: "ENG-42",
  url: "https://linear.app/x/issue/ENG-42",
} as unknown as TicketRef;

const request = {
  cardId: "c1",
  harness: "claude" as const,
  model: "claude-sonnet",
  cwd: "/work/api",
  worktree: false,
};

describe("prepareRun", () => {
  it("run_with_starts_linked_session_with_card_prompt_and_inherits_ticket", async () => {
    const io = {
      getRunPrompt: vi.fn(async () => "Work on board card #3: Fix login"),
      getCardTicket: vi.fn(async () => ticket),
    };
    const { session, prompt } = await prepareRun(request, io);
    // The store's prompt goes through untouched; no ticket text is added.
    expect(prompt).toBe("Work on board card #3: Fix login");
    expect(prompt).not.toContain("ENG-42");
    expect(io.getRunPrompt).toHaveBeenCalledWith("c1");
    expect(session.harness).toBe("claude");
    expect(session.cwd).toBe("/work/api");
    expect(session.linkedWorkItem).toEqual({ kind: "ticket", ticket });
    expect(session.workspaceMode).toBeUndefined();
  });

  it("run_with_with_disconnected_tracker_still_inherits_cached_ticket", async () => {
    // Only the store is asked: the tracker is never contacted, so being
    // disconnected can't stop the session or drop the cached ticket.
    const io = {
      getRunPrompt: async () => "prompt",
      getCardTicket: async () => ticket,
    };
    const { session } = await prepareRun(request, io);
    expect(session.linkedWorkItem).toEqual({ kind: "ticket", ticket });
  });

  it("a card with no ticket, or a ticket that can't be read, still starts", async () => {
    const none = await prepareRun(request, {
      getRunPrompt: async () => "p",
      getCardTicket: async () => null,
    });
    expect(none.session.linkedWorkItem).toBeUndefined();
    const failing = await prepareRun(request, {
      getRunPrompt: async () => "p",
      getCardTicket: async () => {
        throw "store is locked";
      },
    });
    expect(failing.session.linkedWorkItem).toBeUndefined();
    expect(failing.prompt).toBe("p");
  });

  it("a refused prompt stops the run before any session exists", async () => {
    await expect(
      prepareRun(request, {
        getRunPrompt: async () => {
          throw "card is gone";
        },
        getCardTicket: async () => null,
      }),
    ).rejects.toBe("card is gone");
  });

  it("works in a new worktree when asked", async () => {
    const { session } = await prepareRun(
      { ...request, worktree: true },
      { getRunPrompt: async () => "p", getCardTicket: async () => null },
    );
    expect(session.workspaceMode).toBe("worktree");
    expect(session.worktreeBase).toBe("HEAD");
  });
});
