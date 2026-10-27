import { describe, expect, it } from "vitest";
import type { BoardField } from "./board";
import {
  DEFAULT_DISPLAY,
  laneBy,
  layoutDisplay,
  parseDisplay,
  resolveDisplay,
  serializeDisplay,
  withLayoutDisplay,
} from "./display";

const field = (id: string, kind: BoardField["kind"]): BoardField => ({
  id,
  boardId: "b",
  name: id,
  kind,
  optionsJson: null,
  position: 0,
});

describe("display options", () => {
  it("null, bad JSON and non-objects mean the defaults: every toggle on, comfortable, no lanes", () => {
    for (const json of [null, "", "nope", "[]", "3"]) {
      expect(parseDisplay(json)).toEqual(DEFAULT_DISPLAY);
    }
    expect(DEFAULT_DISPLAY.face.fields).toEqual([]);
    expect(DEFAULT_DISPLAY.face.agentFields).toEqual([]);
    expect(Object.values(DEFAULT_DISPLAY.face).filter((v) => v === true)).toHaveLength(7);
  });

  it("keeps valid parts and drops bad ones one by one", () => {
    const display = parseDisplay(
      JSON.stringify({
        face: {
          number: false,
          labels: "no",
          fields: ["a", "a", 3],
          agentFields: ["cost", "bogus", "cost"],
        },
        density: "tiny",
        swimlane: "field:x",
      }),
    );
    expect(display.face.number).toBe(false);
    expect(display.face.labels).toBe(true);
    expect(display.face.fields).toEqual(["a"]);
    expect(display.face.agentFields).toEqual(["cost"]);
    expect(display.density).toBe("comfortable");
    expect(display.swimlane).toBe("field:x");
    expect(parseDisplay(JSON.stringify({ swimlane: "weird" })).swimlane).toBe("none");
  });

  it("round-trips", () => {
    const next = { ...DEFAULT_DISPLAY, density: "compact" as const, swimlane: "label" };
    expect(parseDisplay(serializeDisplay(next))).toEqual(next);
  });

  it("a view's override sits over the board default; All cards ignores it", () => {
    const board = JSON.stringify({ face: { due: false }, density: "compact" });
    const layout = JSON.stringify({
      layout: "board",
      display: { face: { number: false }, swimlane: "priority" },
    });
    const view = resolveDisplay(board, layout);
    expect(view.face.due).toBe(false);
    expect(view.face.number).toBe(false);
    expect(view.density).toBe("compact");
    expect(view.swimlane).toBe("priority");
    const all = resolveDisplay(board, null);
    expect(all.face.number).toBe(true);
    expect(all.swimlane).toBe("none");
    // A layout with no override changes nothing.
    expect(resolveDisplay(board, '{"layout":"board"}')).toEqual(all);
  });

  it("writes and clears an override without touching other layout keys", () => {
    const layout = '{"layout":"table","columns":["title"],"future":1}';
    const withIt = withLayoutDisplay(layout, DEFAULT_DISPLAY);
    expect(JSON.parse(withIt)).toMatchObject({ layout: "table", future: 1 });
    expect(layoutDisplay(withIt)).toEqual(DEFAULT_DISPLAY);
    const cleared = JSON.parse(withLayoutDisplay(withIt, null));
    expect(cleared.display).toBeUndefined();
    expect(cleared.future).toBe(1);
  });

  it("lanes ignore a missing field and non-select fields", () => {
    const fields = [field("s", "select"), field("t", "text")];
    const by = (swimlane: string) => laneBy({ ...DEFAULT_DISPLAY, swimlane }, fields);
    expect(by("none")).toBeNull();
    expect(by("priority")).toEqual({ kind: "priority" });
    expect(by("field:s")).toMatchObject({ kind: "field" });
    expect(by("field:t")).toBeNull();
    expect(by("field:gone")).toBeNull();
  });
});
