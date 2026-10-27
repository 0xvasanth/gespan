import { describe, expect, it } from "vitest";
import { applyFormat, type Format } from "./markdownToolbar";

// Ported from the native `markdown_toolbar.rs` tests. Offsets are UTF-16
// indices here (bytes there), so non-ASCII cases are re-counted.
type Sel = [number, number];

function run(format: Format, text: string, [start, end]: Sel) {
  const next = applyFormat(format, text, start, end);
  return [next.text, next.text.slice(next.start, next.end)];
}
const apply = (format: Format, text: string, [start, end]: Sel) => {
  const next = applyFormat(format, text, start, end);
  return [next.text, [next.start, next.end]];
};
const cursor = (format: Format, text: string, at: number) =>
  apply(format, text, [at, at]);

describe("bold and italic", () => {
  it("wraps a selection and selects the inside", () => {
    expect(run("bold", "make it loud", [8, 12])).toEqual([
      "make it **loud**",
      "loud",
    ]);
  });
  it("puts the cursor between the markers with no selection", () => {
    expect(cursor("bold", "ab", 1)).toEqual(["a****b", [3, 3]]);
    expect(cursor("bold", "", 0)).toEqual(["****", [2, 2]]);
  });
  it("italic wraps in underscores", () => {
    expect(run("italic", "a soft word", [2, 6])).toEqual([
      "a _soft_ word",
      "soft",
    ]);
    expect(cursor("italic", "x", 1)).toEqual(["x__", [2, 2]]);
  });
  it("keeps a trailing newline outside the markers", () => {
    expect(run("bold", "one\ntwo\n", [0, 4])).toEqual(["**one**\ntwo\n", "one"]);
  });
  it("wraps a multi-line selection whole", () => {
    expect(run("italic", "a\nb", [0, 3])).toEqual(["_a\nb_", "a\nb"]);
  });
  it("wraps at the end of the text", () => {
    expect(run("bold", "end word", [4, 8])).toEqual(["end **word**", "word"]);
    expect(cursor("bold", "end", 3)).toEqual(["end****", [5, 5]]);
  });
  it("never splits a surrogate pair", () => {
    expect(run("bold", "😀 x", [0, 2])).toEqual(["**😀** x", "😀"]);
    // An offset inside the pair clamps back to its start.
    expect(cursor("bold", "😀", 1)).toEqual(["****😀", [2, 2]]);
  });
  it("clamps an out-of-range selection", () => {
    expect(cursor("bold", "ab", 99)).toEqual(["ab****", [4, 4]]);
    expect(applyFormat("bold", "ab", 2, 0)).toMatchObject({ text: "**ab**" });
  });
});

describe("heading", () => {
  it("prefixes the cursor line", () => {
    expect(cursor("heading", "Title", 0)).toEqual(["## Title", [3, 3]]);
    expect(cursor("heading", "a\nsecond\nc", 4)).toEqual([
      "a\n## second\nc",
      [7, 7],
    ]);
    expect(cursor("heading", "", 0)).toEqual(["## ", [3, 3]]);
  });
  it("on a selection prefixes only its first line", () => {
    expect(apply("heading", "ab\ncd", [1, 5])).toEqual(["## ab\ncd", [4, 8]]);
  });
  it("on an empty first line prefixes that line", () => {
    expect(cursor("heading", "\nab", 0)).toEqual(["## \nab", [3, 3]]);
  });
  it("works at the end of the text", () => {
    expect(cursor("heading", "a\nb", 3)).toEqual(["a\n## b", [6, 6]]);
  });
});

