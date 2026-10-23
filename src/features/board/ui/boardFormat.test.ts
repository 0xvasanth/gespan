import { describe, expect, it } from "vitest";
import type { BoardActivity } from "../model/board";
import {
  activityText,
  linkValue,
  parseLinkValue,
  relativeTime,
  resolveLink,
  sessionLinkTarget,
  type BoardLinkTarget,
} from "./boardFormat";

const NOW = 1_700_000_000_000;
const MINUTE = 60_000;

function row(overrides: Partial<BoardActivity>): BoardActivity {
  return {
    id: "a1",
    boardId: "b1",
    cardId: "c1",
    kind: "moved",
    actor: "user",
    fromColumn: null,
    toColumn: null,
    detail: null,
    createdAt: NOW,
    ...overrides,
  };
}

describe("relativeTime", () => {
  it.each([
    [0, "just now"],
    [30_000, "just now"],
    [2 * MINUTE, "2m ago"],
    [59 * MINUTE, "59m ago"],
    [3 * 60 * MINUTE, "3h ago"],
    [2 * 24 * 60 * MINUTE, "2d ago"],
    [14 * 24 * 60 * MINUTE, "2w ago"],
  ])("%i ms ago reads %s", (ago, text) => {
    expect(relativeTime(NOW - ago, NOW)).toBe(text);
  });

  it("treats a clock running ahead as just now", () => {
    expect(relativeTime(NOW + 5 * MINUTE, NOW)).toBe("just now");
  });
});

describe("activityText", () => {
  it("reads a system move with the stored column names", () => {
    expect(
      activityText(
        row({
          kind: "moved",
          actor: "system",
          fromColumn: "In Progress",
          toColumn: "Review",
          createdAt: NOW - 2 * MINUTE,
        }),
        NOW,
      ),
    ).toBe("Moved from In Progress to Review · system · 2m ago");
  });

  it.each([
    [
      { kind: "created", toColumn: "Backlog" },
      "Created in Backlog · user · just now",
    ],
    [{ kind: "edited" }, "Edited · user · just now"],
    [{ kind: "linked" }, "Linked work · user · just now"],
    [{ kind: "unlinked" }, "Unlinked work · user · just now"],
    [
      { kind: "started", actor: "system" },
      "Linked work started · system · just now",
    ],
    [
      { kind: "completed", actor: "system" },
      "Linked work completed · system · just now",
    ],
    [
      { kind: "failed", actor: "system" },
      "Linked work failed · system · just now",
    ],
  ] as const)("reads %o", (overrides, text) => {
    expect(activityText(row(overrides), NOW)).toBe(text);
  });
});

describe("link values", () => {
  it("round-trips each kind, keeping ids that contain colons", () => {
    expect(parseLinkValue(linkValue("session", "s:1"))).toEqual({
      kind: "session",
      id: "s:1",
    });
    expect(parseLinkValue(linkValue("pilot", "p1"))).toEqual({
      kind: "pilot",
      id: "p1",
    });
    expect(parseLinkValue(linkValue("flow-run", "r1"))).toEqual({
      kind: "flow-run",
      id: "r1",
    });
  });

  it("rejects an unknown kind, a missing id and the no-link value", () => {
    expect(parseLinkValue("linear:ABC-1")).toBeNull();
    expect(parseLinkValue("session:")).toBeNull();
    expect(parseLinkValue("")).toBeNull();
  });

  it("resolves a card's link, marks it missing only once known gone", () => {
    const session: BoardLinkTarget = {
      kind: "session",
      id: "s1",
      label: "Codex · Fix login",
    };
    const targets = new Map<string, BoardLinkTarget | null>([
      [linkValue("session", "s1"), session],
      [linkValue("flow-run", "gone"), null],
    ]);
    expect(resolveLink({ linkKind: null, linkId: null }, targets)).toBeNull();
    expect(resolveLink({ linkKind: "session", linkId: "s1" }, targets)).toEqual(
      { kind: "session", target: session },
    );
    expect(
      resolveLink({ linkKind: "flow-run", linkId: "gone" }, targets),
    ).toEqual({ kind: "flow-run", target: null });
    // Not looked up yet: unconfirmed, not missing.
    expect(
      resolveLink({ linkKind: "session", linkId: "elsewhere" }, targets),
    ).toEqual({ kind: "session", target: null, unconfirmed: true });
  });
});

describe("sessionLinkTarget", () => {
  const session = {
    id: "s1",
    title: "Fix login",
    cwd: "/work/rebase",
    archived: false,
    draft: false,
  };

  it("labels a session with its title and project", () => {
    expect(sessionLinkTarget(session, "running")).toEqual({
      kind: "session",
      id: "s1",
      label: "Fix login",
      detail: "rebase",
      status: "running",
    });
    expect(sessionLinkTarget({ ...session, cwd: "" }).detail).toBeUndefined();
  });

  it("keeps archived and draft sessions as targets but out of the picker", () => {
    expect(sessionLinkTarget({ ...session, archived: true })).toMatchObject({
      id: "s1",
      unlisted: true,
    });
    expect(sessionLinkTarget({ ...session, draft: true }).unlisted).toBe(true);
    expect(sessionLinkTarget(session).unlisted).toBeUndefined();
  });
});
