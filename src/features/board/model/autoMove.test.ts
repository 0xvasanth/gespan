import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { onLinkedWork } from "./autoMove";

describe("onLinkedWork", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
  });

  it("hands the event to the store's auto-move engine in one call", async () => {
    await onLinkedWork("session", "s1", "started");
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("board_linked_event", {
      kind: "session",
      id: "s1",
      event: "started",
    });
  });

  it.each(["needs_input", "pr_opened", "pr_merged"] as const)(
    "sends the %s event in its snake_case rule key",
    async (event) => {
      await onLinkedWork("pilot", "p1", event);
      expect(invoke).toHaveBeenCalledWith("board_linked_event", {
        kind: "pilot",
        id: "p1",
        event,
      });
    },
  );

  it("rejects with the store's first error", async () => {
    invoke.mockRejectedValue("database is locked");
    await expect(onLinkedWork("session", "s1", "completed")).rejects.toBe(
      "database is locked",
    );
  });
});
