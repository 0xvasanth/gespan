import { describe, expect, it } from "vitest";
import { newSession, type Block, type Session } from "../../sessions/model/session";
import { promoteProposedPlan } from "../../../integrations/harness/core/apply";
import { writeBackReply } from "./writeBackReply";

function withBlocks(blocks: Block[]): Session {
  const session = newSession("codex", "/tmp/project", "codex:test");
  session.id = "s1";
  session.blocks = blocks;
  return session;
}

describe("writeBackReply", () => {
  it("is the turn's last assistant reply, not an interim line before a tool call", () => {
    const session = withBlocks([
      { id: "u1", role: "user", text: "Fix the login bug" },
      { id: "a1", role: "assistant", text: "Let me look at the auth code." },
      { id: "t1", role: "tool", text: "rg login" },
      { id: "a2", role: "assistant", text: "Fixed: the cookie lost SameSite." },
    ]);
    expect(writeBackReply(session)).toBe("Fixed: the cookie lost SameSite.");
  });

  it("a plan turn writes back the plan, not the interim line before it", () => {
    const session = withBlocks([
      { id: "u1", role: "user", text: "Plan the migration", intent: "plan" },
      { id: "a1", role: "assistant", text: "Reading the schema first." },
      { id: "t1", role: "tool", text: "cat schema.sql" },
      {
        id: "p1",
        role: "plan",
        text: "1. Add the column\n2. Backfill\n3. Drop the old one",
        plan: { status: "ready" },
      },
    ]);
    expect(writeBackReply(session)).toBe(
      "1. Add the column\n2. Backfill\n3. Drop the old one",
    );
  });

  it("a Codex <proposed_plan> reply in a build turn writes back the plan", () => {
    const promoted = promoteProposedPlan(
      withBlocks([
        { id: "u1", role: "user", text: "Revise the plan" },
        {
          id: "a1",
          role: "assistant",
          text: "Here is the revision.\n<proposed_plan>\n1. Ship it\n2. Watch it\n</proposed_plan>",
        },
      ]),
    );
    expect(promoted.blocks.some((block) => block.role === "plan")).toBe(true);
    expect(writeBackReply(promoted)).toBe("1. Ship it\n2. Watch it");
  });

  it("an assistant block after the plan is the last reply, so it is the one written", () => {
    const session = withBlocks([
      { id: "u1", role: "user", text: "Plan it", intent: "plan" },
      { id: "p1", role: "plan", text: "1. Do it", plan: { status: "ready" } },
      { id: "a1", role: "assistant", text: "Tell me if this works" },
    ]);
    expect(writeBackReply(session)).toBe("Tell me if this works");
    // The same through a promoted Codex reply with text after the plan.
    const promoted = promoteProposedPlan(
      withBlocks([
        { id: "u1", role: "user", text: "Revise the plan" },
        {
          id: "a1",
          role: "assistant",
          text: "<proposed_plan>\n1. Ship it\n</proposed_plan>\nTell me if this works.",
        },
      ]),
    );
    expect(writeBackReply(promoted)).toBe("Tell me if this works.");
  });

  it("never uses reasoning, tool, approval or system text", () => {
    const session = withBlocks([
      { id: "u1", role: "user", text: "Go" },
      { id: "a1", role: "assistant", text: "Done." },
      { id: "r1", role: "reasoning", text: "secret thoughts" },
      { id: "t1", role: "tool", text: "tool output" },
      { id: "x1", role: "approval", text: "Allow rm?" },
      { id: "s1", role: "system", text: "Session error" },
    ]);
    expect(writeBackReply(session)).toBe("Done.");
    expect(
      writeBackReply(
        withBlocks([
          { id: "u1", role: "user", text: "Go" },
          { id: "r1", role: "reasoning", text: "secret thoughts" },
          { id: "t1", role: "tool", text: "tool output" },
        ]),
      ),
    ).toBe("");
  });

  it("stays inside this turn: an earlier turn's reply is not this one's", () => {
    const session = withBlocks([
      { id: "u1", role: "user", text: "First" },
      { id: "p0", role: "plan", text: "Old plan", plan: { status: "ready" } },
      { id: "a1", role: "assistant", text: "Old reply" },
      { id: "u2", role: "user", text: "Second" },
      { id: "a2", role: "assistant", text: "   " },
    ]);
    expect(writeBackReply(session)).toBe("");
  });

  it("skips a queued message sent while the turn ran", () => {
    const session = withBlocks([
      { id: "u1", role: "user", text: "Go" },
      { id: "a1", role: "assistant", text: "Done." },
      { id: "u2", role: "user", text: "Queued follow-up" },
    ]);
    session.queuedMessages = [{ blockId: "u2" } as never];
    expect(writeBackReply(session)).toBe("Done.");
  });

  it("is empty with no session", () => {
    expect(writeBackReply(undefined)).toBe("");
  });
});
