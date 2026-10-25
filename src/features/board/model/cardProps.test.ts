import { describe, expect, it } from "vitest";
import type { CardLinkRow, CardRelation } from "./board";
import {
  DATE_PICKS,
  datePickValue,
  encodeField,
  fieldChecked,
  fieldOptions,
  fieldText,
  formatEstimate,
  groupRelations,
  parseEstimate,
  relationCandidates,
  relationWrite,
  validPrUrl,
  visibleLinks,
} from "./cardProps";

describe("estimate", () => {
  it("parses hours and minutes", () => {
    for (const [text, minutes] of [
      ["90", 90],
      ["90m", 90],
      [" 1H ", 60],
      ["1h 30m", 90],
      ["1h30m", 90],
      ["1.5h", 90],
      ["2h", 120],
      ["0.5h", 30],
      ["30m 1h", 90],
    ] as const) {
      expect(parseEstimate(text), text).toEqual({ ok: true, minutes });
    }
    expect(parseEstimate("6000000")).toEqual({ ok: true, minutes: 6_000_000 });
  });

  it("rejects zero, negative, garbage and out of range", () => {
    for (const text of [
      "",
      "0",
      "0m",
      "-5",
      "-1h",
      "abc",
      "1x",
      "1h 1h",
      "1h 30",
      "h",
      "1..5h",
      "6000001",
      "0.2m",
    ]) {
      const result = parseEstimate(text);
      expect(result.ok, text).toBe(false);
      if (!result.ok) expect(result.error.length).toBeGreaterThan(0);
    }
  });

  it("displays as hours and minutes", () => {
    expect(formatEstimate(90)).toBe("1h 30m");
    expect(formatEstimate(45)).toBe("45m");
    expect(formatEstimate(120)).toBe("2h");
    expect(formatEstimate(60)).toBe("1h");
    expect(formatEstimate(1)).toBe("1m");
  });
});

describe("date picks", () => {
  it("offers Today, Tomorrow, Next week and In 2 weeks", () => {
    expect(DATE_PICKS.map(([name, days]) => [name, days])).toEqual([
      ["Today", 0],
      ["Tomorrow", 1],
      ["Next week", 7],
      ["In 2 weeks", 14],
    ]);
  });

  it("resolves to local midnight", () => {
    const now = new Date(2026, 9, 26, 15, 30).getTime();
    expect(datePickValue(14, now)).toBe(new Date(2026, 10, 9).getTime());
    expect(datePickValue(0, now)).toBe(new Date(2026, 9, 26).getTime());
  });
});

describe("relations", () => {
  it("writes blocks and blocked by in the right direction", () => {
    expect(relationWrite("blocks", "me", "other")).toEqual({
      cardId: "me",
      relatedCardId: "other",
      kind: "blocks",
    });
    expect(relationWrite("blocked-by", "me", "other")).toEqual({
      cardId: "other",
      relatedCardId: "me",
      kind: "blocks",
    });
    expect(relationWrite("relates", "me", "other")).toEqual({
      cardId: "me",
      relatedCardId: "other",
      kind: "relates",
    });
  });

  it("groups relations from this card's point of view", () => {
    const rows: CardRelation[] = [
      { cardId: "me", relatedCardId: "a", kind: "blocks" },
      { cardId: "b", relatedCardId: "me", kind: "blocks" },
      { cardId: "me", relatedCardId: "c", kind: "relates" },
      { cardId: "d", relatedCardId: "me", kind: "relates" },
      { cardId: "me", relatedCardId: "e", kind: "duplicates" },
    ];
    const groups = groupRelations("me", rows);
    expect(groups.blocks.map((r) => r.otherId)).toEqual(["a"]);
    expect(groups["blocked-by"].map((r) => r.otherId)).toEqual(["b"]);
    expect(groups.relates.map((r) => r.otherId)).toEqual(["c", "d"]);
    // The row carries what removal needs, as stored.
    expect(groups["blocked-by"][0].relation).toBe(rows[1]);
  });

  it("searches other cards by number and title, never self", () => {
    const cards = [
      { id: "me", number: 1, title: "Mine" },
      { id: "a", number: 2, title: "Fix login" },
      { id: "b", number: 12, title: "Ship it" },
    ];
    expect(relationCandidates(cards, "me", "").map((c) => c.id)).toEqual([
      "a",
      "b",
    ]);
    expect(relationCandidates(cards, "me", "#12").map((c) => c.id)).toEqual([
      "b",
    ]);
    expect(relationCandidates(cards, "me", "LOGIN").map((c) => c.id)).toEqual([
      "a",
    ]);
    expect(relationCandidates(cards, "me", "mine")).toEqual([]);
  });
});

describe("custom fields", () => {
  it("encodes by kind", () => {
    expect(encodeField("text", " hi ")).toEqual({ ok: true, value: '"hi"' });
    expect(encodeField("number", "4")).toEqual({ ok: true, value: "4" });
    expect(encodeField("number", "2.5")).toEqual({ ok: true, value: "2.5" });
    expect(encodeField("text", "  ")).toEqual({ ok: true, value: null });
    expect(encodeField("number", "")).toEqual({ ok: true, value: null });
    expect(encodeField("number", "abc").ok).toBe(false);
    expect(encodeField("number", "NaN").ok).toBe(false);
    expect(encodeField("number", "1e999").ok).toBe(false);
    expect(encodeField("select", "x").ok).toBe(false);
  });

  it("reads options and values by kind", () => {
    expect(fieldOptions('["a","b"]')).toEqual(["a", "b"]);
    expect(fieldOptions(null)).toEqual([]);
    expect(fieldOptions("nope")).toEqual([]);
    expect(fieldOptions('{"a":1}')).toEqual([]);
    expect(fieldText("text", '"hi"')).toBe("hi");
    expect(fieldText("select", '"b"')).toBe("b");
    expect(fieldText("number", "3.5")).toBe("3.5");
    expect(fieldText("checkbox", "true")).toBe("Yes");
    expect(fieldText("checkbox", "false")).toBe("No");
    expect(fieldText("date", String(new Date(2026, 10, 2).getTime()))).toBe(
      "Nov 2, 2026",
    );
    expect(fieldText("number", '"x"')).toBeNull();
    expect(fieldText("text", "garbage{")).toBeNull();
    expect(fieldChecked("true")).toBe(true);
    expect(fieldChecked("false")).toBe(false);
    expect(fieldChecked(undefined)).toBe(false);
    expect(fieldChecked("1")).toBe(false);
  });
});

describe("links", () => {
  it("accepts http and https URLs with a host", () => {
    expect(validPrUrl("https://github.com/a/b/pull/1")).toBe(true);
    expect(validPrUrl("http://x.test")).toBe(true);
    for (const bad of [
      "",
      "github.com/a/b",
      "ftp://x.test",
      "https://",
      "https:///x",
      "https://a b",
      "javascript:alert(1)",
    ]) {
      expect(validPrUrl(bad), bad).toBe(false);
    }
  });

  it("hides the primary link from the list", () => {
    const row = (kind: string, reference: string, role: string | null) =>
      ({ cardId: "c", kind, reference, role, createdAt: 1 }) as CardLinkRow;
    const rows = [
      row("session", "s1", "primary"),
      row("pr", "https://x.test", null),
      row("session", "s2", null),
    ];
    expect(visibleLinks(rows).map((r) => r.reference)).toEqual([
      "https://x.test",
      "s2",
    ]);
  });
});
