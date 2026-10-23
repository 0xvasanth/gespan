// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from "vitest";
import { mobileConversation } from "./mobileWorkspace";
import {
  createPilot,
  pilotConversationId,
  pilotIdFromConversation,
  updatePilot,
} from "../../pilots/model/pilot";
import type { Session } from "../../sessions/model/session";

describe("pilotIdFromConversation", () => {
  it("returns the Pilot id after the prefix", () => {
    expect(pilotIdFromConversation("pilot:abc")).toBe("abc");
    expect(pilotIdFromConversation(pilotConversationId("x-1"))).toBe("x-1");
  });

  it("returns null when the prefix is missing", () => {
    expect(pilotIdFromConversation("abc")).toBeNull();
    expect(pilotIdFromConversation("session:abc")).toBeNull();
    expect(pilotIdFromConversation("")).toBeNull();
  });
});

describe("mobileConversation for a Pilot", () => {
  beforeEach(() => localStorage.clear());

  it("resolves pilot:<id> to that Pilot's session", async () => {
    const pilot = createPilot();
    updatePilot(pilot.id, (p) => ({ ...p, sessionId: "session-1" }));
    const live = [
      {
        id: "session-1",
        harness: "claude",
        model: "sonnet",
        blocks: [],
        busy: true,
        backgroundTasks: [],
      },
    ] as unknown as Session[];

    const conversation = await mobileConversation(`pilot:${pilot.id}`, live);

    expect(conversation.id).toBe(`pilot:${pilot.id}`);
    expect(conversation.busy).toBe(true);
  });

  it("rejects a Pilot that no longer exists", async () => {
    await expect(mobileConversation("pilot:missing", [])).rejects.toThrow(
      "Pilot is no longer available",
    );
  });
});
