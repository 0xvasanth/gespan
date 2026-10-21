import { describe, expect, it } from "vitest";
import { turnsBefore, watchUpdate, watchWindow } from "./mobileWatch";
import type { MobileTurn } from "./mobileTranscript";
import {
  applyFrames,
  type WatchFrame,
} from "../../../../mobile/src/features/workspace/watchUpdates";

const turn = (id: string, text: string): MobileTurn => ({
  id,
  items: [{ type: "text", id: `${id}:a`, text }],
});

describe("mobile watch frames", () => {
  it("sends only the appended suffix of a long reply, including Unicode offsets", () => {
    const before = turn("a", "👋 stable text ".repeat(10_000));
    const first = watchUpdate(
      null,
      { busy: true, older: false, turns: [before] },
      true,
    )!;
    const after = turn(
      "a",
      before.items[0].type === "text" ? before.items[0].text + "hello" : "",
    );
    const next = watchUpdate(
      first.baseline,
      { busy: true, older: false, turns: [after] },
      true,
    )!;
    expect(next.update).toMatchObject({
      turns: [],
      patches: [
        {
          id: "a",
          edits: [
            { path: ["items", 0, "text"], from: 150_000, append: "hello" },
          ],
        },
      ],
    });
    expect(JSON.stringify(next.update).length).toBeLessThan(250);
    expect(applyFrames([before], [next.update as WatchFrame])?.turns).toEqual([
      after,
    ]);
  });

  it("patches nested tool state, additions, removals and completion without resending stable replies", () => {
    const stable = {
      type: "text" as const,
      id: "reply",
      text: "stable ".repeat(1000),
    };
    const before: MobileTurn = {
      id: "a",
      durationMs: 100,
      items: [
        stable,
        {
          type: "header",
          id: "h",
          live: true,
          startedAt: 1,
          entries: [{ type: "prose", id: "p", text: "Inspecting" }],
        },
      ],
    };
    const first = watchUpdate(
      null,
      { busy: true, older: false, turns: [before] },
      true,
    )!;
    const updates: MobileTurn[] = [
      {
        id: "a",
        items: [
          stable,
          {
            type: "header",
            id: "h",
            live: false,
            entries: [
              { type: "prose", id: "p", text: "Inspecting files" },
              {
                type: "phase",
                id: "phase",
                kind: "think",
                title: "Read",
                steps: [],
              },
            ],
          },
        ],
      },
      { id: "a", footer: true, items: [stable] },
    ];
    let baseline = first.baseline;
    let known = [before];
    for (const updated of updates) {
      const next = watchUpdate(
        baseline,
        { busy: false, older: false, turns: [updated] },
        true,
      )!;
      expect(next.update).toHaveProperty("patches");
      expect(JSON.stringify(next.update)).not.toContain(stable.text);
      known = applyFrames(known, [next.update as WatchFrame])!
        .turns as MobileTurn[];
      expect(known).toEqual([updated]);
      expect(known[0].items[0]).toBe(stable);
      baseline = next.baseline;
    }
    expect(before.items).toHaveLength(2);
    expect(before.durationMs).toBe(100);
  });

  it("keeps snapshots for new turns, small replacements, and phones without incremental support", () => {
    const previous = turn("a", "before");
    const first = watchUpdate(
      null,
      { busy: true, older: false, turns: [previous] },
      true,
    )!;
    const turns = [turn("a", "replacement"), turn("b", "new")];
    for (const incremental of [false, true]) {
      const next = watchUpdate(
        first.baseline,
        { busy: true, older: false, turns },
        incremental,
      )!;
      expect(next.update).toEqual({
        busy: true,
        older: false,
        order: ["a", "b"],
        turns,
      });
    }
  });

  it("sends everything first, then only the turn that changed", () => {
    const first = watchUpdate(null, {
      busy: true,
      older: false,
      turns: [turn("a", "done"), turn("b", "Wor")],
    })!;
    expect(first.update).toMatchObject({
      order: ["a", "b"],
      turns: { length: 2 },
    });

    const next = watchUpdate(first.baseline, {
      busy: true,
      older: false,
      turns: [turn("a", "done"), turn("b", "Working")],
    })!;
    expect(next.update).toEqual({
      busy: true,
      older: false,
      order: ["a", "b"],
      turns: [turn("b", "Working")],
    });

    expect(
      watchUpdate(next.baseline, {
        busy: true,
        older: false,
        turns: [turn("a", "done"), turn("b", "Working")],
      }),
    ).toBeNull();
    expect(
      watchUpdate(next.baseline, {
        busy: false,
        older: false,
        turns: [turn("a", "done"), turn("b", "Working")],
      })?.update,
    ).toEqual({ busy: false, older: false, order: ["a", "b"], turns: [] });
  });

  it("opens on the latest turns and grows from there", () => {
    const ids = ["a", "b", "c", "d", "e"];
    const opened = watchWindow(ids, null, 2);
    expect(opened).toEqual({ start: 3, from: "d", older: true });
    expect(watchWindow([...ids, "f"], opened.from, 2).start).toBe(3);
    expect(watchWindow(ids, "gone", 10)).toEqual({
      start: 0,
      from: "a",
      older: false,
    });
  });

  it("pages in the turns before the window", () => {
    const ids = ["a", "b", "c", "d", "e"];
    expect(turnsBefore(ids, "d", 2)).toEqual({ start: 1, end: 3, older: true });
    expect(turnsBefore(ids, "b", 5)).toEqual({
      start: 0,
      end: 1,
      older: false,
    });
    expect(() => turnsBefore(ids, "gone", 2)).toThrow();
  });
});
