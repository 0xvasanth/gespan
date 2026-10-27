import { describe, expect, it } from "vitest";
import type { BoardActivity } from "../model/board";
import {
  activityText,
  dayStart,
  dueChip,
  dueInputValue,
  nextLabelColor,
  LABEL_COLORS,
  parseDueInput,
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
    ).toBe("Moved from In Progress to Review · System · 2m ago");
  });

  it.each([
    [
      { kind: "created", toColumn: "Backlog" },
      "Created in Backlog · You · just now",
    ],
    [{ kind: "edited" }, "Edited · You · just now"],
    [{ kind: "linked" }, "Linked work · You · just now"],
    [{ kind: "unlinked" }, "Unlinked work · You · just now"],
    [
      { kind: "started", actor: "system" },
      "Linked work started · System · just now",
    ],
    [
      { kind: "completed", actor: "system" },
      "Linked work completed · System · just now",
    ],
    [
      { kind: "failed", actor: "system" },
      "Linked work failed · System · just now",
    ],
    [
      { kind: "needs_input", actor: "system" },
      "Linked work needs input · System · just now",
    ],
    [
      { kind: "pr_opened", actor: "system" },
      "Pull request opened · System · just now",
    ],
    [
      { kind: "pr_merged", actor: "system" },
      "Pull request merged · System · just now",
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
    cwd: "/work/gespan",
    archived: false,
    draft: false,
  };

  it("labels a session with its title and project", () => {
    expect(sessionLinkTarget(session, "running")).toEqual({
      kind: "session",
      id: "s1",
      label: "Fix login",
      detail: "gespan",
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

describe("dueChip", () => {
  const now = new Date(2026, 9, 25, 15, 30).getTime();
  const at = (y: number, m: number, d: number) => new Date(y, m, d).getTime();

  it("colours by how close the local day is", () => {
    expect(dueChip(at(2026, 9, 24), now)).toEqual({
      text: "Yesterday",
      tone: "overdue",
    });
    expect(dueChip(at(2026, 9, 25), now)).toEqual({
      text: "Today",
      tone: "soon",
    });
    expect(dueChip(at(2026, 9, 26), now)).toEqual({
      text: "Tomorrow",
      tone: "soon",
    });
    expect(dueChip(at(2026, 9, 28), now).tone).toBe("later");
  });

  it("a due date earlier today is not overdue", () => {
    expect(dueChip(new Date(2026, 9, 25, 0, 0).getTime(), now).tone).toBe(
      "soon",
    );
  });

  it("names the weekday within a week, then the date", () => {
    expect(dueChip(at(2026, 9, 28), now).text).toBe("Wed");
    expect(dueChip(at(2026, 10, 3), now)).toEqual({
      text: "Nov 3",
      tone: "later",
    });
    expect(dueChip(at(2027, 0, 5), now).text).toBe("Jan 5, 2027");
    expect(dueChip(at(2026, 8, 1), now)).toEqual({
      text: "Sep 1",
      tone: "overdue",
    });
  });
});

describe("edited rows with a detail", () => {
  it("edited_with_detail_reads_changed_detail", () => {
    expect(
      activityText(row({ kind: "edited", detail: "priority: High" }), NOW),
    ).toBe("Changed priority: High · You · just now");
  });

  it("names the actor: You, System, else Agent", () => {
    const detail = "labels: Bug";
    expect(
      activityText(row({ kind: "edited", detail, actor: "system" }), NOW),
    ).toBe("Changed labels: Bug · System · just now");
    expect(
      activityText(
        row({ kind: "edited", detail, actor: "claude-code" as never }),
        NOW,
      ),
    ).toBe("Changed labels: Bug · Agent · just now");
  });
});

describe("due dates and label colours", () => {
  it("builds local-midnight days and reads them back for the date input", () => {
    const now = new Date(2026, 9, 25, 23, 59).getTime();
    expect(dayStart(0, now)).toBe(new Date(2026, 9, 25).getTime());
    expect(dayStart(1, now)).toBe(new Date(2026, 9, 26).getTime());
    expect(dayStart(7, now)).toBe(new Date(2026, 10, 1).getTime());
    expect(dueInputValue(new Date(2026, 10, 2, 18).getTime())).toBe(
      "2026-11-02",
    );
    expect(dueInputValue(null)).toBe("");
  });

  it("parses a date input to local midnight, null when empty or invalid", () => {
    expect(parseDueInput("2026-12-31")).toBe(new Date(2026, 11, 31).getTime());
    expect(parseDueInput("")).toBeNull();
    expect(parseDueInput("2026-13-40")).toBeNull();
    expect(parseDueInput("nope")).toBeNull();
  });

  it("hands out the eight palette colours round-robin", () => {
    expect(LABEL_COLORS).toHaveLength(8);
    for (const color of LABEL_COLORS) expect(color).toMatch(/^[0-9a-f]{6}$/);
    expect(nextLabelColor(0)).toBe(LABEL_COLORS[0]);
    expect(nextLabelColor(8)).toBe(LABEL_COLORS[0]);
    expect(nextLabelColor(10)).toBe(LABEL_COLORS[2]);
  });
});