describe("lists", () => {
  it("bullets prefix each selected line, and only those", () => {
    expect(apply("bullets", "a\nb\nc", [0, 5])).toEqual([
      "- a\n- b\n- c",
      [0, 11],
    ]);
    expect(apply("bullets", "a\nb\nc", [2, 3])).toEqual(["a\n- b\nc", [2, 5]]);
  });
  it("bullets with no selection prefix the cursor line", () => {
    expect(cursor("bullets", "a\nbc", 3)).toEqual(["a\n- bc", [5, 5]]);
    expect(cursor("bullets", "", 0)).toEqual(["- ", [2, 2]]);
  });
  it("a selection ending in a newline leaves the next line alone", () => {
    expect(apply("bullets", "a\nb\nc", [0, 4])).toEqual([
      "- a\n- b\nc",
      [0, 7],
    ]);
    expect(apply("numbered", "a\nb\n", [0, 4])).toEqual([
      "1. a\n2. b\n",
      [0, 9],
    ]);
  });
  it("numbered lists count from one", () => {
    expect(apply("numbered", "a\nb\nc", [0, 5])).toEqual([
      "1. a\n2. b\n3. c",
      [0, 14],
    ]);
    expect(cursor("numbered", "", 0)).toEqual(["1. ", [3, 3]]);
  });
  it("numbered works at the end of the text", () => {
    expect(apply("numbered", "x\nlast", [2, 6])).toEqual([
      "x\n1. last",
      [2, 9],
    ]);
  });
});

describe("link", () => {
  it("turns the selection into text and selects the url", () => {
    expect(run("link", "see docs now", [4, 8])).toEqual([
      "see [docs](url) now",
      "url",
    ]);
  });
  it("with no selection puts the cursor inside the brackets", () => {
    expect(cursor("link", "ab", 2)).toEqual(["ab[]()", [3, 3]]);
  });
  it("works at the end and with a trailing newline", () => {
    expect(run("link", "go site", [3, 7])).toEqual(["go [site](url)", "url"]);
    expect(run("link", "site\n", [0, 5])).toEqual(["[site](url)\n", "url"]);
  });
});

describe("code", () => {
  it("wraps one line in backticks", () => {
    expect(run("code", "run ls now", [4, 6])).toEqual(["run `ls` now", "ls"]);
    expect(cursor("code", "a", 1)).toEqual(["a``", [2, 2]]);
  });
  it("fences several lines", () => {
    expect(run("code", "a\nb", [0, 3])).toEqual(["```\na\nb\n```", "a\nb"]);
  });
  it("keeps a trailing newline outside the fence", () => {
    expect(run("code", "a\nb\nrest", [0, 4])).toEqual([
      "```\na\nb\n```\nrest",
      "a\nb",
    ]);
    // One line plus its newline is still one line.
    expect(run("code", "a\nrest", [0, 2])).toEqual(["`a`\nrest", "a"]);
  });
  it("fences at the end of the text", () => {
    expect(run("code", "x\na\nb", [2, 5])).toEqual([
      "x\n```\na\nb\n```",
      "a\nb",
    ]);
  });
  it("starts the fence on its own line when the selection begins mid-line", () => {
    expect(run("code", "foo bar\nbaz", [4, 11])).toEqual([
      "foo \n```\nbar\nbaz\n```",
      "bar\nbaz",
    ]);
  });
  it("closes the fence on its own line when the selection ends mid-line", () => {
    expect(run("code", "foo\nbar baz", [0, 7])).toEqual([
      "```\nfoo\nbar\n```\n baz",
      "foo\nbar",
    ]);
  });
  it("handles a selection that starts and ends mid-line", () => {
    expect(run("code", "foo bar\nbaz qux", [4, 11])).toEqual([
      "foo \n```\nbar\nbaz\n```\n qux",
      "bar\nbaz",
    ]);
  });
});

describe("non-ASCII text", () => {
  it("formats on the right indices", () => {
    expect(apply("heading", "ça\nüber", [3, 6])).toEqual([
      "ça\n## über",
      [6, 9],
    ]);
    expect(apply("bullets", "é\nü\nñ", [2, 3])).toEqual(["é\n- ü\nñ", [2, 5]]);
    expect(run("link", "voir été ici", [5, 8])).toEqual([
      "voir [été](url) ici",
      "url",
    ]);
    expect(run("code", "lancer ñandú", [7, 12])).toEqual([
      "lancer `ñandú`",
      "ñandú",
    ]);
    expect(run("code", "é ü\nñ", [2, 5])).toEqual(["é \n```\nü\nñ\n```", "ü\nñ"]);
    expect(run("bold", "é ü", [0, 1])).toEqual(["**é** ü", "é"]);
  });
});
