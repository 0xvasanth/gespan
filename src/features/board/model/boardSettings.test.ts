import { describe, expect, it } from "vitest";
import {
  DEFAULT_PRIORITY_SCHEME,
  labelNameTaken,
  offeredPriorities,
  overLimitMessage,
  parsePriorityScheme,
  parseWipLimit,
  priorityName,
  serializePriorityScheme,
  validPriorityName,
  withHidden,
  withName,
  wipCount,
} from "./boardSettings";

describe("priority scheme", () => {
  it("null, bad JSON and non-objects mean the defaults", () => {
    for (const json of [null, "", "nope", "[]", "7", "null"]) {
      expect(parsePriorityScheme(json)).toEqual(DEFAULT_PRIORITY_SCHEME);
    }
  });

  it("keeps good names and falls back per level for bad ones", () => {
    const scheme = parsePriorityScheme(
      JSON.stringify({
        names: ["Nope", "", "x".repeat(21), 4, "Critical"],
        hidden: [],
      }),
    );
    expect(scheme.names).toEqual(["Nope", "Low", "Medium", "High", "Critical"]);
  });

  it("never hides level 0 and drops junk or repeated levels", () => {
    const scheme = parsePriorityScheme(
      JSON.stringify({ hidden: [0, 2, 2, 9, "3", 1.5, 4] }),
    );
    expect(scheme.hidden).toEqual([2, 4]);
    expect(withHidden(scheme, 0, true).hidden).toEqual([2, 4]);
  });

  it("round-trips through the stored JSON", () => {
    const scheme = withHidden(
      withName(DEFAULT_PRIORITY_SCHEME, 4, " Blocker "),
      2,
      true,
    );
    expect(scheme.names[4]).toBe("Blocker");
    expect(parsePriorityScheme(serializePriorityScheme(scheme))).toEqual(
      scheme,
    );
  });

  it("names are 1 to 20 characters", () => {
    expect(validPriorityName("")).toBe(false);
    expect(validPriorityName("   ")).toBe(false);
    expect(validPriorityName("a")).toBe(true);
    expect(validPriorityName("x".repeat(20))).toBe(true);
    expect(validPriorityName("x".repeat(21))).toBe(false);
  });

  it("hidden levels leave the offered list but still have a name", () => {
    const scheme = withHidden(DEFAULT_PRIORITY_SCHEME, 1, true);
    expect(offeredPriorities(scheme)).toEqual([0, 2, 3, 4]);
    expect(priorityName(scheme, 1)).toBe("Low");
  });
});

describe("WIP count", () => {
  it("shows total/limit and flags only a total past the limit", () => {
    expect(wipCount({ total: 3, limit: 4 })).toEqual({
      text: "3/4",
      over: false,
    });
    expect(wipCount({ total: 4, limit: 4 })).toEqual({
      text: "4/4",
      over: false,
    });
    expect(wipCount({ total: 5, limit: 4 })).toEqual({
      text: "5/4",
      over: true,
    });
  });

  it("is the bare total with no limit", () => {
    expect(wipCount({ total: 7, limit: null })).toEqual({
      text: "7",
      over: false,
    });
  });

  it("filtered, reads shown/total and compares the limit to the total", () => {
    expect(wipCount({ total: 5, shown: 1, limit: 4 })).toEqual({
      text: "1/5",
      over: true,
    });
    expect(wipCount({ total: 3, shown: 1, limit: 4 })).toEqual({
      text: "1/3",
      over: false,
    });
  });

  it("words the toast", () => {
    expect(overLimitMessage(4, 3)).toBe("Over the WIP limit (4/3)");
  });

  it("parses typed limits", () => {
    expect(parseWipLimit("")).toBeNull();
    expect(parseWipLimit(" 5 ")).toBe(5);
    for (const bad of ["0", "-1", "2.5", "abc"]) {
      expect(parseWipLimit(bad)).toBe("invalid");
    }
  });
});

describe("label names", () => {
  const labels = [
    { id: "a", name: "Bug" },
    { id: "b", name: "Docs" },
  ];
  it("are unique ignoring case and spacing, except for the label itself", () => {
    expect(labelNameTaken(labels, " bug ")).toBe(true);
    expect(labelNameTaken(labels, "BUG", "a")).toBe(false);
    expect(labelNameTaken(labels, "bug", "b")).toBe(true);
    expect(labelNameTaken(labels, "Ops")).toBe(false);
  });
});
