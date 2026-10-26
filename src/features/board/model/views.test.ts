import { describe, expect, it } from "vitest";
import type { BoardField } from "./board";
import {
  EMPTY_SPEC,
  chipText,
  currentSpec,
  fieldChoices,
  fieldTitle,
  liveRows,
  parseSpec,
  runningCardIds,
  sameJson,
  sameSpec,
  type FilterSources,
} from "./views";

const field = (
  id: string,
  kind: BoardField["kind"],
  optionsJson: string | null = null,
): BoardField => ({
  id,
  boardId: "b1",
  name: `Field ${id}`,
  kind,
  optionsJson,
  position: 0,
});

const sources: FilterSources = {
  columns: [
    { id: "done", name: "Done", position: 1 },
    { id: "todo", name: "Todo", position: 0 },
  ],
  labels: [{ id: "l1", name: "Bug" }],
  projects: [{ path: "/work/api", name: "api" }],
  fields: [
    field("sel", "select", '["Backend","Web"]'),
    field("box", "checkbox"),
    field("txt", "text"),
    field("day", "date"),
  ],
};

describe("fieldChoices", () => {
  it("lists columns by position and priorities from Urgent down", () => {
    expect(fieldChoices("status", sources).map(([, name]) => name)).toEqual([
      "Todo",
      "Done",
    ]);
    expect(fieldChoices("priority", sources)[0]).toEqual(["4", "Urgent"]);
    expect(fieldChoices("priority", sources).at(-1)).toEqual(["0", "None"]);
  });

  it("encodes select options as JSON strings and checkboxes as true/false", () => {
    expect(fieldChoices("field:sel", sources)).toEqual([
      ['"Backend"', "Backend"],
      ['"Web"', "Web"],
    ]);
    expect(fieldChoices("field:box", sources)).toEqual([
      ["true", "Yes"],
      ["false", "No"],
    ]);
  });

  it("offers free-form fields only what the row holds, and a deleted field nothing", () => {
    expect(fieldChoices("field:txt", sources)).toEqual([]);
    expect(fieldChoices("field:txt", sources, ['"alpha"', "12"])).toEqual([
      ['"alpha"', "alpha"],
      ["12", "12"],
    ]);
    expect(fieldChoices("field:gone", sources, ['"x"'])).toEqual([]);
  });

  it("survives a select whose options are not valid JSON", () => {
    const broken = { ...sources, fields: [field("sel", "select", "{nope")] };
    expect(fieldChoices("field:sel", broken)).toEqual([]);
  });
});

describe("chipText and fieldTitle", () => {
  it("says the field, the op and the names", () => {
    expect(
      chipText(
        { field: "status", op: "is_not", values: ["todo", "done"] },
        sources,
      ),
    ).toBe("Status is not Todo, Done");
  });

  it("leaves out values that are gone and shows … when none are left", () => {
    expect(
      chipText({ field: "labels", op: "is", values: ["l1", "gone"] }, sources),
    ).toBe("Labels is Bug");
    expect(
      chipText({ field: "labels", op: "is", values: ["gone"] }, sources),
    ).toBe("Labels is …");
  });

  it("names a custom field and flags a deleted one", () => {
    expect(fieldTitle("field:sel", sources)).toBe("Field sel");
    expect(
      chipText({ field: "field:gone", op: "is", values: ['"x"'] }, sources),
    ).toBe("Deleted field is …");
  });
});

describe("liveRows", () => {
  it("drops missing values and rows left empty, keeping order", () => {
    const rows = [
      { field: "labels", op: "is" as const, values: ["gone"] },
      { field: "status", op: "is" as const, values: ["todo", "gone"] },
      { field: "priority", op: "is_not" as const, values: ["3"] },
    ];
    expect(liveRows(rows, sources)).toEqual([
      { field: "status", op: "is", values: ["todo"] },
      { field: "priority", op: "is_not", values: ["3"] },
    ]);
  });

  it("drops a row on a deleted field and a deleted select option", () => {
    expect(
      liveRows(
        [
          { field: "field:gone", op: "is", values: ['"x"'] },
          { field: "field:sel", op: "is", values: ['"Mobile"'] },
        ],
        sources,
      ),
    ).toEqual([]);
  });

  it("keeps free-form values for the engine to judge", () => {
    const rows = [{ field: "field:txt", op: "is" as const, values: ['"a"'] }];
    expect(liveRows(rows, sources)).toEqual(rows);
  });
});

describe("parseSpec and sameSpec", () => {
  it("falls back to an empty spec for text that does not parse", () => {
    expect(parseSpec("not json")).toEqual(EMPTY_SPEC);
    expect(parseSpec("null")).toEqual(EMPTY_SPEC);
    expect(parseSpec("{}")).toEqual(EMPTY_SPEC);
  });

  it("drops rows without values and unusable rows, and fixes a bad sort", () => {
    const spec = parseSpec(
      JSON.stringify({
        query: "  hi ",
        rows: [
          { field: "labels", op: "is", values: [] },
          { op: "is", values: ["x"] },
          { field: "status", op: "weird", values: ["todo", 3] },
        ],
        sort: { key: "nonsense", dir: "sideways" },
      }),
    );
    expect(spec).toEqual({
      query: "hi",
      rows: [{ field: "status", op: "is", values: ["todo"] }],
      sort: { key: "manual", dir: "asc" },
    });
  });

  it("compares a saved spec with the screen regardless of key order", () => {
    const saved = parseSpec(
      '{"sort":{"dir":"desc","key":"due"},"rows":[{"values":["3"],"op":"is","field":"priority"}],"query":"x"}',
    );
    const screen = currentSpec({
      query: " x ",
      rows: [
        { field: "priority", op: "is", values: ["3"] },
        { field: "labels", op: "is", values: [] },
      ],
      sort: { key: "due", dir: "desc" },
    });
    expect(sameSpec(saved, screen)).toBe(true);
    expect(sameSpec(saved, { ...screen, query: "y" })).toBe(false);
    expect(
      sameSpec(saved, { ...screen, sort: { key: "due", dir: "asc" } }),
    ).toBe(false);
  });
});

describe("sameJson", () => {
  it("ignores key order and spacing, not content", () => {
    expect(sameJson('{"a":1,"b":{"c":2}}', '{ "b": {"c": 2}, "a": 1 }')).toBe(
      true,
    );
    expect(sameJson('{"a":[1,2]}', '{"a":[2,1]}')).toBe(false);
    expect(sameJson("nope", "nope")).toBe(true);
    expect(sameJson("nope", "{}")).toBe(false);
  });
});

describe("runningCardIds", () => {
  it("returns the sorted ids of cards whose link is running", () => {
    const cards = [
      { id: "c2", linkKind: "session" as const, linkId: "s2" },
      { id: "c1", linkKind: "pilot" as const, linkId: "p1" },
      { id: "c3", linkKind: null, linkId: null },
      { id: "c4", linkKind: "session" as const, linkId: "s4" },
    ];
    expect(
      runningCardIds(cards, (_, id) => id === "s2" || id === "p1"),
    ).toEqual(["c1", "c2"]);
  });
});
